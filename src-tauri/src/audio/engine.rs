//! Real-time audio engine.
//!
//! Design in one paragraph: the topology (which devices, which lines, which
//! routes) is immutable once built — any structural change tears down every
//! stream and rebuilds. This keeps the audio callbacks 100% free of
//! allocation-on-topology and locking. Continuous parameters (faders, mutes,
//! ducking amounts) are read through atomics on every block, so they are
//! live without a rebuild and without a glitch.
//!
//! Signal path — everything meets in a canonical 48 kHz interleaved-stereo
//! domain:
//!
//! ```text
//! capture device ─(downmix to stereo)─(resample → 48k)─▶ ring buffer per route ─┐
//!                                                                               ▼
//! render device ◀─(soft clip)─(resample → device rate)─(Σ line·route·duck gains)┘
//! ```
//!
//! `cpal::Stream` is `!Send`, so all streams are owned by one dedicated
//! engine thread; the rest of the app talks to it through an mpsc channel
//! (topology) and through [`Controls`] atomics (live parameters).

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, Stream, StreamConfig};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use serde::Serialize;
use tauri::Emitter;

use super::controls::{Controls, LineCtl, OutputCtl, RouteCtl};
use super::dsp::{soft_clip, Resampler, StereoBiquad, EQ_Q};
use super::model::{AppConfig, EqBandCfg};

/// Canonical internal sample rate. Every line is mixed in this domain.
const SR: u32 = 48_000;
/// Ring buffer capacity per route, in f32 samples (stereo interleaved).
/// 2^17 samples ≈ 0.68 s @ 48 kHz — headroom for clock drift.
const RING_CAP: usize = 1 << 17;
/// Frames of silence pre-filled in each ring. This is the base latency
/// between capture and render: 30 ms — low enough for A/V sync, and the
/// drift servo in the render callback keeps the fill pinned there.
const PREFILL_FRAMES: usize = 1_440;
/// One-pole gain smoothing coefficient per 48 kHz sample (~10 ms time
/// constant) — removes zipper noise when faders move.
const SMOOTH: f32 = 0.002;

/// Messages accepted by the engine thread.
pub enum EngineMsg {
    /// Tear down all streams and rebuild from this config/controls pair.
    Rebuild(AppConfig, Arc<Controls>),
    /// Graceful stop (currently the OS teardown does the job; kept for a
    /// future clean-exit path).
    #[allow(dead_code)]
    Shutdown,
}

/// Status event emitted to the frontend after each (re)build.
#[derive(Clone, Serialize)]
pub struct EngineStatus {
    /// Human-readable problems (device missing, stream failed...).
    pub warnings: Vec<String>,
    /// Number of capture streams actually running.
    pub active_captures: usize,
    /// Number of render streams actually running.
    pub active_renders: usize,
}

/// Spawn the engine thread. It owns every `cpal::Stream` (which are `!Send`)
/// and lives for the whole app lifetime.
pub fn spawn(app: tauri::AppHandle, rx: Receiver<EngineMsg>) {
    std::thread::Builder::new()
        .name("mixflow-audio-engine".into())
        .spawn(move || {
            // Dropping a cpal Stream stops it — clearing this Vec is the
            // whole teardown story.
            let mut streams: Vec<Stream> = Vec::new();
            while let Ok(msg) = rx.recv() {
                match msg {
                    EngineMsg::Rebuild(cfg, controls) => {
                        streams.clear();
                        let (built, status) = build(&cfg, &controls);
                        streams = built;
                        let _ = app.emit("engine_status", &status);
                    }
                    EngineMsg::Shutdown => break,
                }
            }
        })
        .expect("failed to spawn audio engine thread");
}

// ---------------------------------------------------------------------------
// Build: config -> live streams
// ---------------------------------------------------------------------------

/// A line feeding one output bus, as seen by that bus's render callback.
struct RenderInput {
    cons: HeapCons<f32>,
    line_id: String,
    line: Arc<LineCtl>,
    route: Arc<RouteCtl>,
    /// Smoothed effective gain (line × route × duck), per-sample one-pole.
    gain_sm: f32,
}

fn build(cfg: &AppConfig, controls: &Arc<Controls>) -> (Vec<Stream>, EngineStatus) {
    let host = cpal::default_host();
    let mut warnings = Vec::new();
    let mut streams = Vec::new();

    // 1. Create one SPSC ring buffer per active route (line with a capture
    //    device -> output with a render device).
    let mut line_prods: HashMap<String, Vec<HeapProd<f32>>> = HashMap::new();
    let mut out_inputs: HashMap<String, Vec<RenderInput>> = HashMap::new();

    for line in &cfg.lines {
        if line.input_device.is_none() {
            continue; // dormant line
        }
        for route in &line.routes {
            let Some(out_cfg) = cfg.outputs.iter().find(|o| o.id == route.output_id) else {
                continue; // stale route (output deleted)
            };
            if out_cfg.device.is_empty() {
                continue; // unassigned output bus
            }
            let rb = HeapRb::<f32>::new(RING_CAP);
            let (mut prod, cons) = rb.split();
            // Pre-fill silence: fixes the base latency and absorbs the
            // startup jitter between the two device clocks.
            for _ in 0..PREFILL_FRAMES * 2 {
                let _ = prod.try_push(0.0);
            }
            let line_ctl = controls.lines[&line.id].clone();
            let route_ctl = line_ctl.routes[&route.output_id].clone();
            line_prods.entry(line.id.clone()).or_default().push(prod);
            out_inputs
                .entry(route.output_id.clone())
                .or_default()
                .push(RenderInput {
                    cons,
                    line_id: line.id.clone(),
                    line: line_ctl,
                    route: route_ctl,
                    gain_sm: 0.0,
                });
        }
    }

    // 2. One capture stream per line that actually feeds something.
    let mut active_captures = 0;
    for line in &cfg.lines {
        let Some(dev_name) = &line.input_device else {
            continue;
        };
        let Some(prods) = line_prods.remove(&line.id) else {
            continue;
        };
        match build_capture(&host, dev_name, prods, controls.lines[&line.id].clone()) {
            Ok(s) => {
                streams.push(s);
                active_captures += 1;
            }
            Err(e) => warnings.push(format!("Line \"{}\": {e}", line.name)),
        }
    }

    // 3. One render stream per output bus that receives at least one line.
    let mut active_renders = 0;
    for out in &cfg.outputs {
        let Some(inputs) = out_inputs.remove(&out.id) else {
            continue;
        };
        match build_render(
            &host,
            &out.device,
            inputs,
            controls.outputs[&out.id].clone(),
            controls.clone(),
        ) {
            Ok(s) => {
                streams.push(s);
                active_renders += 1;
            }
            Err(e) => warnings.push(format!("Output \"{}\": {e}", out.name)),
        }
    }

    let status = EngineStatus {
        warnings,
        active_captures,
        active_renders,
    };
    (streams, status)
}

/// Plain fn (not a closure) so it can be handed to several generic
/// `build_*_stream` instantiations.
fn stream_err(e: cpal::StreamError) {
    eprintln!("[mixflow] stream error: {e}");
}

// ---------------------------------------------------------------------------
// Capture side
// ---------------------------------------------------------------------------

/// Per-capture-stream state, moved into the audio callback.
struct CaptureState {
    channels: usize,
    resampler: Resampler,
    prods: Vec<HeapProd<f32>>,
    ctl: Arc<LineCtl>,
    /// One peaking biquad per parametric band, applied in the 48 kHz domain.
    eq_filters: Vec<StereoBiquad>,
    /// Band settings the coefficients were computed from.
    eq_cached: Vec<EqBandCfg>,
    /// Device block downmixed to interleaved stereo (device rate).
    stereo: Vec<f32>,
    /// Same block resampled to 48 kHz.
    res: Vec<f32>,
}

impl CaptureState {
    /// `data` is one device block as interleaved f32, `self.channels` wide.
    fn process(&mut self, data: &[f32]) {
        let frames = data.len() / self.channels;
        if frames == 0 {
            return;
        }
        // Downmix to stereo: mono is duplicated, extra channels are dropped.
        self.stereo.clear();
        for f in 0..frames {
            let base = f * self.channels;
            let l = data[base];
            let r = if self.channels > 1 { data[base + 1] } else { l };
            self.stereo.push(l);
            self.stereo.push(r);
        }
        // Into the canonical 48 kHz domain.
        self.resampler.process_all(&self.stereo, &mut self.res);

        // Parametric EQ. `try_read` keeps the callback lock-free (the UI
        // writing a point move just delays the update by one ~10 ms block);
        // coefficients are recomputed only for bands that actually moved.
        if let Some(bands) = self.ctl.eq.try_read() {
            if bands.len() != self.eq_cached.len() {
                // A point was added/removed: reconcile by frequency instead
                // of rebuilding every filter from scratch. Rebuilding blindly
                // would reset the biquad history (self.s) of EVERY band, not
                // just the one that changed, causing an audible click on all
                // the OTHER, untouched bands too.
                let mut old_cached = std::mem::take(&mut self.eq_cached);
                let mut old_filters = std::mem::take(&mut self.eq_filters);
                let mut new_filters = Vec::with_capacity(bands.len());
                for b in bands.iter() {
                    if let Some(pos) = old_cached
                        .iter()
                        .position(|c| (c.freq - b.freq).abs() < 0.5)
                    {
                        old_cached.remove(pos);
                        let mut filt = old_filters.remove(pos);
                        filt.set_peaking(SR as f32, b.freq, EQ_Q, b.gain);
                        new_filters.push(filt);
                    } else {
                        new_filters.push(StereoBiquad::peaking(SR as f32, b.freq, EQ_Q, b.gain));
                    }
                }
                self.eq_filters = new_filters;
                self.eq_cached = bands.clone();
            } else {
                for (i, b) in bands.iter().enumerate() {
                    let c = &self.eq_cached[i];
                    if (b.gain - c.gain).abs() > 0.01 || (b.freq - c.freq).abs() > 0.5 {
                        self.eq_cached[i] = *b;
                        self.eq_filters[i].set_peaking(SR as f32, b.freq, EQ_Q, b.gain);
                    }
                }
            }
        }
        for filt in &mut self.eq_filters {
            filt.process_interleaved(&mut self.res);
        }

        // VU peak (read-and-decayed by the levels emitter) + ducking
        // envelope follower (decaying peak).
        let mut peak = 0.0f32;
        for &s in &self.res {
            peak = peak.max(s.abs());
        }
        if peak > self.ctl.peak.get() {
            self.ctl.peak.set(peak);
        }
        let decay = self.ctl.duck_decay.get();
        self.ctl.env.set((self.ctl.env.get() * decay).max(peak));

        // Fan out to every routed output. If a ring is full (render device
        // stalled, or its clock runs slower), the excess is dropped — the
        // ring never blocks.
        for p in &mut self.prods {
            let _ = p.push_slice(&self.res);
        }
    }
}

fn build_capture(
    host: &cpal::Host,
    device_name: &str,
    prods: Vec<HeapProd<f32>>,
    ctl: Arc<LineCtl>,
) -> Result<Stream, String> {
    // Regular capture endpoint (mic, cable "Output" side)…
    let found = host
        .input_devices()
        .map_err(|e| e.to_string())?
        .find(|d| d.name().map(|n| n == device_name).unwrap_or(false));
    // …or WASAPI loopback: opening an *input* stream on a *render* device
    // captures everything currently playing on it ("capturer un haut-parleur").
    let (device, supported) = match found {
        Some(d) => {
            let cfg = d.default_input_config().map_err(|e| e.to_string())?;
            (d, cfg)
        }
        None => {
            let d = host
                .output_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().map(|n| n == device_name).unwrap_or(false))
                .ok_or_else(|| format!("périphérique \"{device_name}\" introuvable"))?;
            let cfg = d.default_output_config().map_err(|e| e.to_string())?;
            (d, cfg)
        }
    };
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();

    let eq_cached: Vec<EqBandCfg> = ctl.eq.read().clone();
    let state = CaptureState {
        channels: config.channels as usize,
        resampler: Resampler::new(config.sample_rate.0, SR),
        prods,
        eq_filters: eq_cached
            .iter()
            .map(|b| StereoBiquad::peaking(SR as f32, b.freq, EQ_Q, b.gain))
            .collect(),
        eq_cached,
        ctl,
        stereo: Vec::with_capacity(8192),
        res: Vec::with_capacity(8192),
    };

    let stream = match sample_format {
        SampleFormat::F32 => capture_typed::<f32>(&device, &config, state),
        SampleFormat::I16 => capture_typed::<i16>(&device, &config, state),
        SampleFormat::U16 => capture_typed::<u16>(&device, &config, state),
        SampleFormat::I32 => capture_typed::<i32>(&device, &config, state),
        other => return Err(format!("unsupported sample format {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

/// Monomorphized capture stream builder: converts the device's native sample
/// type to f32 then feeds [`CaptureState::process`].
fn capture_typed<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut state: CaptureState,
) -> Result<Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    // Conversion scratch, reused across callbacks (grows once, then stable).
    let mut as_f32: Vec<f32> = Vec::with_capacity(8192);
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            as_f32.clear();
            as_f32.extend(data.iter().map(|&s| f32::from_sample(s)));
            state.process(&as_f32);
        },
        stream_err,
        None,
    )
}

// ---------------------------------------------------------------------------
// Render side
// ---------------------------------------------------------------------------

/// Per-render-stream state, moved into the audio callback.
struct RenderState {
    channels: usize,
    resampler: Resampler,
    inputs: Vec<RenderInput>,
    ctl: Arc<OutputCtl>,
    /// Whole control plane — needed to resolve ducking side-chains.
    controls: Arc<Controls>,
    /// 48 kHz stereo mix bus.
    mix: Vec<f32>,
    /// Per-input pop scratch.
    tmp: Vec<f32>,
    /// Device-rate stereo, post output-fader and soft clip.
    dev: Vec<f32>,
    /// Smoothed output fader.
    gain_sm: f32,
    /// Smoothed ring-fill (frames) for the drift servo.
    fill_avg: f32,
}

impl RenderState {
    /// Fill `self.dev` with `frames` stereo frames ready for the device.
    fn render(&mut self, frames: usize) {
        // Drift servo: capture and render devices run on different clocks;
        // without correction the rings slowly drain (crackles) or grow
        // (latency). Nudge the consumption rate ±0.3% — inaudible — to keep
        // the emptiest ring pinned at the target prefill.
        if !self.inputs.is_empty() {
            let fill = self
                .inputs
                .iter()
                .map(|i| i.cons.occupied_len() / 2)
                .min()
                .unwrap_or(PREFILL_FRAMES) as f32;
            self.fill_avg += (fill - self.fill_avg) * 0.02;
            let error = (self.fill_avg - PREFILL_FRAMES as f32) / PREFILL_FRAMES as f32;
            self.resampler
                .set_trim((error as f64 * 0.003).clamp(-0.003, 0.003));
        }

        let need = self.resampler.required_input(frames);
        self.mix.clear();
        self.mix.resize(need * 2, 0.0);

        for input in &mut self.inputs {
            // Resolve the ducking factor for this line on this block.
            // `try_read` keeps the callback lock-free: if the UI happens to
            // be writing the rules right now, we keep last block's gain for
            // ~10 ms — inaudible.
            let mut duck = 1.0f32;
            if let Some(rules) = self.controls.ducking.try_read() {
                for rule in rules.iter().filter(|r| r.target_line == input.line_id) {
                    if let Some(src) = self.controls.lines.get(&rule.source_line) {
                        // Gate opens as the side-chain envelope crosses
                        // roughly -40 dBFS, fully open near -26 dBFS.
                        let gate = ((src.env.get() - 0.01) / 0.04).clamp(0.0, 1.0);
                        duck *= 1.0 - rule.amount.clamp(0.0, 1.0) * gate;
                    }
                }
            }
            let target = if input.line.muted.load(Ordering::Relaxed) {
                0.0
            } else {
                input.line.gain.get() * input.route.gain.get() * duck
            };

            // Pull this line's 48 kHz audio; a short ring (capture underrun
            // or clock drift) leaves the tail silent instead of blocking.
            self.tmp.clear();
            self.tmp.resize(need * 2, 0.0);
            let _got = input.cons.pop_slice(&mut self.tmp);

            // Accumulate with per-sample smoothed gain (no zipper noise).
            let mut g = input.gain_sm;
            for f in 0..need {
                g += (target - g) * SMOOTH;
                self.mix[f * 2] += self.tmp[f * 2] * g;
                self.mix[f * 2 + 1] += self.tmp[f * 2 + 1] * g;
            }
            input.gain_sm = g;
        }

        // 48 kHz -> device rate.
        self.resampler
            .process_exact(&self.mix, frames, &mut self.dev);

        // Output fader × global MASTER, soft clip, VU.
        let target = if self.ctl.muted.load(Ordering::Relaxed) {
            0.0
        } else {
            self.ctl.gain.get() * self.controls.master.get()
        };
        let mut g = self.gain_sm;
        let mut peak = 0.0f32;
        for f in 0..frames {
            g += (target - g) * SMOOTH;
            for c in 0..2 {
                let s = soft_clip(self.dev[f * 2 + c] * g);
                self.dev[f * 2 + c] = s;
                peak = peak.max(s.abs());
            }
        }
        self.gain_sm = g;
        if peak > self.ctl.peak.get() {
            self.ctl.peak.set(peak);
        }
    }

    /// Map the stereo `self.dev` buffer onto the device's channel layout:
    /// mono devices get an L+R downmix, >2-channel devices get stereo on the
    /// first pair and silence elsewhere.
    fn write_out<T>(&self, data: &mut [T])
    where
        T: cpal::Sample + cpal::FromSample<f32>,
    {
        let frames = data.len() / self.channels;
        for f in 0..frames {
            let l = self.dev[f * 2];
            let r = self.dev[f * 2 + 1];
            if self.channels == 1 {
                data[f] = T::from_sample((l + r) * 0.5);
            } else {
                data[f * self.channels] = T::from_sample(l);
                data[f * self.channels + 1] = T::from_sample(r);
                for c in 2..self.channels {
                    data[f * self.channels + c] = T::from_sample(0.0f32);
                }
            }
        }
    }
}

fn build_render(
    host: &cpal::Host,
    device_name: &str,
    inputs: Vec<RenderInput>,
    ctl: Arc<OutputCtl>,
    controls: Arc<Controls>,
) -> Result<Stream, String> {
    let device = host
        .output_devices()
        .map_err(|e| e.to_string())?
        .find(|d| d.name().map(|n| n == device_name).unwrap_or(false))
        .ok_or_else(|| format!("output device \"{device_name}\" not found"))?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();

    let state = RenderState {
        channels: config.channels as usize,
        resampler: Resampler::new(SR, config.sample_rate.0),
        inputs,
        ctl,
        controls,
        mix: Vec::with_capacity(8192),
        tmp: Vec::with_capacity(8192),
        dev: Vec::with_capacity(8192),
        gain_sm: 0.0,
        fill_avg: PREFILL_FRAMES as f32,
    };

    let stream = match sample_format {
        SampleFormat::F32 => render_typed::<f32>(&device, &config, state),
        SampleFormat::I16 => render_typed::<i16>(&device, &config, state),
        SampleFormat::U16 => render_typed::<u16>(&device, &config, state),
        SampleFormat::I32 => render_typed::<i32>(&device, &config, state),
        other => return Err(format!("unsupported sample format {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

/// Monomorphized render stream builder: mixes in f32 then converts to the
/// device's native sample type.
fn render_typed<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut state: RenderState,
) -> Result<Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let frames = data.len() / state.channels;
            state.render(frames);
            state.write_out(data);
        },
        stream_err,
        None,
    )
}

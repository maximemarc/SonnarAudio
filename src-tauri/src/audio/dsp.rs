//! Small DSP helpers: lock-free atomic f32, linear resampler, soft clipper.

use std::sync::atomic::{AtomicU32, Ordering};

/// f32 stored as raw bits in an AtomicU32 — safe to share between the UI
/// thread (writes gains) and real-time audio callbacks (read gains, write
/// levels) without locking.
#[derive(Debug)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }
    #[inline]
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    #[inline]
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
}

/// Center frequencies (Hz) of the per-line 5-band graphic EQ.
pub const EQ_FREQS: [f32; 5] = [80.0, 250.0, 1_000.0, 4_000.0, 12_000.0];
/// Q factor shared by all EQ bands (gentle, musical overlap).
pub const EQ_Q: f32 = 1.0;

/// Peaking biquad (RBJ audio-EQ cookbook) processing interleaved stereo.
///
/// Coefficients can be swapped live via [`set_peaking`](Self::set_peaking)
/// without resetting the filter state, so moving an EQ slider never clicks.
/// At |gain| < 0.05 dB the band is bypassed entirely (zero cost when flat).
pub struct StereoBiquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    /// Per-channel state: [x1, x2, y1, y2].
    s: [[f32; 4]; 2],
    bypass: bool,
}

impl StereoBiquad {
    pub fn peaking(fs: f32, f0: f32, q: f32, gain_db: f32) -> Self {
        let mut bq = Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            s: [[0.0; 4]; 2],
            bypass: true,
        };
        bq.set_peaking(fs, f0, q, gain_db);
        bq
    }

    pub fn set_peaking(&mut self, fs: f32, f0: f32, q: f32, gain_db: f32) {
        self.bypass = gain_db.abs() < 0.05;
        if self.bypass {
            return;
        }
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * f0 / fs;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        let a0 = 1.0 + alpha / a;
        self.b0 = (1.0 + alpha * a) / a0;
        self.b1 = (-2.0 * cos_w0) / a0;
        self.b2 = (1.0 - alpha * a) / a0;
        self.a1 = (-2.0 * cos_w0) / a0;
        self.a2 = (1.0 - alpha / a) / a0;
    }

    /// Direct-form-I over interleaved stereo, in place.
    #[inline]
    pub fn process_interleaved(&mut self, buf: &mut [f32]) {
        if self.bypass {
            return;
        }
        for f in 0..buf.len() / 2 {
            for c in 0..2 {
                let x = buf[f * 2 + c];
                let st = &mut self.s[c];
                let y = self.b0 * x + self.b1 * st[0] + self.b2 * st[1]
                    - self.a1 * st[2]
                    - self.a2 * st[3];
                st[1] = st[0];
                st[0] = x;
                st[3] = st[2];
                st[2] = y;
                buf[f * 2 + c] = y;
            }
        }
    }
}

/// Cubic-rational soft clipper. Transparent below ~-6 dBFS, saturates
/// smoothly instead of hard-clipping when several lines sum above 0 dBFS.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
}

/// Streaming linear resampler for interleaved stereo f32.
///
/// Every line is normalized to a canonical 48 kHz stereo domain inside the
/// engine; this resampler adapts capture/render devices running at other
/// rates (44.1 k, 96 k...). Linear interpolation is fully adequate here —
/// this is a routing/monitoring tool, not a mastering chain.
pub struct Resampler {
    /// Nominal ratio (in_rate / out_rate).
    base: f64,
    /// Input frames consumed per output frame — `base` adjusted by the
    /// drift-servo trim.
    step: f64,
    /// Fractional read position in "virtual index" space, where index 0 is
    /// the last frame of the previous block and index k (1..=n) is frame
    /// k-1 of the current block. Peut être NÉGATIVE (jusqu'à -1) côté
    /// rendu quand `step < 1` — voir [`process_exact`](Self::process_exact).
    pos: f64,
    /// Last frame of the previous block (interpolation continuity).
    prev: [f32; 2],
    /// Avant-dernière frame du bloc précédent : sert quand `pos` repart
    /// négative, cas où l'interpolation démarre à l'index virtuel -1.
    prev2: [f32; 2],
    passthrough: bool,
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        let ratio = in_rate as f64 / out_rate as f64;
        Self {
            base: ratio,
            step: ratio,
            pos: 0.0,
            prev: [0.0; 2],
            prev2: [0.0; 2],
            passthrough: in_rate == out_rate,
        }
    }

    /// Drift servo: nudge the consumption rate by `trim` (e.g. ±0.003 =
    /// ±0.3%, inaudible) so the ring buffers stay near their target fill
    /// despite capture/render clock drift.
    #[inline]
    pub fn set_trim(&mut self, trim: f64) {
        self.step = self.base * (1.0 + trim);
    }

    /// Only exercised by tests today, but part of the resampler's API.
    #[allow(dead_code)]
    #[inline]
    pub fn passthrough(&self) -> bool {
        self.passthrough
    }

    /// Frame à l'index virtuel donné : -1 = avant-dernière frame du bloc
    /// précédent, 0 = dernière, k ≥ 1 = frame k-1 du bloc courant (bornée
    /// à la dernière disponible en cas de sous-alimentation).
    #[inline]
    fn frame_at(&self, input: &[f32], virtual_idx: isize) -> [f32; 2] {
        if virtual_idx <= -1 {
            self.prev2
        } else if virtual_idx == 0 {
            self.prev
        } else {
            let frames = input.len() / 2;
            let f = ((virtual_idx as usize) - 1).min(frames.saturating_sub(1));
            [input[f * 2], input[f * 2 + 1]]
        }
    }

    /// Mémorise les deux dernières frames du bloc pour la continuité.
    #[inline]
    fn remember_tail(&mut self, input: &[f32], n: usize) {
        self.prev2 = if n >= 2 {
            [input[(n - 2) * 2], input[(n - 2) * 2 + 1]]
        } else {
            self.prev
        };
        self.prev = [input[(n - 1) * 2], input[(n - 1) * 2 + 1]];
    }

    /// Capture side: consume a whole input block, append every producible
    /// output frame to `out` (variable count, ~n/step frames).
    pub fn process_all(&mut self, input: &[f32], out: &mut Vec<f32>) {
        out.clear();
        let n = input.len() / 2;
        if n == 0 {
            return;
        }
        while self.pos < n as f64 {
            // La boucle garantit pos ≥ 0 ici (elle ne s'arrête qu'une fois
            // pos ≥ n, et le rebasage laisse un résidu dans [0, step)).
            let i = self.pos.floor() as isize;
            let frac = (self.pos - i as f64) as f32;
            let a = self.frame_at(input, i);
            let b = self.frame_at(input, i + 1);
            out.push(a[0] + (b[0] - a[0]) * frac);
            out.push(a[1] + (b[1] - a[1]) * frac);
            self.pos += self.step;
        }
        self.pos -= n as f64;
        self.remember_tail(input, n);
    }

    /// Render side: how many input frames must be available to produce
    /// exactly `out_frames` output frames from the current position.
    /// (General formula — also exact when step == 1.0, so the drift trim
    /// works on same-rate paths too.)
    pub fn required_input(&self, out_frames: usize) -> usize {
        if out_frames == 0 {
            return 0;
        }
        let last_pos = self.pos + (out_frames as f64 - 1.0) * self.step;
        // `pos` peut être négative : passer par isize évite qu'un floor
        // négatif casté en usize ne devienne une valeur astronomique.
        let needed = last_pos.floor() as isize + 1;
        needed.max(0) as usize
    }

    /// Render side: produce exactly `out_frames` frames into `out` from an
    /// input block sized by [`required_input`]. Missing input (device
    /// underrun) must be zero-padded by the caller.
    pub fn process_exact(&mut self, input: &[f32], out_frames: usize, out: &mut Vec<f32>) {
        out.clear();
        let n = input.len() / 2;
        if n == 0 || out_frames == 0 {
            out.resize(out_frames * 2, 0.0);
            return;
        }
        for _ in 0..out_frames {
            let i = (self.pos.floor() as isize).min(n as isize - 1);
            let frac = (self.pos - i as f64) as f32;
            let a = self.frame_at(input, i);
            let b = self.frame_at(input, i + 1);
            out.push(a[0] + (b[0] - a[0]) * frac);
            out.push(a[1] + (b[1] - a[1]) * frac);
            self.pos += self.step;
        }
        self.pos -= n as f64;
        // Le résidu vaut frac(dernière position) + step - 1, donc il descend
        // jusqu'à step-1 : NÉGATIF dès que step < 1, c'est-à-dire dès que le
        // périphérique de rendu tourne au-dessus de 48 kHz (96/192 kHz sont
        // courants sur les DAC et casques USB). La prochaine interpolation
        // démarre alors entre l'avant-dernière et la dernière frame de ce
        // bloc — d'où `prev2`. L'ancien clamp à 0 jetait cette phase à
        // CHAQUE bloc (~100 fois par seconde une fois le servo en trim), ce
        // qui s'entend comme un crachotement périodique sur les aigus.
        if self.pos < -1.0 {
            self.pos = -1.0;
        }
        self.remember_tail(input, n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_rates() {
        let rs = Resampler::new(48_000, 48_000);
        assert!(rs.passthrough());
        assert_eq!(rs.required_input(128), 128);
    }

    #[test]
    fn upsample_produces_more_frames() {
        let mut rs = Resampler::new(44_100, 48_000);
        let input: Vec<f32> = (0..441 * 2).map(|i| (i % 2) as f32).collect();
        let mut out = Vec::new();
        rs.process_all(&input, &mut out);
        let frames = out.len() / 2;
        assert!((478..=482).contains(&frames), "got {frames}");
    }

    #[test]
    fn exact_output_count() {
        let mut rs = Resampler::new(48_000, 44_100);
        let need = rs.required_input(441);
        let input = vec![0.5f32; need * 2];
        let mut out = Vec::new();
        rs.process_exact(&input, 441, &mut out);
        assert_eq!(out.len(), 441 * 2);
    }

    /// Régression : sur une sortie PLUS RAPIDE que le domaine interne
    /// (step < 1 — un DAC à 96 kHz), la position résiduelle repart
    /// négative à certains blocs. L'ancien code la clampait à 0, jetant
    /// jusqu'à un demi-échantillon de phase par bloc. Sur une rampe
    /// parfaitement linéaire, un rééchantillonnage linéaire doit produire
    /// une rampe d'incrément CONSTANT : tout clamp se voit comme un saut.
    #[test]
    fn exact_preserves_phase_across_blocks_when_upsampling() {
        let (in_rate, out_rate) = (44_100u32, 96_000u32);
        let mut rs = Resampler::new(in_rate, out_rate);
        let expected = in_rate as f32 / out_rate as f32;

        let mut produced: Vec<f32> = Vec::new();
        let mut next_frame = 0.0f32; // rampe globale : frame k vaut k
        for _ in 0..8 {
            let need = rs.required_input(64);
            let mut input = Vec::with_capacity(need * 2);
            for _ in 0..need {
                input.push(next_frame);
                input.push(next_frame);
                next_frame += 1.0;
            }
            let mut out = Vec::new();
            rs.process_exact(&input, 64, &mut out);
            produced.extend(out.iter().step_by(2));
        }

        // On saute l'amorçage (prev/prev2 partent à zéro).
        let deltas: Vec<f32> = produced.windows(2).map(|w| w[1] - w[0]).collect();
        for (i, d) in deltas.iter().enumerate().skip(8) {
            assert!(
                (d - expected).abs() < 1e-3,
                "saut de phase à l'échantillon {i} : delta {d}, attendu {expected}"
            );
        }
    }

    #[test]
    fn soft_clip_is_bounded_and_transparent() {
        assert!(soft_clip(4.0) <= 1.01);
        assert!(soft_clip(-4.0) >= -1.01);
        assert!((soft_clip(0.2) - 0.2).abs() < 0.005);
    }

    #[test]
    fn eq_flat_band_is_bypassed() {
        let mut bq = StereoBiquad::peaking(48_000.0, 1_000.0, EQ_Q, 0.0);
        let mut buf: Vec<f32> = (0..256).map(|i| ((i % 7) as f32 - 3.0) * 0.1).collect();
        let orig = buf.clone();
        bq.process_interleaved(&mut buf);
        assert_eq!(buf, orig);
    }

    #[test]
    fn eq_boost_raises_center_frequency() {
        // 1 kHz sine through a +12 dB band at 1 kHz should come out ~4x hotter.
        let fs = 48_000.0;
        let mut bq = StereoBiquad::peaking(fs, 1_000.0, EQ_Q, 12.0);
        let mut buf = Vec::with_capacity(9600 * 2);
        for i in 0..9600 {
            let s = (2.0 * std::f32::consts::PI * 1_000.0 * i as f32 / fs).sin() * 0.1;
            buf.push(s);
            buf.push(s);
        }
        bq.process_interleaved(&mut buf);
        // Skip the transient, then measure the peak.
        let peak = buf[4800..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.3, "expected ~0.4 peak, got {peak}");
    }
}

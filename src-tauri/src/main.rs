//! MixFlow — virtual audio routing & mixing (SteelSeries Sonar-like).
//!
//! This file wires everything together:
//! - persistent [`AppConfig`] (JSON in the OS config dir),
//! - the shared [`Controls`] plane (atomics read by audio callbacks),
//! - the engine thread (owns the `!Send` cpal streams),
//! - per-app routing via Windows audio policy (see `winapps.rs`),
//! - the Tauri commands called by the React frontend,
//! - a 20 Hz "levels" event for the VU meters,
//! - the system tray (Discord-like minimize-to-tray).
//!
//! Command taxonomy — two kinds, and the distinction matters:
//! - **Topology commands** (add/remove line/output, device or route change):
//!   mutate the config, then rebuild the whole engine. Streams restart; a
//!   sub-100 ms gap is acceptable for structural edits.
//! - **Live commands** (gains, mutes, EQ, ducking amounts): mutate the
//!   config AND poke the matching atomic in the current `Controls`. The
//!   engine picks the new value up on the next audio block — no rebuild,
//!   no glitch.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod winapps;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait};
use parking_lot::{Mutex, RwLock};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, State};

use audio::controls::Controls;
use audio::dsp::EQ_FREQS;
use audio::engine::{self, EngineMsg};
use audio::model::{
    new_id, AppConfig, DeviceList, DuckRule, EqBandCfg, EqPreset, LevelsPayload, LineConfig,
    OutputConfig, Route, CURRENT_SCHEMA_VERSION,
};

/// Default parametric curve: the classic 5 flat bands.
fn default_bands() -> Vec<EqBandCfg> {
    EQ_FREQS
        .iter()
        .map(|&freq| EqBandCfg { freq, gain: 0.0 })
        .collect()
}

/// Minimum frequency ratio enforced between adjacent EQ points (~1/20th of
/// an octave). Two peaking filters stacked on nearly the same frequency add
/// their gains directly — the output soft-clip prevents digital clipping,
/// but the band ends up audibly squashed. Spacing points apart avoids that
/// without limiting how tightly a real multi-band curve can be shaped.
const MIN_BAND_FREQ_RATIO: f32 = 1.03;

/// Clamp, sort, cap and space out a band list coming from the UI.
fn sanitize_bands(mut bands: Vec<EqBandCfg>) -> Vec<EqBandCfg> {
    for b in &mut bands {
        b.freq = b.freq.clamp(20.0, 20_000.0);
        b.gain = b.gain.clamp(-12.0, 12.0);
    }
    bands.sort_by(|a, b| a.freq.total_cmp(&b.freq));
    bands.truncate(10);
    for i in 1..bands.len() {
        let min_freq = (bands[i - 1].freq * MIN_BAND_FREQ_RATIO).min(20_000.0);
        if bands[i].freq < min_freq {
            bands[i].freq = min_freq;
        }
    }
    bands
}
use winapps::AppInfo;

// ---------------------------------------------------------------------------
// State & persistence
// ---------------------------------------------------------------------------

struct AppState {
    /// Source of truth, persisted to disk on every mutation.
    config: Mutex<AppConfig>,
    /// Current control plane; swapped atomically on every rebuild.
    controls: RwLock<Arc<Controls>>,
    /// Channel to the engine thread (std mpsc Sender is !Sync — hence Mutex).
    engine_tx: Mutex<Sender<EngineMsg>>,
    config_path: PathBuf,
    /// Set by `persist()`, cleared by the debounced-save thread (see
    /// `main()`). Live commands (gain/mute/EQ/...) can fire many times a
    /// second during a fader drag; flagging "needs a save" instead of
    /// writing synchronously every time avoids hammering disk I/O.
    dirty: AtomicBool,
}

/// Write the config to disk right now if it's been marked dirty since the
/// last flush. Used by the debounced-save thread and on quit.
fn flush_if_dirty(state: &AppState) {
    if state.dirty.swap(false, Ordering::Relaxed) {
        let cfg = state.config.lock().clone();
        save_config(&state.config_path, &cfg);
    }
}

fn save_config(path: &Path, cfg: &AppConfig) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string_pretty(cfg) {
        Ok(json) => {
            if let Err(e) = std::fs::write(path, json) {
                eprintln!("[mixflow] failed to save config: {e}");
            }
        }
        Err(e) => eprintln!("[mixflow] failed to serialize config: {e}"),
    }
}

fn load_or_default(path: &Path) -> AppConfig {
    match std::fs::read_to_string(path) {
        Ok(json) => serde_json::from_str(&json).unwrap_or_else(|e| {
            eprintln!("[mixflow] config unreadable ({e}), starting fresh");
            default_config()
        }),
        Err(_) => default_config(),
    }
}

/// First-run config: the classic Sonar layout — Game / Chat / Media lines,
/// Headphones bus, and a "Chat ducks Game by 50%" priority rule. Devices are
/// left unassigned on purpose: the user picks them in the UI.
fn default_config() -> AppConfig {
    let out_id = new_id("out");
    let game = new_id("line");
    let chat = new_id("line");
    let media = new_id("line");
    let route = |gain: f32| Route {
        output_id: out_id.clone(),
        gain,
    };
    let line = |id: &String, name: &str, color: &str, kind: &str| LineConfig {
        id: id.clone(),
        name: name.into(),
        kind: kind.into(),
        color: color.into(),
        input_device: None,
        gain: 1.0,
        muted: false,
        eq: [0.0; 5],
        eq_bands: default_bands(),
        apps: Vec::new(),
        cable_render_id: None,
        routes: vec![route(1.0)],
    };
    let mic = new_id("line");
    AppConfig {
        lines: vec![
            line(&game, "Game", "#7c3aed", "app"),
            line(&chat, "Chat", "#22d3ee", "app"),
            line(&media, "Media", "#f59e0b", "app"),
            line(&mic, "Mic", "#fb923c", "mic"),
        ],
        outputs: vec![OutputConfig {
            id: out_id,
            name: "Headphones".into(),
            device: String::new(),
            gain: 1.0,
            muted: false,
        }],
        ducking: vec![DuckRule {
            source_line: chat,
            target_line: game,
            amount: 0.5,
        }],
        master_gain: 1.0,
        eq_presets: Vec::new(),
        schema_version: CURRENT_SCHEMA_VERSION,
    }
}

/// Persist the current config, snapshot fresh Controls from it, swap them in
/// and ask the engine thread to rebuild every stream.
fn rebuild(state: &AppState) {
    let cfg = state.config.lock().clone();
    save_config(&state.config_path, &cfg);
    let controls = Controls::from_config(&cfg);
    *state.controls.write() = controls.clone();
    let _ = state
        .engine_tx
        .lock()
        .send(EngineMsg::Rebuild(cfg, controls));
}

/// Mark the config dirty — used by live commands where the atomics were
/// poked directly and no stream restart is needed. The debounced-save
/// thread flushes to disk within ~800 ms; see `flush_if_dirty`.
fn persist(state: &AppState) {
    state.dirty.store(true, Ordering::Relaxed);
}

/// Does this capture-device name look like a virtual cable?
fn is_virtual_capture(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("cable")
        || n.contains("vb-audio")
        || n.contains("virtual")
        || n.contains("voicemeeter")
}

/// Sonar-style startup: every line without a source claims a free virtual
/// cable, and the cable's Windows render endpoint takes the line's name —
/// so "Game", "Chat", "Media" appear as selectable speakers in Windows
/// without any manual step. Best effort: no cable, no admin rights → the
/// line simply stays dormant.
///
/// Split into three phases (plan / resolve / apply) so callers that already
/// hold `state.config`'s lock can run the blocking `resolve` step (COM +
/// thread-spawn, see `winapps::in_com_thread`) OUTSIDE the lock — see
/// `add_line`. This function itself is only safe to call while `cfg` is
/// exclusively owned (e.g. before the app state exists, at startup).
fn auto_bind_lines(cfg: &mut AppConfig) {
    let plan = plan_cable_bindings(cfg);
    let resolved = resolve_cable_bindings(cfg, &plan);
    apply_cable_bindings(cfg, resolved);
}

/// Phase 1 — pure, no I/O: which lines need a cable, and which free cable
/// each one gets. Safe to call with just a snapshot/clone of the config.
fn plan_cable_bindings(cfg: &AppConfig) -> Vec<(String, String)> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    let cables: Vec<String> = devices
        .filter_map(|d| d.name().ok())
        .filter(|n| is_virtual_capture(n))
        .collect();

    let mut used: Vec<String> = cfg
        .lines
        .iter()
        .filter_map(|l| l.input_device.clone())
        .collect();
    let mut plan = Vec::new();
    for line in &cfg.lines {
        if line.kind == "mic" || line.input_device.is_some() {
            continue;
        }
        let Some(cable) = cables.iter().find(|c| !used.contains(c)) else {
            break;
        };
        used.push(cable.clone());
        plan.push((line.id.clone(), cable.clone()));
    }
    plan
}

/// Phase 2 — the blocking part: resolves each cable's render-side MMDevice
/// id and renames it after the line. Call this WITHOUT holding any
/// `state.config` lock (each resolution is a COM round-trip on its own
/// thread, see `winapps::in_com_thread`).
fn resolve_cable_bindings(
    cfg: &AppConfig,
    plan: &[(String, String)],
) -> Vec<(String, String, Option<String>)> {
    plan.iter()
        .map(|(line_id, cable)| {
            let line = cfg.lines.iter().find(|l| &l.id == line_id);
            let cached = line.and_then(|l| l.cable_render_id.clone());
            let name = line.map(|l| l.name.clone()).unwrap_or_default();
            let render_id = cached.or_else(|| winapps::render_id_for_capture(cable).ok());
            if let Some(rid) = &render_id {
                let _ = winapps::rename_render_device(rid, &name);
            }
            (line_id.clone(), cable.clone(), render_id)
        })
        .collect()
}

/// Phase 3 — pure mutation, keyed by line id (never by index: safe even if
/// lines were added/removed/reordered between `plan` and here). Skips a
/// line that already picked up a source in the meantime rather than
/// clobbering it with a stale auto-bind result.
fn apply_cable_bindings(cfg: &mut AppConfig, resolved: Vec<(String, String, Option<String>)>) {
    for (line_id, cable, render_id) in resolved {
        if let Some(line) = cfg.lines.iter_mut().find(|l| l.id == line_id) {
            if line.input_device.is_none() {
                line.input_device = Some(cable);
                line.cable_render_id = render_id;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Commands — queries
// ---------------------------------------------------------------------------

#[tauri::command]
fn list_devices() -> Result<DeviceList, String> {
    let host = cpal::default_host();
    let inputs = host
        .input_devices()
        .map_err(|e| e.to_string())?
        .filter_map(|d| d.name().ok())
        .collect();
    let outputs = host
        .output_devices()
        .map_err(|e| e.to_string())?
        .filter_map(|d| d.name().ok())
        .collect();
    Ok(DeviceList { inputs, outputs })
}

#[tauri::command]
fn get_config(state: State<AppState>) -> AppConfig {
    state.config.lock().clone()
}

/// Applications that currently own an audio session (volume-mixer style).
#[tauri::command]
fn list_apps() -> Result<Vec<AppInfo>, String> {
    winapps::list_apps()
}

// ---------------------------------------------------------------------------
// Commands — app drag-and-drop routing
// ---------------------------------------------------------------------------

/// Payload of `assign_app_to_line`: the new config plus an optional
/// non-fatal notice (e.g. the Windows device rename was denied).
#[derive(Clone, serde::Serialize)]
struct AssignResult {
    config: AppConfig,
    notice: Option<String>,
}

/// Drop an application onto a line:
/// 1. make sure the line captures a virtual cable (auto-pick a free one),
/// 2. tell Windows to route the app's audio to that cable's render side,
/// 3. rename that render endpoint after the line (Sonar-style: apps and the
///    Windows output picker show "Game (VB-Audio Virtual Cable)"),
/// 4. remember the app on the line and rebuild.
#[tauri::command]
fn assign_app_to_line(
    line_id: String,
    exe: String,
    state: State<AppState>,
) -> Result<AssignResult, String> {
    // -- 1. resolve (or auto-pick) the line's cable, capture side ------------
    let (capture_name, cached_render_id, line_name) = {
        let cfg = state.config.lock();
        let line = cfg
            .lines
            .iter()
            .find(|l| l.id == line_id)
            .ok_or("ligne introuvable")?;
        let capture = match &line.input_device {
            Some(d) if is_virtual_capture(d) => d.clone(),
            Some(d) => {
                return Err(format!(
                    "la tranche \"{}\" capture \"{d}\" (une entrée physique) — choisis d'abord un câble virtuel comme source, ou vide la source",
                    line.name
                ))
            }
            None => {
                // Auto-pick the first virtual cable not used by another line.
                let host = cpal::default_host();
                let candidates: Vec<String> = host
                    .input_devices()
                    .map_err(|e| e.to_string())?
                    .filter_map(|d| d.name().ok())
                    .filter(|n| is_virtual_capture(n))
                    .collect();
                let used: Vec<&String> = cfg
                    .lines
                    .iter()
                    .filter(|l| l.id != line_id)
                    .filter_map(|l| l.input_device.as_ref())
                    .collect();
                candidates
                    .into_iter()
                    .find(|c| !used.contains(&c))
                    .ok_or_else(|| {
                        "aucun câble virtuel libre — installe VB-Cable (et le pack A+B pour plus de lignes)"
                            .to_string()
                    })?
            }
        };
        (capture, line.cable_render_id.clone(), line.name.clone())
    };

    // -- 2. resolve the cable's render side (id cached after the first time;
    //       the pairing survives MixFlow's own endpoint renames) -------------
    let render_id = match cached_render_id {
        Some(id) => id,
        None => winapps::render_id_for_capture(&capture_name)?,
    };
    winapps::route_app_by_id(&exe, &render_id)?;

    // -- 3. Sonar-style: the Windows device takes the line's name ------------
    let notice = winapps::rename_render_device(&render_id, &line_name)
        .err()
        .map(|e| format!("App routée, mais {e}"));

    // -- 4. persist on the line + rebuild ------------------------------------
    let cfg_after = {
        let mut cfg = state.config.lock();
        // Narrow race: the line could have been deleted while we were doing
        // the (unlocked) Windows/COM work above. Rather than silently
        // leaving the app routed to an orphaned cable nobody in the UI
        // tracks anymore, undo the Windows-level routing and report it.
        if !cfg.lines.iter().any(|l| l.id == line_id) {
            drop(cfg);
            let _ = winapps::unroute_app(&exe);
            return Err("la ligne a été supprimée pendant le routage — annulé".to_string());
        }
        // An app lives on at most one line.
        for l in &mut cfg.lines {
            l.apps.retain(|a| !a.eq_ignore_ascii_case(&exe));
        }
        if let Some(line) = cfg.lines.iter_mut().find(|l| l.id == line_id) {
            line.input_device = Some(capture_name);
            line.cable_render_id = Some(render_id);
            line.apps.push(exe);
        }
        cfg.clone()
    };
    rebuild(&state);
    Ok(AssignResult {
        config: cfg_after,
        notice,
    })
}

/// Remove an app from a line and hand its audio back to the system default.
#[tauri::command]
fn unassign_app_from_line(
    line_id: String,
    exe: String,
    state: State<AppState>,
) -> Result<AppConfig, String> {
    // Best-effort: if the app has no live session, Windows keeps the old
    // persisted route until the app is re-assigned; nothing else we can do.
    let _ = winapps::unroute_app(&exe);
    let cfg_after = {
        let mut cfg = state.config.lock();
        if let Some(line) = cfg.lines.iter_mut().find(|l| l.id == line_id) {
            line.apps.retain(|a| !a.eq_ignore_ascii_case(&exe));
        }
        cfg.clone()
    };
    rebuild(&state);
    Ok(cfg_after)
}

// ---------------------------------------------------------------------------
// Commands — topology (config mutation + engine rebuild)
// ---------------------------------------------------------------------------

#[tauri::command]
fn add_line(name: String, color: String, kind: String, state: State<AppState>) -> AppConfig {
    // Push the new line and snapshot the config under a SHORT lock (no COM
    // calls in this block) — the blocking cable-binding work happens after
    // we've released `state.config`, so it never stalls other commands.
    let snapshot = {
        let mut cfg = state.config.lock();
        // A single route, like every other line: `set_line_output` and the
        // "Sortie" dropdown assume one bus per line. Route to the first
        // existing bus (if any) — fanning out to ALL of them was a relic of
        // the old routing-matrix UI and silently duplicated playback across
        // every output the moment 2+ buses existed.
        let routes = cfg
            .outputs
            .first()
            .map(|o| Route {
                output_id: o.id.clone(),
                gain: 1.0,
            })
            .into_iter()
            .collect();
        // Les micros démarrent coupés : le retour ("s'entendre") s'active
        // explicitement via le bouton casque de la tranche.
        let muted = kind == "mic";
        cfg.lines.push(LineConfig {
            id: new_id("line"),
            name,
            kind,
            color,
            input_device: None,
            gain: 1.0,
            muted,
            eq: [0.0; 5],
            eq_bands: default_bands(),
            apps: Vec::new(),
            cable_render_id: None,
            routes,
        });
        cfg.clone()
    };

    // If a cable is still free, the new line becomes a Windows speaker
    // right away, like the default lines — resolved outside any lock.
    let plan = plan_cable_bindings(&snapshot);
    let resolved = resolve_cable_bindings(&snapshot, &plan);

    let cfg_after = {
        let mut cfg = state.config.lock();
        apply_cable_bindings(&mut cfg, resolved);
        cfg.clone()
    };
    rebuild(&state);
    cfg_after
}

#[tauri::command]
fn remove_line(id: String, state: State<AppState>) -> AppConfig {
    // Read the line's routed apps, then release the lock BEFORE the blocking
    // Windows COM calls (each `unroute_app` spawns and joins its own thread)
    // — holding `state.config` across those would stall every other command
    // (gain/mute/etc.) for as long as Windows Audio takes to answer.
    let apps: Vec<String> = {
        let cfg = state.config.lock();
        cfg.lines
            .iter()
            .find(|l| l.id == id)
            .map(|l| l.apps.clone())
            .unwrap_or_default()
    };
    for exe in &apps {
        let _ = winapps::unroute_app(exe);
    }
    let cfg_after = {
        let mut cfg = state.config.lock();
        cfg.lines.retain(|l| l.id != id);
        cfg.ducking
            .retain(|d| d.source_line != id && d.target_line != id);
        cfg.clone()
    };
    rebuild(&state);
    cfg_after
}

/// Bind a line to a SET of physical speakers/headphones at once (fan-out —
/// e.g. monitor on headphones AND speakers simultaneously). The output
/// buses still exist in the engine, but they are invisible plumbing now:
/// this command finds-or-creates a bus per device, replaces the line's
/// routes to match exactly that set, and garbage-collects buses no line
/// uses anymore. A device that stays selected keeps its existing gain.
#[tauri::command]
fn set_line_outputs(line_id: String, devices: Vec<String>, state: State<AppState>) -> AppConfig {
    let cfg_after = {
        let mut cfg = state.config.lock();
        let existing_gain_by_device: HashMap<String, f32> = cfg
            .lines
            .iter()
            .find(|l| l.id == line_id)
            .into_iter()
            .flat_map(|l| &l.routes)
            .filter_map(|r| {
                cfg.outputs
                    .iter()
                    .find(|o| o.id == r.output_id)
                    .map(|o| (o.device.clone(), r.gain))
            })
            .collect();

        let mut new_routes = Vec::with_capacity(devices.len());
        for device in devices.iter().filter(|d| !d.is_empty()) {
            let bus_id = match cfg.outputs.iter().find(|o| &o.device == device) {
                Some(o) => o.id.clone(),
                None => {
                    let id = new_id("out");
                    // Short display name: "Haut-parleurs (Creative…)" -> "Haut-parleurs".
                    let name = device.split(" (").next().unwrap_or(device).to_string();
                    cfg.outputs.push(OutputConfig {
                        id: id.clone(),
                        name,
                        device: device.clone(),
                        gain: 1.0,
                        muted: false,
                    });
                    id
                }
            };
            let gain = existing_gain_by_device.get(device).copied().unwrap_or(1.0);
            new_routes.push(Route {
                output_id: bus_id,
                gain,
            });
        }

        if let Some(line) = cfg.lines.iter_mut().find(|l| l.id == line_id) {
            line.routes = new_routes;
        }
        prune_unused_buses(&mut cfg);
        cfg.clone()
    };
    rebuild(&state);
    cfg_after
}

/// Drop output buses no line routes to (they are auto-created per device).
fn prune_unused_buses(cfg: &mut AppConfig) {
    let used: std::collections::HashSet<String> = cfg
        .lines
        .iter()
        .flat_map(|l| l.routes.iter().map(|r| r.output_id.clone()))
        .collect();
    cfg.outputs.retain(|o| used.contains(&o.id));
}

#[tauri::command]
fn set_line_input(id: String, device: Option<String>, state: State<AppState>) -> AppConfig {
    let cfg_after = {
        let mut cfg = state.config.lock();
        if let Some(line) = cfg.lines.iter_mut().find(|l| l.id == id) {
            line.input_device = device;
            // The cached render id belonged to the previous cable.
            line.cable_render_id = None;
        }
        cfg.clone()
    };
    rebuild(&state);
    cfg_after
}

// ---------------------------------------------------------------------------
// Commands — cosmetic (config only, nothing audible changes)
// ---------------------------------------------------------------------------

#[tauri::command]
fn update_line_meta(id: String, name: String, color: String, state: State<AppState>) {
    let render_id = {
        let mut cfg = state.config.lock();
        if let Some(line) = cfg.lines.iter_mut().find(|l| l.id == id) {
            line.name = name.clone();
            line.color = color;
            line.cable_render_id.clone()
        } else {
            None
        }
    };
    // Keep the Windows device name in sync with the line name (best effort).
    if let Some(rid) = render_id {
        let _ = winapps::rename_render_device(&rid, &name);
    }
    persist(&state);
}

// ---------------------------------------------------------------------------
// Commands — live parameters (atomics, no rebuild, no glitch)
// ---------------------------------------------------------------------------

#[tauri::command]
fn set_line_gain(id: String, gain: f32, state: State<AppState>) {
    let gain = gain.clamp(0.0, 1.5);
    {
        let mut cfg = state.config.lock();
        if let Some(line) = cfg.lines.iter_mut().find(|l| l.id == id) {
            line.gain = gain;
        }
    }
    if let Some(ctl) = state.controls.read().lines.get(&id) {
        ctl.gain.set(gain);
    }
    persist(&state);
}

#[tauri::command]
fn set_line_muted(id: String, muted: bool, state: State<AppState>) {
    {
        let mut cfg = state.config.lock();
        if let Some(line) = cfg.lines.iter_mut().find(|l| l.id == id) {
            line.muted = muted;
        }
    }
    if let Some(ctl) = state.controls.read().lines.get(&id) {
        ctl.muted.store(muted, Ordering::Relaxed);
    }
    persist(&state);
}

/// Replace a line's parametric EQ curve (add/move/remove points). Live: the
/// capture callback picks the change up on the next block via `try_read`.
#[tauri::command]
fn set_line_eq_bands(id: String, bands: Vec<EqBandCfg>, state: State<AppState>) {
    let bands = sanitize_bands(bands);
    {
        let mut cfg = state.config.lock();
        if let Some(line) = cfg.lines.iter_mut().find(|l| l.id == id) {
            line.eq_bands = bands.clone();
        }
    }
    if let Some(ctl) = state.controls.read().lines.get(&id) {
        *ctl.eq.write() = bands;
    }
    persist(&state);
}

/// Save (or overwrite) a user EQ preset (full parametric curve).
#[tauri::command]
fn save_eq_preset(name: String, bands: Vec<EqBandCfg>, state: State<AppState>) -> AppConfig {
    let name = name.trim().to_string();
    let cfg_after = {
        let mut cfg = state.config.lock();
        cfg.eq_presets
            .retain(|p| !p.name.eq_ignore_ascii_case(&name));
        cfg.eq_presets.push(EqPreset {
            name,
            gains: Vec::new(),
            bands: sanitize_bands(bands),
        });
        cfg.eq_presets.sort_by(|a, b| a.name.cmp(&b.name));
        cfg.clone()
    };
    persist(&state);
    cfg_after
}

#[tauri::command]
fn delete_eq_preset(name: String, state: State<AppState>) -> AppConfig {
    let cfg_after = {
        let mut cfg = state.config.lock();
        cfg.eq_presets
            .retain(|p| !p.name.eq_ignore_ascii_case(&name));
        cfg.clone()
    };
    persist(&state);
    cfg_after
}

/// Global MASTER fader — the ceiling of every track, applied to all output
/// buses in the render callbacks. Live, no rebuild.
#[tauri::command]
fn set_master_gain(gain: f32, state: State<AppState>) {
    let gain = gain.clamp(0.0, 1.0);
    {
        let mut cfg = state.config.lock();
        cfg.master_gain = gain;
    }
    state.controls.read().master.set(gain);
    persist(&state);
}

/// Replace the whole ducking rule set. Rules live behind an RwLock that the
/// render callbacks `try_read`, so this is glitch-free too.
#[tauri::command]
fn set_duck_rules(rules: Vec<DuckRule>, state: State<AppState>) {
    {
        let mut cfg = state.config.lock();
        cfg.ducking = rules.clone();
    }
    *state.controls.read().ducking.write() = rules;
    persist(&state);
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    tauri::Builder::default()
        // Discord-like behavior: closing the window hides it to the tray,
        // the audio engine keeps running in the background.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            // System tray: left-click reopens, menu offers open/quit.
            let open = MenuItem::with_id(app, "open", "Ouvrir MixFlow", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            TrayIconBuilder::with_id("mixflow-tray")
                .icon(
                    app.default_window_icon()
                        .expect("bundle icon missing")
                        .clone(),
                )
                .tooltip("MixFlow — routage audio actif")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => {
                        // Best-effort: flush any pending debounced save
                        // before the process actually exits.
                        if let Some(state) = app.try_state::<AppState>() {
                            flush_if_dirty(&state);
                        }
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(w) = tray.app_handle().get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;

            let config_path = app
                .path()
                .app_config_dir()
                .expect("no config dir on this platform")
                .join("mixflow.config.json");
            let mut config = load_or_default(&config_path);
            // Migration EQ fixe (5 bandes) → EQ paramétrique.
            for line in &mut config.lines {
                if line.eq_bands.is_empty() {
                    line.eq_bands = EQ_FREQS
                        .iter()
                        .zip(line.eq.iter())
                        .map(|(&freq, &gain)| EqBandCfg { freq, gain })
                        .collect();
                }
            }
            for preset in &mut config.eq_presets {
                if preset.bands.is_empty() && preset.gains.len() == EQ_FREQS.len() {
                    preset.bands = EQ_FREQS
                        .iter()
                        .zip(preset.gains.iter())
                        .map(|(&freq, &gain)| EqBandCfg { freq, gain })
                        .collect();
                }
            }
            // Migration : les canaux d'apps ne capturent plus de matériel
            // physique (l'UI loopback a été retirée) — on libère ces sources.
            for line in &mut config.lines {
                if line.kind != "mic" {
                    if let Some(d) = &line.input_device {
                        if !is_virtual_capture(d) {
                            line.input_device = None;
                            line.cable_render_id = None;
                        }
                    }
                }
            }
            // Migration : garantir une tranche Micro.
            if !config.lines.iter().any(|l| l.kind == "mic") {
                let routes = config
                    .outputs
                    .iter()
                    .map(|o| Route {
                        output_id: o.id.clone(),
                        gain: 1.0,
                    })
                    .collect();
                config.lines.push(LineConfig {
                    id: new_id("line"),
                    name: "Mic".into(),
                    kind: "mic".into(),
                    color: "#fb923c".into(),
                    input_device: None,
                    gain: 1.0,
                    muted: true, // retour micro désactivé par défaut
                    eq: [0.0; 5],
                    eq_bands: default_bands(),
                    apps: Vec::new(),
                    cable_render_id: None,
                    routes,
                });
            }
            // Migration : chaque ligne doit sortir quelque part. Les routes
            // orphelines sont nettoyées, et toute ligne sans sortie est
            // branchée sur le périphérique de sortie par défaut de Windows
            // (sauf si c'est un de nos câbles — boucle garantie sinon).
            {
                let bus_ids: std::collections::HashSet<String> =
                    config.outputs.iter().map(|o| o.id.clone()).collect();
                for line in &mut config.lines {
                    line.routes.retain(|r| bus_ids.contains(&r.output_id));
                }
                let default_dev = cpal::default_host()
                    .default_output_device()
                    .and_then(|d| d.name().ok())
                    .filter(|n| !is_virtual_capture(n));
                if let Some(dev) = default_dev {
                    let needs: Vec<usize> = (0..config.lines.len())
                        .filter(|&i| config.lines[i].routes.is_empty())
                        .collect();
                    if !needs.is_empty() {
                        let bus_id = match config.outputs.iter().find(|o| o.device == dev) {
                            Some(o) => o.id.clone(),
                            None => {
                                let id = new_id("out");
                                let name = dev.split(" (").next().unwrap_or(&dev).to_string();
                                config.outputs.push(OutputConfig {
                                    id: id.clone(),
                                    name,
                                    device: dev.clone(),
                                    gain: 1.0,
                                    muted: false,
                                });
                                id
                            }
                        };
                        for i in needs {
                            config.lines[i].routes.push(Route {
                                output_id: bus_id.clone(),
                                gain: 1.0,
                            });
                        }
                    }
                }
                prune_unused_buses(&mut config);
            }
            // Claim free cables + rename their Windows endpoints so the
            // lines exist as speakers in Windows from the very first launch.
            auto_bind_lines(&mut config);
            config.schema_version = CURRENT_SCHEMA_VERSION;
            save_config(&config_path, &config);
            let controls = Controls::from_config(&config);

            // Engine thread — owns the !Send cpal streams.
            let (tx, rx) = std::sync::mpsc::channel();
            engine::spawn(app.handle().clone(), rx);
            let _ = tx.send(EngineMsg::Rebuild(config.clone(), controls.clone()));

            app.manage(AppState {
                config: Mutex::new(config),
                controls: RwLock::new(controls),
                engine_tx: Mutex::new(tx),
                config_path,
                dirty: AtomicBool::new(false),
            });

            // VU meter pump: 20 Hz "levels" event. Peaks are read-and-halved
            // so a stopped stream visibly falls back to zero.
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("mixflow-levels".into())
                .spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(50));
                    let state = handle.state::<AppState>();
                    let controls = state.controls.read().clone();
                    let mut lines = HashMap::new();
                    for (id, ctl) in &controls.lines {
                        let p = ctl.peak.get();
                        ctl.peak.set(p * 0.5);
                        lines.insert(id.clone(), p);
                    }
                    let mut outputs = HashMap::new();
                    for (id, ctl) in &controls.outputs {
                        let p = ctl.peak.get();
                        ctl.peak.set(p * 0.5);
                        outputs.insert(id.clone(), p);
                    }
                    let _ = handle.emit("levels", LevelsPayload { lines, outputs });
                })
                .expect("failed to spawn levels thread");

            // Debounced persistence: live commands only flag the config
            // dirty (see `persist`); this thread flushes to disk on a fixed
            // cadence instead of once per fader tick.
            let handle2 = app.handle().clone();
            std::thread::Builder::new()
                .name("mixflow-persist".into())
                .spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(800));
                    flush_if_dirty(&handle2.state::<AppState>());
                })
                .expect("failed to spawn persist thread");

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_devices,
            get_config,
            list_apps,
            assign_app_to_line,
            unassign_app_from_line,
            add_line,
            remove_line,
            set_line_input,
            set_line_outputs,
            update_line_meta,
            set_line_gain,
            set_line_muted,
            set_line_eq_bands,
            save_eq_preset,
            delete_eq_preset,
            set_master_gain,
            set_duck_rules
        ])
        .run(tauri::generate_context!())
        .expect("error while running MixFlow");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band(freq: f32, gain: f32) -> EqBandCfg {
        EqBandCfg { freq, gain }
    }

    #[test]
    fn sanitize_bands_clamps_sorts_and_caps() {
        let out = sanitize_bands(vec![
            band(50_000.0, 20.0),
            band(-10.0, -20.0),
            band(500.0, 3.0),
        ]);
        assert_eq!(out.len(), 3);
        assert!(out.windows(2).all(|w| w[0].freq <= w[1].freq));
        assert_eq!(out[0].freq, 20.0);
        assert_eq!(out[0].gain, -12.0);
        assert_eq!(out.last().unwrap().freq, 20_000.0);
        assert_eq!(out.last().unwrap().gain, 12.0);
    }

    #[test]
    fn sanitize_bands_truncates_to_ten() {
        let bands: Vec<EqBandCfg> = (0..15)
            .map(|i| band(100.0 + i as f32 * 500.0, 1.0))
            .collect();
        assert_eq!(sanitize_bands(bands).len(), 10);
    }

    #[test]
    fn sanitize_bands_spaces_out_overlapping_points() {
        // Two points dropped almost on top of each other must not stay
        // stacked — a coincident pair of +12 dB peaking filters would push
        // the combined boost at that frequency far past any single band's
        // ±12 dB clamp.
        let out = sanitize_bands(vec![band(1000.0, 8.0), band(1000.5, -4.0)]);
        assert_eq!(out.len(), 2);
        assert!(
            out[1].freq >= out[0].freq * MIN_BAND_FREQ_RATIO - 0.01,
            "points still overlap: {:?}",
            out
        );
    }

    #[test]
    fn sanitize_bands_leaves_well_spaced_points_alone() {
        let input = vec![band(80.0, 2.0), band(1000.0, -3.0), band(8000.0, 4.0)];
        let out = sanitize_bands(input.clone());
        for (a, b) in input.iter().zip(out.iter()) {
            assert_eq!(a.freq, b.freq);
            assert_eq!(a.gain, b.gain);
        }
    }
}

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
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use audio::controls::Controls;
use audio::dsp::EQ_FREQS;
use audio::engine::{self, EngineMsg};
use audio::model::{
    new_id, reactivity_decay, AppConfig, DeviceList, DuckRule, EqBandCfg, EqPreset, LevelsPayload,
    LineConfig, LineSnapshot, OutputConfig, Profile, Route, CURRENT_SCHEMA_VERSION,
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

fn finite_or(v: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        fallback
    }
}

/// Ramène une config venue du disque ou d'un import aux invariants que le
/// reste du code suppose. Les commandes live clampent leurs entrées une par
/// une, mais un JSON écrit à la main peut porter n'importe quoi : un gain
/// `1e300` devient `inf` en f32 (serde passe par f64 puis cast), l'inf
/// contamine le lissage de gain du rendu ((inf-inf)*k = NaN) puis l'état
/// des biquads — canal définitivement muet ou bruit fort. On borne donc
/// tout, on dédoublonne ids et routes (deux routes vers le même bus = deux
/// ring buffers = son doublé), on purge les références orphelines et on
/// applique la migration EQ legacy (un export d'une vieille version
/// n'aurait sinon jamais de bandes paramétriques).
fn sanitize_config(cfg: &mut AppConfig) {
    use std::collections::HashSet;

    let mut seen_outs = HashSet::new();
    cfg.outputs.retain(|o| seen_outs.insert(o.id.clone()));
    let mut seen_lines = HashSet::new();
    cfg.lines.retain(|l| seen_lines.insert(l.id.clone()));

    for o in &mut cfg.outputs {
        o.gain = finite_or(o.gain, 1.0).clamp(0.0, 1.5);
    }

    let bus_ids: HashSet<String> = cfg.outputs.iter().map(|o| o.id.clone()).collect();
    for line in &mut cfg.lines {
        line.gain = finite_or(line.gain, 1.0).clamp(0.0, 1.5);
        if line.eq_bands.is_empty() {
            line.eq_bands = EQ_FREQS
                .iter()
                .zip(line.eq.iter())
                .map(|(&freq, &gain)| EqBandCfg { freq, gain })
                .collect();
        }
        for b in &mut line.eq_bands {
            b.freq = finite_or(b.freq, 1_000.0);
            b.gain = finite_or(b.gain, 0.0);
        }
        line.eq_bands = sanitize_bands(std::mem::take(&mut line.eq_bands));
        if !matches!(
            line.duck_reactivity.as_str(),
            "douce" | "normale" | "rapide"
        ) {
            line.duck_reactivity = "normale".into();
        }
        let mut seen_routes = HashSet::new();
        line.routes
            .retain(|r| bus_ids.contains(&r.output_id) && seen_routes.insert(r.output_id.clone()));
        for r in &mut line.routes {
            r.gain = finite_or(r.gain, 1.0).clamp(0.0, 1.5);
        }
    }

    cfg.master_gain = finite_or(cfg.master_gain, 1.0).clamp(0.0, 1.0);

    let line_ids: HashSet<String> = cfg.lines.iter().map(|l| l.id.clone()).collect();
    // `source == target` ferait qu'une ligne s'atténue elle-même selon son
    // propre niveau (pompage audible) — voir `set_duck_rules`.
    cfg.ducking.retain(|d| {
        d.source_line != d.target_line
            && line_ids.contains(&d.source_line)
            && line_ids.contains(&d.target_line)
    });
    for d in &mut cfg.ducking {
        d.amount = finite_or(d.amount, 0.5).clamp(0.0, 1.0);
    }

    for p in &mut cfg.eq_presets {
        if p.bands.is_empty() && p.gains.len() == EQ_FREQS.len() {
            p.bands = EQ_FREQS
                .iter()
                .zip(p.gains.iter())
                .map(|(&freq, &gain)| EqBandCfg { freq, gain })
                .collect();
        }
        for b in &mut p.bands {
            b.freq = finite_or(b.freq, 1_000.0);
            b.gain = finite_or(b.gain, 0.0);
        }
        p.bands = sanitize_bands(std::mem::take(&mut p.bands));
    }

    for prof in &mut cfg.profiles {
        prof.master_gain = finite_or(prof.master_gain, 1.0).clamp(0.0, 1.0);
        prof.ducking
            .retain(|d| line_ids.contains(&d.source_line) && line_ids.contains(&d.target_line));
        for d in &mut prof.ducking {
            d.amount = finite_or(d.amount, 0.5).clamp(0.0, 1.0);
        }
        for snap in &mut prof.lines {
            snap.gain = finite_or(snap.gain, 1.0).clamp(0.0, 1.5);
            for b in &mut snap.eq_bands {
                b.freq = finite_or(b.freq, 1_000.0);
                b.gain = finite_or(b.gain, 0.0);
            }
            snap.eq_bands = sanitize_bands(std::mem::take(&mut snap.eq_bands));
            // Un bus jamais assigné à un périphérique se photographie en "".
            snap.output_devices.retain(|d| !d.is_empty());
        }
    }
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
    /// Dernier profil appliqué — partagé entre la commande `apply_profile`
    /// (manuelle) et le thread d'auto-switch, pour que le thread ne
    /// ré-applique pas 2 s plus tard un profil que l'utilisateur vient de
    /// poser à la main (rebuild inutile + réglages écrasés).
    active_profile: Mutex<Option<String>>,
}

/// Write the config to disk right now if it's been marked dirty since the
/// last flush. Used by the debounced-save thread and on quit.
///
/// L'écriture se fait SOUS le verrou config : elle est ainsi sérialisée
/// avec celle de `rebuild()` (qui tient le même verrou), sinon un flush
/// dont le clone date d'avant un rebuild concurrent pouvait réécrire une
/// config périmée par-dessus la fraîche.
fn flush_if_dirty(state: &AppState) {
    if state.dirty.swap(false, Ordering::Relaxed) {
        let cfg = state.config.lock();
        save_config(&state.config_path, &cfg);
    }
}

fn save_config(path: &Path, cfg: &AppConfig) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string_pretty(cfg) {
        Ok(json) => {
            // Écriture atomique (tmp + rename, MOVEFILE_REPLACE_EXISTING sous
            // Windows) : un crash ou une coupure en pleine écriture ne peut
            // plus laisser un JSON tronqué — que `load_or_default` aurait
            // remplacé en silence par la config d'usine au démarrage suivant.
            let tmp = path.with_extension("json.tmp");
            let res = std::fs::write(&tmp, json).and_then(|()| std::fs::rename(&tmp, path));
            if let Err(e) = res {
                eprintln!("[mixflow] failed to save config: {e}");
            }
        }
        Err(e) => eprintln!("[mixflow] failed to serialize config: {e}"),
    }
}

/// Message à montrer à l'utilisateur au prochain affichage de l'UI (config
/// illisible mise de côté…). Rempli au démarrage, avant que `AppState`
/// n'existe ; consommé une fois par la commande `take_startup_notice`.
static STARTUP_NOTICE: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new();

fn set_startup_notice(msg: String) {
    eprintln!("[mixflow] {msg}");
    *STARTUP_NOTICE.get_or_init(|| Mutex::new(None)).lock() = Some(msg);
}

/// Récupérée une seule fois par le frontend au montage : le binaire de
/// release tourne sans console (`windows_subsystem = "windows"`), un
/// `eprintln!` y est invisible.
#[tauri::command]
fn take_startup_notice() -> Option<String> {
    STARTUP_NOTICE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .take()
}

/// Un JSON syntaxiquement valide n'est pas pour autant une config MixFlow :
/// TOUS les champs d'`AppConfig` sont `#[serde(default)]` et serde ignore
/// les clés inconnues, donc `{}` — et par conséquent n'importe quel objet
/// JSON, `package.json` compris — se désérialise sans erreur en une config
/// VIDE. On exige donc la signature structurelle du format.
fn looks_like_mixflow_config(v: &serde_json::Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    obj.get("lines").map(|x| x.is_array()).unwrap_or(false)
        && obj.get("outputs").map(|x| x.is_array()).unwrap_or(false)
}

/// Copie de sûreté avant d'écraser la config courante (import). Renvoie le
/// chemin écrit, pour pouvoir le citer à l'utilisateur.
fn backup_config(path: &Path) -> Option<PathBuf> {
    let bak = path.with_extension("json.bak");
    std::fs::copy(path, &bak).ok().map(|_| bak)
}

fn load_or_default(path: &Path) -> AppConfig {
    match std::fs::read_to_string(path) {
        Ok(json) => match serde_json::from_str::<AppConfig>(&json) {
            Ok(mut cfg) => {
                // Le fichier peut avoir été édité à la main : mêmes garde-fous
                // qu'à l'import.
                sanitize_config(&mut cfg);
                cfg
            }
            Err(e) => {
                // NE PAS laisser `main()` réécrire par-dessus : le fichier
                // fautif est souvent réparable à la main (une virgule en
                // trop, une troncature) et serait sinon perdu à jamais.
                let corrupt = path.with_extension("json.corrupt");
                let saved = std::fs::rename(path, &corrupt).is_ok();
                set_startup_notice(if saved {
                    format!(
                        "Configuration illisible ({e}) — repartie des réglages d'usine. \
                         L'ancien fichier est conservé ici : {}",
                        corrupt.display()
                    )
                } else {
                    format!("Configuration illisible ({e}) — repartie des réglages d'usine.")
                });
                default_config()
            }
        },
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
        duck_reactivity: "normale".into(),
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
        profiles: Vec::new(),
        schema_version: CURRENT_SCHEMA_VERSION,
    }
}

/// Persist the current config, snapshot fresh Controls from it, swap them in
/// and ask the engine thread to rebuild every stream.
///
/// Le verrou config est tenu d'un bout à l'autre, pour deux raisons :
/// 1. deux rebuilds concurrents (main thread vs thread mixflow-profiles)
///    pouvaient entrelacer leurs swaps de Controls et leurs envois au
///    moteur — le moteur finissait câblé sur un plan de contrôle que plus
///    aucune commande live n'écrivait (faders/mutes sans effet audible) ;
/// 2. une commande live qui s'intercalait entre le clone et le swap
///    écrivait sa valeur dans l'ANCIEN Controls, perdue après le swap.
///
/// Aucun appelant de `rebuild` ne tient déjà ce verrou (vérifié — sinon
/// deadlock, parking_lot n'est pas réentrant).
fn rebuild(state: &AppState) {
    let cfg_guard = state.config.lock();
    let cfg = cfg_guard.clone();
    save_config(&state.config_path, &cfg);
    let controls = Controls::from_config(&cfg);
    *state.controls.write() = controls.clone();
    let _ = state
        .engine_tx
        .lock()
        .send(EngineMsg::Rebuild(cfg, controls));
    drop(cfg_guard);
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

/// Suffixe d'adaptateur "(VB-Audio …)" d'un nom d'endpoint — la partie que
/// MixFlow ne renomme jamais (voir `render_id_for_capture`).
fn adapter_suffix(name: &str) -> Option<&str> {
    name.rfind('(').map(|i| &name[i..])
}

/// Le côté RENDU de ce câble sert-il de périphérique à un bus de sortie
/// (montage streamer : les lignes jouent vers un câble capturé par OBS) ?
/// Un tel câble n'est PAS libre : se l'approprier comme source renommerait
/// l'endpoint que le bus référence par NOM (flux stream tué au rebuild
/// suivant) et réinjecterait le mix diffusé dans une ligne (boucle).
fn cable_used_as_output(cfg: &AppConfig, capture_name: &str) -> bool {
    let Some(suffix) = adapter_suffix(capture_name) else {
        return false;
    };
    cfg.outputs
        .iter()
        .any(|o| is_virtual_capture(&o.device) && adapter_suffix(&o.device) == Some(suffix))
}

/// Réconciliations appliquées à TOUTE config qui entre dans l'application —
/// démarrage comme import. Là où `sanitize_config` ne fait que borner des
/// valeurs, celles-ci garantissent la structure en tenant compte de
/// l'environnement réel (périphériques présents, câbles libres).
///
/// Contient du COM bloquant (`auto_bind_lines`) : à n'appeler QUE hors du
/// verrou `state.config`.
fn reconcile_config(config: &mut AppConfig) {
    // Les canaux d'apps ne capturent plus de matériel physique (l'UI
    // loopback a été retirée) — on libère ces sources.
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
    // Garantir une tranche Micro.
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
            duck_reactivity: "normale".into(),
        });
    }
    // Chaque ligne doit sortir quelque part : les routes orphelines sont
    // nettoyées, et toute ligne sans sortie est branchée sur le
    // périphérique par défaut de Windows (jamais sur un de nos câbles —
    // boucle garantie sinon).
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
                let bus_id = find_or_create_bus(config, &dev);
                for i in needs {
                    config.lines[i].routes.push(Route {
                        output_id: bus_id.clone(),
                        gain: 1.0,
                    });
                }
            }
        }
        prune_unused_buses(config);
    }
    // Les lignes s'approprient un câble libre et le renomment, pour exister
    // comme haut-parleurs dans Windows dès le premier lancement.
    auto_bind_lines(config);
    config.schema_version = CURRENT_SCHEMA_VERSION;
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
        let Some(cable) = cables
            .iter()
            .find(|c| !used.contains(c) && !cable_used_as_output(cfg, c))
        else {
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
            // Les ids MMDevice ne survivent pas à une réinstallation de
            // VB-Cable : revalider le cache avant de s'en servir, sinon on
            // route vers un endpoint fantôme (Windows accepte sans broncher).
            let render_id = cached
                .filter(|id| winapps::render_device_active(id))
                .or_else(|| winapps::render_id_for_capture(cable).ok());
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
///
/// Async (comme toutes les commandes qui font du COM ou de l'I/O bloquant) :
/// une commande synchrone s'exécute sur le thread principal tao, et une
/// énumération de sessions qui traîne (driver Bluetooth capricieux) y
/// gelait faders, tray et déplacement de fenêtre le temps du round-trip.
#[tauri::command]
async fn list_apps() -> Result<Vec<AppInfo>, String> {
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
async fn assign_app_to_line(
    line_id: String,
    exe: String,
    state: State<'_, AppState>,
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
                    .find(|c| !used.contains(&c) && !cable_used_as_output(&cfg, c))
                    .ok_or_else(|| {
                        "aucun câble virtuel libre — installe VB-Cable (et le pack A+B pour plus de lignes)"
                            .to_string()
                    })?
            }
        };
        (capture, line.cable_render_id.clone(), line.name.clone())
    };

    // -- 2. resolve the cable's render side (id cached after the first time;
    //       the pairing survives MixFlow's own endpoint renames, but NOT a
    //       reinstall of VB-Cable — hence the revalidation) ------------------
    let render_id = match cached_render_id {
        Some(id) if winapps::render_device_active(&id) => id,
        _ => winapps::render_id_for_capture(&capture_name)?,
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
async fn unassign_app_from_line(
    line_id: String,
    exe: String,
    state: State<'_, AppState>,
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
            duck_reactivity: "normale".into(),
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

/// Find the output bus already bound to `device`, or create one. Shared by
/// every command that resolves a device NAME into a bus id (profiles,
/// streamer mode) — `set_line_outputs` keeps its own inline copy since it
/// also needs the per-device gain-preservation pass around it.
fn find_or_create_bus(cfg: &mut AppConfig, device: &str) -> String {
    if let Some(o) = cfg.outputs.iter().find(|o| o.device == device) {
        return o.id.clone();
    }
    let id = new_id("out");
    let name = device.split(" (").next().unwrap_or(device).to_string();
    cfg.outputs.push(OutputConfig {
        id: id.clone(),
        name,
        device: device.to_string(),
        gain: 1.0,
        muted: false,
    });
    id
}

/// Apply a saved [`Profile`] onto the live config. Lines are matched by id
/// (a profile saved before a line was deleted just skips that entry);
/// outputs are resolved by device name, keeping a route's existing gain if
/// that device is already selected. Ducking rules referencing a line that
/// no longer exists are dropped rather than carried over as dangling refs.
fn apply_profile_to_config(cfg: &mut AppConfig, profile: &Profile) {
    for snap in &profile.lines {
        let existing_gain_by_device: HashMap<String, f32> = cfg
            .lines
            .iter()
            .find(|l| l.id == snap.line_id)
            .into_iter()
            .flat_map(|l| &l.routes)
            .filter_map(|r| {
                cfg.outputs
                    .iter()
                    .find(|o| o.id == r.output_id)
                    .map(|o| (o.device.clone(), r.gain))
            })
            .collect();
        let mut new_routes = Vec::with_capacity(snap.output_devices.len());
        for device in snap.output_devices.iter().filter(|d| !d.is_empty()) {
            let bus_id = find_or_create_bus(cfg, device);
            let gain = existing_gain_by_device.get(device).copied().unwrap_or(1.0);
            new_routes.push(Route {
                output_id: bus_id,
                gain,
            });
        }
        if let Some(line) = cfg.lines.iter_mut().find(|l| l.id == snap.line_id) {
            line.gain = snap.gain;
            line.muted = snap.muted;
            line.eq_bands = snap.eq_bands.clone();
            line.routes = new_routes;
        }
    }
    let line_ids: std::collections::HashSet<String> =
        cfg.lines.iter().map(|l| l.id.clone()).collect();
    cfg.ducking = profile
        .ducking
        .iter()
        .filter(|d| line_ids.contains(&d.source_line) && line_ids.contains(&d.target_line))
        .cloned()
        .collect();
    cfg.master_gain = profile.master_gain;
    prune_unused_buses(cfg);
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
///
/// Les règles sont filtrées ici et pas seulement dans l'UI : l'import et les
/// profils écrivent aussi `ducking`. Une règle source == cible fait qu'une
/// ligne s'atténue elle-même en fonction de son propre niveau (pompage), et
/// une règle référant une ligne absente n'aurait aucun effet utile.
#[tauri::command]
fn set_duck_rules(rules: Vec<DuckRule>, state: State<AppState>) {
    let sane = {
        let mut cfg = state.config.lock();
        let ids: std::collections::HashSet<String> =
            cfg.lines.iter().map(|l| l.id.clone()).collect();
        let sane: Vec<DuckRule> = rules
            .into_iter()
            .filter(|r| {
                r.source_line != r.target_line
                    && ids.contains(&r.source_line)
                    && ids.contains(&r.target_line)
            })
            .map(|mut r| {
                r.amount = finite_or(r.amount, 0.5).clamp(0.0, 1.0);
                r
            })
            .collect();
        cfg.ducking = sane.clone();
        sane
    };
    *state.controls.read().ducking.write() = sane;
    persist(&state);
}

/// "Démarrer avec Windows" — reads the actual current-user Run key state
/// rather than mirroring a config flag, so it can't drift from reality.
#[tauri::command]
fn get_autostart_enabled(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart_enabled(enabled: bool, app: AppHandle) -> Result<(), String> {
    let mgr = app.autolaunch();
    let res = if enabled { mgr.enable() } else { mgr.disable() };
    res.map_err(|e| e.to_string())
}

/// Global-shortcut handler (Ctrl+Alt+M): toggle every mic line's mute at
/// once. Live, atomics-only — same "no rebuild" contract as `set_line_muted`,
/// just fanned out to every mic and broadcast so the UI (which didn't
/// initiate this) picks up the new state.
fn toggle_all_mics(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let cfg_after = {
        let mut cfg = state.config.lock();
        let any_unmuted = cfg.lines.iter().any(|l| l.kind == "mic" && !l.muted);
        for line in cfg.lines.iter_mut().filter(|l| l.kind == "mic") {
            line.muted = any_unmuted;
        }
        cfg.clone()
    };
    {
        let controls = state.controls.read();
        for line in cfg_after.lines.iter().filter(|l| l.kind == "mic") {
            if let Some(ctl) = controls.lines.get(&line.id) {
                ctl.muted.store(line.muted, Ordering::Relaxed);
            }
        }
    }
    persist(&state);
    let _ = app.emit("config_updated", &cfg_after);
}

/// Per-route gain — the ceiling of item #2 (per-output gain) was already
/// live in the render callback (`input.route.gain`); this was the only
/// missing piece: a way to set it. Live, no rebuild.
#[tauri::command]
fn set_route_gain(line_id: String, output_id: String, gain: f32, state: State<AppState>) {
    let gain = gain.clamp(0.0, 1.5);
    {
        let mut cfg = state.config.lock();
        if let Some(line) = cfg.lines.iter_mut().find(|l| l.id == line_id) {
            if let Some(route) = line.routes.iter_mut().find(|r| r.output_id == output_id) {
                route.gain = gain;
            }
        }
    }
    if let Some(ctl) = state.controls.read().lines.get(&line_id) {
        if let Some(route_ctl) = ctl.routes.get(&output_id) {
            route_ctl.gain.set(gain);
        }
    }
    persist(&state);
}

/// Ducking "réactivité" — how fast this line's side-chain envelope reacts
/// when it's the SOURCE of a rule. Live: only the decay coefficient moves.
#[tauri::command]
fn set_line_duck_reactivity(id: String, level: String, state: State<AppState>) {
    let level = match level.as_str() {
        "douce" | "rapide" => level,
        _ => "normale".to_string(),
    };
    {
        let mut cfg = state.config.lock();
        if let Some(line) = cfg.lines.iter_mut().find(|l| l.id == id) {
            line.duck_reactivity = level.clone();
        }
    }
    if let Some(ctl) = state.controls.read().lines.get(&id) {
        ctl.duck_decay.set(reactivity_decay(&level));
    }
    persist(&state);
}

// ---------------------------------------------------------------------------
// Commands — export / import config
// ---------------------------------------------------------------------------

#[tauri::command]
async fn export_config(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let cfg = state.config.lock().clone();
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    let Some(path) = app
        .dialog()
        .file()
        .add_filter("Configuration MixFlow", &["json"])
        .set_file_name("mixflow-config.json")
        .blocking_save_file()
    else {
        return Ok(None); // user cancelled
    };
    let path_buf = path.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path_buf, json).map_err(|e| e.to_string())?;
    Ok(Some(path_buf.display().to_string()))
}

#[tauri::command]
async fn import_config(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<AppConfig>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(path) = app
        .dialog()
        .file()
        .add_filter("Configuration MixFlow", &["json"])
        .blocking_pick_file()
    else {
        return Ok(None); // user cancelled
    };
    let path_buf = path.into_path().map_err(|e| e.to_string())?;
    let json = std::fs::read_to_string(&path_buf).map_err(|e| e.to_string())?;

    // 1. Est-ce seulement une config MixFlow ? Tous les champs d'AppConfig
    //    sont `#[serde(default)]`, donc n'importe quel objet JSON (un
    //    package.json choisi par erreur dans le dialogue, qui ne filtre que
    //    sur .json) se désérialiserait en une config VIDE et effacerait tout.
    let value: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| format!("fichier illisible : {e}"))?;
    if !looks_like_mixflow_config(&value) {
        return Err(
            "ce fichier n'est pas une configuration MixFlow (il lui manque les sections \
             « lines » et « outputs »). Rien n'a été modifié."
                .into(),
        );
    }
    let mut imported: AppConfig =
        serde_json::from_value(value).map_err(|e| format!("fichier invalide : {e}"))?;
    // 2. Un JSON valide n'est pas pour autant sain : sans ce passage, un
    //    `"gain": 1e300` (→ +inf en f32) contaminait le lissage de gain du
    //    rendu puis l'état des biquads, laissant la ligne muette ou bruyante.
    sanitize_config(&mut imported);

    // 3. L'import est destructif et irréversible : on demande confirmation en
    //    annonçant ce qui arrive, et on garde une copie de l'existant.
    let confirmed = app
        .dialog()
        .message(format!(
            "La configuration actuelle sera remplacée par : {} canal/canaux, {} profil(s), \
             {} preset(s) d'égaliseur.\n\nUne sauvegarde de la configuration actuelle sera \
             conservée à côté du fichier de config.",
            imported.lines.len(),
            imported.profiles.len(),
            imported.eq_presets.len(),
        ))
        .title("Importer cette configuration ?")
        .kind(tauri_plugin_dialog::MessageDialogKind::Warning)
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancel)
        .blocking_show();
    if !confirmed {
        return Ok(None);
    }
    let backup = backup_config(&state.config_path);

    // 4. Windows garde un routage PERSISTÉ par application
    //    (SetPersistedDefaultAudioEndpoint). Les apps de l'ancienne config
    //    que la nouvelle ne reprend pas continueraient sinon de jouer dans un
    //    câble que plus aucune ligne ne capture : silence total, sans le
    //    moindre avertissement. On les rend à la sortie par défaut.
    let stale_apps: Vec<String> = {
        let cfg = state.config.lock();
        let kept: std::collections::HashSet<String> = imported
            .lines
            .iter()
            .flat_map(|l| l.apps.iter().map(|a| a.to_lowercase()))
            .collect();
        cfg.lines
            .iter()
            .flat_map(|l| l.apps.iter())
            .filter(|a| !kept.contains(&a.to_lowercase()))
            .cloned()
            .collect()
    };
    for exe in &stale_apps {
        let _ = winapps::unroute_app(exe);
    }

    // 5. Mêmes réconciliations qu'au démarrage (câbles, sorties, tranche
    //    micro) — la config vient peut-être d'une autre machine. Fait HORS
    //    du verrou : `reconcile_config` contient du COM bloquant.
    reconcile_config(&mut imported);

    let cfg_after = {
        let mut cfg = state.config.lock();
        *cfg = imported;
        cfg.clone()
    };
    rebuild(&state);
    if let Some(bak) = backup {
        eprintln!(
            "[mixflow] config précédente sauvegardée : {}",
            bak.display()
        );
    }
    Ok(Some(cfg_after))
}

// ---------------------------------------------------------------------------
// Commands — profiles (save/apply a full mix state, optionally auto-switch)
// ---------------------------------------------------------------------------

#[tauri::command]
fn save_profile(name: String, trigger_exe: Option<String>, state: State<AppState>) -> AppConfig {
    let cfg_after = {
        let mut cfg = state.config.lock();
        let lines: Vec<LineSnapshot> = cfg
            .lines
            .iter()
            .map(|l| LineSnapshot {
                line_id: l.id.clone(),
                gain: l.gain,
                muted: l.muted,
                eq_bands: l.eq_bands.clone(),
                // Un bus jamais assigné à un périphérique porte device "" (état
                // livré par défaut). Le photographier tel quel produisait un
                // snapshot [""] que `apply_profile_to_config` filtre ensuite —
                // la ligne se retrouvait alors SANS AUCUNE sortie, mix muet et
                // sans avertissement. On ne photographie que le concret.
                output_devices: l
                    .routes
                    .iter()
                    .filter_map(|r| {
                        cfg.outputs
                            .iter()
                            .find(|o| o.id == r.output_id)
                            .map(|o| o.device.clone())
                    })
                    .filter(|d| !d.is_empty())
                    .collect(),
            })
            .collect();
        let ducking = cfg.ducking.clone();
        let master_gain = cfg.master_gain;
        cfg.profiles.push(Profile {
            id: new_id("profile"),
            name: name.trim().to_string(),
            trigger_exe: trigger_exe.filter(|s| !s.trim().is_empty()),
            lines,
            ducking,
            master_gain,
        });
        cfg.clone()
    };
    persist(&state);
    cfg_after
}

#[tauri::command]
fn apply_profile(id: String, state: State<AppState>) -> Result<AppConfig, String> {
    let profile = {
        let cfg = state.config.lock();
        cfg.profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| "profil introuvable".to_string())?
    };
    let cfg_after = {
        let mut cfg = state.config.lock();
        apply_profile_to_config(&mut cfg, &profile);
        cfg.clone()
    };
    rebuild(&state);
    // Mémorisé dans l'état PARTAGÉ (pas dans une variable locale au thread
    // d'auto-switch) : sans ça, le thread ré-appliquait 2 s plus tard le
    // profil que l'utilisateur venait de poser à la main, écrasant ses
    // réglages sous ses yeux.
    *state.active_profile.lock() = Some(id);
    Ok(cfg_after)
}

#[tauri::command]
fn delete_profile(id: String, state: State<AppState>) -> AppConfig {
    let cfg_after = {
        let mut cfg = state.config.lock();
        cfg.profiles.retain(|p| p.id != id);
        cfg.clone()
    };
    persist(&state);
    cfg_after
}

#[tauri::command]
fn set_profile_trigger(
    id: String,
    trigger_exe: Option<String>,
    state: State<AppState>,
) -> AppConfig {
    let cfg_after = {
        let mut cfg = state.config.lock();
        if let Some(p) = cfg.profiles.iter_mut().find(|p| p.id == id) {
            p.trigger_exe = trigger_exe.filter(|s| !s.trim().is_empty());
        }
        cfg.clone()
    };
    persist(&state);
    cfg_after
}

/// Auto-switch: called every ~2 s from a dedicated thread (see `main`) with
/// the current foreground exe. Applies the first profile whose trigger
/// matches, if it isn't already the active one.
///
/// Le « profil actif » vit dans `AppState` et non dans le thread : la
/// commande `apply_profile` l'y écrit aussi, donc un profil appliqué à la
/// main n'est plus ré-appliqué (et les réglages faits ensuite plus
/// écrasés) au tick suivant.
fn maybe_apply_triggered_profile(app: &AppHandle, foreground_exe: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let target = {
        let cfg = state.config.lock();
        cfg.profiles
            .iter()
            .find(|p| {
                p.trigger_exe
                    .as_deref()
                    .map(|t| t.eq_ignore_ascii_case(foreground_exe))
                    .unwrap_or(false)
            })
            .map(|p| p.id.clone())
    };
    let Some(target_id) = target else {
        return;
    };
    if state.active_profile.lock().as_deref() == Some(target_id.as_str()) {
        return;
    }
    let profile = {
        let cfg = state.config.lock();
        cfg.profiles.iter().find(|p| p.id == target_id).cloned()
    };
    let Some(profile) = profile else {
        return;
    };
    let cfg_after = {
        let mut cfg = state.config.lock();
        apply_profile_to_config(&mut cfg, &profile);
        cfg.clone()
    };
    rebuild(&state);
    let _ = app.emit("config_updated", &cfg_after);
    *state.active_profile.lock() = Some(target_id);
}

// ---------------------------------------------------------------------------
// Commands — Mode Streamer
// ---------------------------------------------------------------------------

/// Adds `device` as an extra output on every line that doesn't already play
/// to it — a one-click way to send the whole mix to a dedicated "stream"
/// virtual cable, on top of each line's normal (personal-listening)
/// outputs. No engine changes needed: this is exactly the fan-out +
/// per-route-gain machinery every line already has, just applied in bulk.
#[tauri::command]
fn enable_streamer_mode(device: String, state: State<AppState>) -> AppConfig {
    let cfg_after = {
        let mut cfg = state.config.lock();
        let bus_id = find_or_create_bus(&mut cfg, &device);
        for line in &mut cfg.lines {
            if !line.routes.iter().any(|r| r.output_id == bus_id) {
                line.routes.push(Route {
                    output_id: bus_id.clone(),
                    gain: 1.0,
                });
            }
        }
        cfg.clone()
    };
    rebuild(&state);
    cfg_after
}

// ---------------------------------------------------------------------------
// Commands — update check
// ---------------------------------------------------------------------------

/// Deliberately NOT wired to `tauri-plugin-updater` yet: that plugin refuses
/// to initialize at all without a real signing pubkey + release endpoint in
/// `tauri.conf.json` (verified empirically — it panics the whole app on
/// startup otherwise), and this repo has neither (no GitHub remote/releases
/// configured — see CLAUDE.md). Once both exist, swap this stub for the
/// plugin's `updater().check()` call; the frontend contract (`Result<Option<
/// String>, String>`, `Some(version)` | `None` = up to date) is already
/// future-proofed for that.
#[tauri::command]
fn check_for_update() -> Result<Option<String>, String> {
    Err("mise à jour automatique non configurée pour ce build (nécessite un remote GitHub avec des releases publiées et une clé de signature)".into())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        toggle_all_mics(app);
                    }
                })
                .build(),
        )
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

            // Ctrl+Alt+M: toggle every mic's mute, even with the window
            // hidden in the tray. Best-effort — another app may already own
            // the combo; that's a `notice`-worthy situation, not fatal.
            let mic_mute_shortcut =
                Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyM);
            if let Err(e) = app.global_shortcut().register(mic_mute_shortcut) {
                eprintln!("[mixflow] raccourci Ctrl+Alt+M indisponible : {e}");
            }

            let config_path = app
                .path()
                .app_config_dir()
                .expect("no config dir on this platform")
                .join("mixflow.config.json");
            let mut config = load_or_default(&config_path);
            // Migrations de forme + garanties structurelles + appropriation
            // des câbles — partagées avec `import_config`.
            reconcile_config(&mut config);
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
                active_profile: Mutex::new(None),
            });

            // VU meter pump: 20 Hz "levels" event. Peaks are read-and-halved
            // so a stopped stream visibly falls back to zero.
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("mixflow-levels".into())
                .spawn(move || {
                    // id -> dernier `capture_tick` observé, pour repérer un
                    // flux de capture mort (voir plus bas).
                    let mut last_ticks: HashMap<String, u32> = HashMap::new();
                    loop {
                        std::thread::sleep(Duration::from_millis(50));
                        let state = handle.state::<AppState>();
                        let controls = state.controls.read().clone();
                        let mut lines = HashMap::new();
                        for (id, ctl) in &controls.lines {
                            let p = ctl.peak.get();
                            ctl.peak.set(p * 0.5);
                            lines.insert(id.clone(), p);
                            // `env` (side-chain du ducking) n'est décrémentée que
                            // par le callback de capture. Si celui-ci s'arrête
                            // alors que la ligne parlait, l'enveloppe reste figée
                            // au-dessus du seuil et ses cibles restent atténuées
                            // pour toujours. Un compteur qui stagne = flux mort :
                            // on relâche l'enveloppe ici, hors du chemin temps
                            // réel. Le lissage de gain du rendu (~10 ms) évite
                            // tout clic au retour à la normale.
                            let tick = ctl.capture_tick.load(Ordering::Relaxed);
                            let stalled = last_ticks.insert(id.clone(), tick) == Some(tick);
                            if stalled {
                                let env = ctl.env.get();
                                if env > 0.0 {
                                    ctl.env.set(if env < 1e-4 { 0.0 } else { env * 0.5 });
                                }
                            }
                        }
                        let mut outputs = HashMap::new();
                        for (id, ctl) in &controls.outputs {
                            let p = ctl.peak.get();
                            ctl.peak.set(p * 0.5);
                            outputs.insert(id.clone(), p);
                        }
                        let _ = handle.emit("levels", LevelsPayload { lines, outputs });
                        // Les lignes disparues (rebuild de topologie) ne doivent
                        // pas faire grossir la table indéfiniment.
                        last_ticks.retain(|id, _| controls.lines.contains_key(id));
                    }
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

            // Device-health watch: a stream that errors mid-session (device
            // unplugged) only logs to stderr today (see `stream_err` in
            // engine.rs) — the UI never finds out. Polling the live device
            // list against what's actually configured surfaces that as an
            // explicit warning instead of the mix just going silent.
            let handle3 = app.handle().clone();
            std::thread::Builder::new()
                .name("mixflow-health".into())
                .spawn(move || {
                    let mut last_warnings: Vec<String> = Vec::new();
                    loop {
                        std::thread::sleep(Duration::from_secs(3));
                        let state = handle3.state::<AppState>();
                        let cfg = state.config.lock().clone();
                        let host = cpal::default_host();
                        let live_inputs: std::collections::HashSet<String> = host
                            .input_devices()
                            .map(|it| it.filter_map(|d| d.name().ok()).collect())
                            .unwrap_or_default();
                        let live_outputs: std::collections::HashSet<String> = host
                            .output_devices()
                            .map(|it| it.filter_map(|d| d.name().ok()).collect())
                            .unwrap_or_default();
                        let mut warnings = Vec::new();
                        for line in &cfg.lines {
                            if let Some(d) = &line.input_device {
                                if !live_inputs.contains(d) {
                                    warnings.push(format!(
                                        "« {} » : périphérique d'entrée débranché (\"{d}\")",
                                        line.name
                                    ));
                                }
                            }
                        }
                        for out in &cfg.outputs {
                            if !out.device.is_empty() && !live_outputs.contains(&out.device) {
                                warnings.push(format!(
                                    "« {} » : périphérique de sortie débranché (\"{}\")",
                                    out.name, out.device
                                ));
                            }
                        }
                        if warnings != last_warnings {
                            let _ = handle3.emit("device_warnings", &warnings);
                            last_warnings = warnings;
                        }
                    }
                })
                .expect("failed to spawn health thread");

            // Profile auto-switch: which app has focus, checked every 2 s
            // against each profile's trigger_exe.
            let handle4 = app.handle().clone();
            std::thread::Builder::new()
                .name("mixflow-profiles".into())
                .spawn(move || loop {
                    std::thread::sleep(Duration::from_secs(2));
                    if let Some(fg) = winapps::foreground_exe() {
                        maybe_apply_triggered_profile(&handle4, &fg);
                    }
                })
                .expect("failed to spawn profile-watch thread");

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
            set_duck_rules,
            get_autostart_enabled,
            set_autostart_enabled,
            set_route_gain,
            set_line_duck_reactivity,
            export_config,
            import_config,
            save_profile,
            apply_profile,
            delete_profile,
            set_profile_trigger,
            enable_streamer_mode,
            check_for_update,
            take_startup_notice
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

    /// Le cœur du garde-fou d'import : une valeur non finie ne doit JAMAIS
    /// atteindre le moteur (un +inf contamine le lissage de gain puis l'état
    /// des biquads, et la ligne reste muette ou bruyante).
    #[test]
    fn sanitize_config_rejects_non_finite_and_out_of_range_values() {
        let mut cfg = default_config();
        let line_id = cfg.lines[0].id.clone();
        cfg.master_gain = f32::INFINITY;
        {
            let line = &mut cfg.lines[0];
            line.gain = 1e30; // devient +inf côté f32 après un aller-retour JSON
            line.eq_bands = vec![band(1_000.0, f32::NAN), band(2_000.0, 99.0)];
            line.duck_reactivity = "n'importe quoi".into();
        }
        // Route vers un bus inexistant + doublon vers un bus valide.
        let bus = cfg.outputs[0].id.clone();
        cfg.lines[0].routes = vec![
            Route {
                output_id: "bus-fantome".into(),
                gain: f32::NEG_INFINITY,
            },
            Route {
                output_id: bus.clone(),
                gain: 1.0,
            },
            Route {
                output_id: bus.clone(),
                gain: 1.0,
            },
        ];
        // Règle de ducking pointant une ligne supprimée.
        cfg.ducking.push(DuckRule {
            source_line: "ligne-fantome".into(),
            target_line: line_id.clone(),
            amount: 42.0,
        });
        // Règle auto-référente : la ligne s'atténuerait elle-même.
        cfg.ducking.push(DuckRule {
            source_line: line_id.clone(),
            target_line: line_id.clone(),
            amount: 0.5,
        });

        sanitize_config(&mut cfg);

        assert!(cfg.master_gain.is_finite() && (0.0..=1.0).contains(&cfg.master_gain));
        let line = cfg.lines.iter().find(|l| l.id == line_id).unwrap();
        assert!(line.gain.is_finite() && (0.0..=1.5).contains(&line.gain));
        assert!(line
            .eq_bands
            .iter()
            .all(|b| b.freq.is_finite() && b.gain.is_finite() && (-12.0..=12.0).contains(&b.gain)));
        assert_eq!(line.duck_reactivity, "normale");
        // Bus fantôme retiré, doublon dédoublonné : une seule route valide.
        assert_eq!(line.routes.len(), 1);
        assert_eq!(line.routes[0].output_id, bus);
        assert!(line.routes.iter().all(|r| r.gain.is_finite()));
        // Règles orpheline ET auto-référente écartées, rescapées bornées.
        assert!(cfg.ducking.iter().all(|d| d.source_line != "ligne-fantome"));
        assert!(cfg.ducking.iter().all(|d| d.source_line != d.target_line));
        assert!(cfg.ducking.iter().all(|d| (0.0..=1.0).contains(&d.amount)));
    }

    /// Le point clé de la sécurité de l'import : tous les champs d'AppConfig
    /// étant `#[serde(default)]`, n'importe quel objet JSON se désérialise
    /// sans erreur en une config VIDE. Sans ce filtre de forme, choisir un
    /// mauvais .json dans le dialogue effaçait toute la configuration.
    #[test]
    fn import_rejects_json_that_is_not_a_mixflow_config() {
        // Ceux-ci sont le vrai danger : serde les accepte SANS ERREUR et en
        // fait une config vide — c'est exactement ce qui effaçait tout.
        let silencieusement_acceptes_par_serde = [
            r#"{}"#,
            r#"{"name":"mixflow","version":"0.1.0","scripts":{"dev":"vite"}}"#, // package.json
            r#"{"compilerOptions":{"strict":true}}"#,                           // tsconfig.json
            r#"{"lines":[]}"#, // « outputs » manquant
        ];
        for src in silencieusement_acceptes_par_serde {
            let v: serde_json::Value = serde_json::from_str(src).unwrap();
            assert!(
                serde_json::from_value::<AppConfig>(v.clone()).is_ok(),
                "serde aurait dû l'accepter (c'est le piège) : {src}"
            );
            assert!(
                !looks_like_mixflow_config(&v),
                "aurait dû être refusé par le filtre de forme : {src}"
            );
        }

        // Ceux-là, serde les rejette déjà ; le filtre les refuse aussi.
        for src in [
            r#"[]"#,
            r#""juste une chaîne""#,
            r#"{"lines":"pas un tableau","outputs":[]}"#,
        ] {
            let v: serde_json::Value = serde_json::from_str(src).unwrap();
            assert!(
                !looks_like_mixflow_config(&v),
                "aurait dû être refusé : {src}"
            );
        }

        // Une vraie config passe, y compris vidée de ses lignes.
        let real = serde_json::to_value(default_config()).unwrap();
        assert!(looks_like_mixflow_config(&real));
        let emptied: serde_json::Value =
            serde_json::from_str(r#"{"lines":[],"outputs":[]}"#).unwrap();
        assert!(looks_like_mixflow_config(&emptied));
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

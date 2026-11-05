//! MixFlow sous Linux — mélangeur par application au-dessus de PipeWire.
//!
//! ```text
//! appli ──► sink virtuel « MixFlow · <appli> »  (mixflow.<slug>)
//!             └─ 10 × bq_peaking (EQ) ─► linear (volume/mute)
//!                  └─ flux mixflow.<slug>.out ─► sortie choisie (HDMI, casque, BT…)
//! ```
//!
//! - **Chaînes** : un service systemd utilisateur par appli (`chain.rs`),
//!   donc le son reste traité quand la fenêtre est fermée.
//! - **Graphe** : `pw-dump --monitor` tenu à jour dans un thread (`pw.rs`).
//! - **Routage** : la métadonnée `target.object` (celle que WirePlumber
//!   mémorise par appli) envoie chaque flux d'une appli configurée dans son
//!   sink, et chaque chaîne vers la sortie choisie. WirePlumber s'en
//!   souvient : une appli relancée retourne dans sa chaîne même si MixFlow
//!   n'est pas ouvert.
//! - **Réglages live** (volume, mute, EQ) : `pw-cli set-param`, coalescés
//!   par un thread dédié ; la conf de la chaîne est réécrite en différé
//!   pour survivre au prochain démarrage.

mod chain;
mod icons;
mod model;
mod pw;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use chain::{playback_name, sink_name, Chains};
use model::{sanitize_bands, sanitize_gain, AppSettings, EqBand, EqPreset, LinuxConfig};
use pw::Graph;

struct Inner {
    config: Mutex<LinuxConfig>,
    graph: Mutex<Graph>,
    chains: Chains,
    config_path: PathBuf,
    /// Clés d'applis dont les paramètres live doivent être poussés.
    pending_apply: Mutex<HashSet<String>>,
    /// Config à écrire sur disque (débouncé).
    dirty_save: AtomicBool,
    /// Le graphe ou la config a changé : re-router et notifier l'UI.
    graph_dirty: AtomicBool,
    /// Consignes `target.object` déjà envoyées : id de flux → valeur.
    routed: Mutex<HashMap<u32, String>>,
    /// Dernier sink sur lequel les paramètres de chaque appli ont été
    /// poussés : un sink qui (ré)apparaît reçoit l'état courant.
    pushed_sink: Mutex<HashMap<String, u32>>,
    notice: Mutex<Option<String>>,
}

type Shared = Arc<Inner>;

#[derive(Serialize, Clone)]
struct AppView {
    key: String,
    name: String,
    icon: Option<String>,
    color: String,
    /// L'appli a des réglages MixFlow (et donc une chaîne).
    configured: bool,
    /// Au moins un flux en cours de lecture.
    playing: bool,
    /// Nombre de flux audio ouverts par l'appli.
    streams: usize,
    /// Sa chaîne existe dans PipeWire (sinon : en cours de démarrage).
    chain_ready: bool,
    gain: f32,
    muted: bool,
    output: String,
    eq_bands: Vec<EqBand>,
}

#[derive(Serialize, Clone)]
struct OutputView {
    name: String,
    label: String,
    bluetooth: bool,
}

#[derive(Serialize, Clone)]
struct LinuxState {
    apps: Vec<AppView>,
    outputs: Vec<OutputView>,
    /// `node.name` de la sortie par défaut du système.
    default_output: String,
    eq_presets: Vec<EqPreset>,
    pipewire_ok: bool,
    notice: Option<String>,
}

const OWN_BINARIES: &[&str] = &["mixflow", "WebKitWebProcess", "WebKitNetworkProcess"];

fn set_notice(inner: &Inner, msg: impl Into<String>) {
    let msg = msg.into();
    eprintln!("[mixflow] {msg}");
    *inner.notice.lock() = Some(msg);
    inner.graph_dirty.store(true, Ordering::Relaxed);
}

// -- persistance --------------------------------------------------------------

fn load_config(path: &Path) -> (LinuxConfig, Option<String>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (LinuxConfig::default(), None);
    };
    match serde_json::from_str::<LinuxConfig>(&text) {
        Ok(mut c) => {
            c.sanitize();
            (c, None)
        }
        Err(e) => {
            let bak = path.with_extension("json.illisible");
            let _ = std::fs::rename(path, &bak);
            (
                LinuxConfig::default(),
                Some(format!(
                    "Réglages illisibles ({e}) : mis de côté dans {} et remplacés par des réglages neufs.",
                    bak.display()
                )),
            )
        }
    }
}

fn save_config(inner: &Inner) {
    let cfg = inner.config.lock().clone();
    let json = match serde_json::to_string_pretty(&cfg) {
        Ok(j) => j,
        Err(e) => return set_notice(inner, format!("Sauvegarde impossible : {e}")),
    };
    if let Some(dir) = inner.config_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = inner.config_path.with_extension("json.tmp");
    let res = std::fs::write(&tmp, json).and_then(|_| std::fs::rename(&tmp, &inner.config_path));
    if let Err(e) = res {
        set_notice(inner, format!("Sauvegarde impossible : {e}"));
    }
    for app in &cfg.apps {
        if let Err(e) = inner.chains.write_conf(app) {
            set_notice(inner, format!("Conf de la chaîne {} : {e}", app.name));
        }
    }
}

fn mark_changed(inner: &Inner, key: &str) {
    inner.pending_apply.lock().insert(key.to_lowercase());
    inner.dirty_save.store(true, Ordering::Relaxed);
}

// -- état pour l'UI -------------------------------------------------------------

fn build_state(inner: &Inner) -> LinuxState {
    let graph = inner.graph.lock().clone();
    let cfg = inner.config.lock().clone();

    // Flux d'applis regroupés par clé (insensible à la casse).
    let mut streams: HashMap<String, Vec<&pw::Node>> = HashMap::new();
    for n in graph.nodes.values().filter(|n| n.is_app_playback()) {
        if let Some(k) = n.app_key() {
            if OWN_BINARIES.contains(&k.as_str()) {
                continue;
            }
            streams.entry(k.to_lowercase()).or_default().push(n);
        }
    }

    let mut apps: Vec<AppView> = Vec::new();
    for a in &cfg.apps {
        let s = streams.remove(&a.key.to_lowercase()).unwrap_or_default();
        let icon_name = s
            .iter()
            .map(|n| n.icon_name.as_str())
            .find(|i| !i.is_empty());
        let id = icons::identify(&a.key, &a.name, icon_name.unwrap_or(""));
        apps.push(AppView {
            key: a.key.clone(),
            name: a.name.clone(),
            icon: id.icon,
            color: a.color.clone(),
            configured: true,
            playing: s.iter().any(|n| n.state == "running"),
            streams: s.len(),
            chain_ready: graph.node_by_name(&sink_name(&a.slug)).is_some(),
            gain: a.gain,
            muted: a.muted,
            output: a.output.clone(),
            eq_bands: a.eq_bands.clone(),
        });
    }
    for (_, s) in streams {
        let first = s[0];
        let key = first.app_key().unwrap_or_default();
        let icon_name = s
            .iter()
            .map(|n| n.icon_name.as_str())
            .find(|i| !i.is_empty());
        let id = icons::identify(&key, &first.app_name, icon_name.unwrap_or(""));
        let name = id.name.unwrap_or_else(|| {
            if first.app_name.is_empty() {
                key.clone()
            } else {
                first.app_name.clone()
            }
        });
        apps.push(AppView {
            key,
            name,
            icon: id.icon,
            color: "#8b8b9a".into(),
            configured: false,
            playing: s.iter().any(|n| n.state == "running"),
            streams: s.len(),
            chain_ready: false,
            gain: 1.0,
            muted: false,
            output: String::new(),
            eq_bands: model::default_bands(),
        });
    }
    // Applis qui jouent d'abord, puis celles qui ont un flux ouvert, puis
    // les autres ; ordre alphabétique à l'intérieur.
    apps.sort_by_key(|a| (!a.playing, a.streams == 0, a.name.to_lowercase()));

    let mut outputs: Vec<OutputView> = graph
        .nodes
        .values()
        .filter(|n| n.is_hw_sink())
        .map(|n| OutputView {
            name: n.name.clone(),
            label: if n.description.is_empty() {
                n.name.clone()
            } else {
                n.description.clone()
            },
            bluetooth: n.api == "bluez5" || n.name.starts_with("bluez_output"),
        })
        .collect();
    outputs.sort_by(|a, b| a.label.cmp(&b.label));

    LinuxState {
        apps,
        outputs,
        default_output: graph.default_sink.clone(),
        eq_presets: cfg.eq_presets.clone(),
        pipewire_ok: graph.ready,
        notice: inner.notice.lock().clone(),
    }
}

// -- routage ----------------------------------------------------------------------

/// Envoie chaque flux d'appli configurée dans sa chaîne, et chaque chaîne
/// vers la sortie choisie. Idempotent : n'agit que sur les écarts.
fn reconcile(inner: &Inner) {
    let graph = inner.graph.lock().clone();
    let cfg = inner.config.lock().clone();
    let mut routed = inner.routed.lock();
    routed.retain(|id, _| graph.nodes.contains_key(id));

    let mut ask = |id: u32, value: Option<&str>, routed: &mut HashMap<u32, String>| {
        let v = value.unwrap_or("-1").to_string();
        let current = graph.targets.get(&id).map(String::as_str).unwrap_or("");
        if current == v || routed.get(&id) == Some(&v) {
            return;
        }
        match pw::set_target(id, value) {
            Ok(()) => {
                routed.insert(id, v);
            }
            Err(e) => eprintln!("[mixflow] routage du flux {id} : {e}"),
        }
    };

    for a in &cfg.apps {
        let Some(sink) = graph.node_by_name(&sink_name(&a.slug)) else {
            continue;
        };
        // Un sink (ré)apparu reçoit l'état courant des réglages.
        {
            let mut pushed = inner.pushed_sink.lock();
            if pushed.get(&a.slug) != Some(&sink.id) {
                pushed.insert(a.slug.clone(), sink.id);
                inner.pending_apply.lock().insert(a.key.to_lowercase());
            }
        }
        // Flux de l'appli → sa chaîne.
        for s in graph.nodes.values().filter(|n| n.is_app_playback()) {
            if s.app_key().is_some_and(|k| k.eq_ignore_ascii_case(&a.key)) {
                ask(s.id, Some(&sink.serial), &mut routed);
            }
        }
        // Chaîne → sortie choisie (ou défaut du système).
        if let Some(out) = graph.node_by_name(&playback_name(&a.slug)) {
            if a.output.is_empty() {
                ask(out.id, None, &mut routed);
            } else if let Some(hw) = graph.node_by_name(&a.output) {
                ask(out.id, Some(&hw.serial), &mut routed);
            }
            // Sortie absente (casque BT éteint…) : WirePlumber se rabat sur
            // la sortie par défaut ; on re-cible dès qu'elle revient.
        }
    }
}

// -- threads -------------------------------------------------------------------------

fn spawn_threads(app: AppHandle, inner: Shared) {
    // 1. Moniteur du graphe PipeWire (relancé si pw-dump s'arrête).
    let i = inner.clone();
    std::thread::Builder::new()
        .name("mixflow-pw".into())
        .spawn(move || loop {
            let mut first = true;
            let res = pw::monitor(|batch| {
                let mut g = i.graph.lock();
                if first {
                    *g = Graph::default();
                    first = false;
                }
                g.apply(&batch);
                g.ready = true;
                drop(g);
                i.graph_dirty.store(true, Ordering::Relaxed);
            });
            if let Err(e) = res {
                set_notice(&i, format!("PipeWire inaccessible : {e}"));
            }
            i.graph.lock().ready = false;
            i.graph_dirty.store(true, Ordering::Relaxed);
            std::thread::sleep(Duration::from_secs(2));
        })
        .expect("thread mixflow-pw");

    // 2. Routage + notification de l'UI (au plus 4×/s).
    let i = inner.clone();
    std::thread::Builder::new()
        .name("mixflow-route".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(250));
            if i.graph_dirty.swap(false, Ordering::Relaxed) {
                reconcile(&i);
                let _ = app.emit("lx_state", build_state(&i));
            }
        })
        .expect("thread mixflow-route");

    // 3. Paramètres live (coalescés) + sauvegarde différée.
    let i = inner;
    std::thread::Builder::new()
        .name("mixflow-apply".into())
        .spawn(move || {
            let mut last_change = Instant::now();
            loop {
                std::thread::sleep(Duration::from_millis(40));
                let keys: Vec<String> = i.pending_apply.lock().drain().collect();
                for k in keys {
                    let Some(app) = i.config.lock().find(&k).cloned() else {
                        continue;
                    };
                    let sink = i
                        .graph
                        .lock()
                        .node_by_name(&sink_name(&app.slug))
                        .map(|n| n.id);
                    if let Some(id) = sink {
                        if let Err(e) = pw::set_props(id, &chain::props_pod(&app)) {
                            eprintln!("[mixflow] réglages de {} : {e}", app.name);
                        }
                    }
                    last_change = Instant::now();
                }
                if i.dirty_save.load(Ordering::Relaxed)
                    && last_change.elapsed() > Duration::from_millis(700)
                {
                    i.dirty_save.store(false, Ordering::Relaxed);
                    save_config(&i);
                }
            }
        })
        .expect("thread mixflow-apply");
}

// -- commandes ------------------------------------------------------------------------

/// Garantit qu'une appli a des réglages et une chaîne active.
fn ensure_app(inner: &Inner, key: &str) -> Result<(), String> {
    if inner.config.lock().find(key).is_some() {
        return Ok(());
    }
    let name = build_state(inner)
        .apps
        .into_iter()
        .find(|a| a.key.eq_ignore_ascii_case(key))
        .map(|a| a.name)
        .unwrap_or_else(|| key.to_string());
    let app = {
        let mut cfg = inner.config.lock();
        let idx = cfg.add_app(key, &name);
        cfg.apps[idx].clone()
    };
    save_config(inner);
    inner.chains.ensure_unit()?;
    inner.chains.start(&app)?;
    inner.graph_dirty.store(true, Ordering::Relaxed);
    Ok(())
}

fn with_app(inner: &Inner, key: &str, f: impl FnOnce(&mut AppSettings)) -> Result<(), String> {
    ensure_app(inner, key)?;
    let mut cfg = inner.config.lock();
    let app = cfg.find_mut(key).ok_or("appli inconnue")?;
    f(app);
    drop(cfg);
    mark_changed(inner, key);
    Ok(())
}

#[tauri::command]
async fn lx_get_state(state: State<'_, Shared>) -> Result<LinuxState, String> {
    Ok(build_state(&state))
}

#[tauri::command]
async fn lx_set_gain(key: String, gain: f32, state: State<'_, Shared>) -> Result<(), String> {
    with_app(&state, &key, |a| a.gain = sanitize_gain(gain))
}

#[tauri::command]
async fn lx_set_muted(key: String, muted: bool, state: State<'_, Shared>) -> Result<(), String> {
    with_app(&state, &key, |a| a.muted = muted)
}

#[tauri::command]
async fn lx_set_eq(
    key: String,
    bands: Vec<EqBand>,
    state: State<'_, Shared>,
) -> Result<(), String> {
    with_app(&state, &key, |a| a.eq_bands = sanitize_bands(bands))
}

#[tauri::command]
async fn lx_set_output(
    key: String,
    output: String,
    state: State<'_, Shared>,
) -> Result<LinuxState, String> {
    with_app(&state, &key, |a| a.output = output)?;
    state.graph_dirty.store(true, Ordering::Relaxed);
    reconcile(&state);
    Ok(build_state(&state))
}

/// Oublie une appli : supprime sa chaîne ; ses flux retournent sur la
/// sortie par défaut.
#[tauri::command]
async fn lx_forget_app(key: String, state: State<'_, Shared>) -> Result<LinuxState, String> {
    let removed = {
        let mut cfg = state.config.lock();
        let pos = cfg
            .apps
            .iter()
            .position(|a| a.key.eq_ignore_ascii_case(&key));
        pos.map(|p| cfg.apps.remove(p))
    };
    if let Some(app) = removed {
        let streams: Vec<u32> = state
            .graph
            .lock()
            .nodes
            .values()
            .filter(|n| {
                n.is_app_playback() && n.app_key().is_some_and(|k| k.eq_ignore_ascii_case(&key))
            })
            .map(|n| n.id)
            .collect();
        for id in streams {
            let _ = pw::clear_target(id);
            state.routed.lock().remove(&id);
        }
        state.pushed_sink.lock().remove(&app.slug);
        if let Err(e) = state.chains.remove(&app.slug) {
            set_notice(&state, e);
        }
        save_config(&state);
    }
    state.graph_dirty.store(true, Ordering::Relaxed);
    Ok(build_state(&state))
}

#[tauri::command]
async fn lx_save_preset(
    name: String,
    bands: Vec<EqBand>,
    state: State<'_, Shared>,
) -> Result<LinuxState, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Donne un nom au préréglage.".into());
    }
    {
        let mut cfg = state.config.lock();
        cfg.eq_presets.retain(|p| p.name != name);
        cfg.eq_presets.push(EqPreset {
            name,
            bands: sanitize_bands(bands),
        });
    }
    save_config(&state);
    Ok(build_state(&state))
}

#[tauri::command]
async fn lx_delete_preset(name: String, state: State<'_, Shared>) -> Result<LinuxState, String> {
    state.config.lock().eq_presets.retain(|p| p.name != name);
    save_config(&state);
    Ok(build_state(&state))
}

#[tauri::command]
async fn lx_dismiss_notice(state: State<'_, Shared>) -> Result<(), String> {
    *state.notice.lock() = None;
    Ok(())
}

// -- point d'entrée ------------------------------------------------------------------

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let config_dir = app
                .path()
                .app_config_dir()
                .expect("pas de dossier de configuration");
            let config_path = config_dir.join("mixflow-linux.json");
            let (config, notice) = load_config(&config_path);
            let inner: Shared = Arc::new(Inner {
                config: Mutex::new(config),
                graph: Mutex::new(Graph::default()),
                chains: Chains::new(&config_dir),
                config_path,
                pending_apply: Mutex::new(HashSet::new()),
                dirty_save: AtomicBool::new(false),
                graph_dirty: AtomicBool::new(true),
                routed: Mutex::new(HashMap::new()),
                pushed_sink: Mutex::new(HashMap::new()),
                notice: Mutex::new(notice),
            });
            app.manage(inner.clone());

            // Les chaînes des applis déjà configurées (idempotent : ne fait
            // rien si systemd les a déjà lancées avec la session).
            let i = inner.clone();
            std::thread::spawn(move || {
                save_config(&i);
                if let Err(e) = i.chains.ensure_unit() {
                    return set_notice(&i, e);
                }
                let apps = i.config.lock().apps.clone();
                for a in apps {
                    if let Err(e) = i.chains.start(&a) {
                        set_notice(&i, format!("Chaîne de {} : {e}", a.name));
                    }
                }
            });

            spawn_threads(app.handle().clone(), inner);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            lx_get_state,
            lx_set_gain,
            lx_set_muted,
            lx_set_eq,
            lx_set_output,
            lx_forget_app,
            lx_save_preset,
            lx_delete_preset,
            lx_dismiss_notice,
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de MixFlow");
}

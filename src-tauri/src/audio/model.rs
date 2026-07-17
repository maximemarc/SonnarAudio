//! Configuration model — serialized as JSON (persistence) and mirrored in the
//! TypeScript types of the frontend (`src/types.ts`).

use serde::{Deserialize, Serialize};

/// A route: "this virtual line feeds that output bus, at this gain".
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Route {
    /// Id of the target [`OutputConfig`].
    pub output_id: String,
    /// Per-route gain, linear [0.0 .. 1.5].
    #[serde(default = "default_gain")]
    pub gain: f32,
}

/// A virtual line (a "channel" in Sonar terms: Game, Chat, Media, Mic...).
///
/// A line captures audio from ONE input endpoint — either a physical mic or,
/// for application audio, the capture side of a virtual cable (e.g. VB-Cable's
/// "CABLE Output"). It can then be routed to any number of output buses.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LineConfig {
    pub id: String,
    pub name: String,
    /// "app" (fed by application audio through a virtual cable) or "mic"
    /// (fed by a physical capture device). Drives grouping in the UI and
    /// which lines the cable auto-binding touches.
    #[serde(default = "default_kind")]
    pub kind: String,
    /// Accent color used by the UI (hex).
    #[serde(default = "default_color")]
    pub color: String,
    /// cpal device name of the capture endpoint. `None` = line is dormant.
    #[serde(default)]
    pub input_device: Option<String>,
    /// Line fader, linear [0.0 .. 1.5].
    #[serde(default = "default_gain")]
    pub gain: f32,
    #[serde(default)]
    pub muted: bool,
    /// LEGACY 5-band gains (pre-parametric configs) — migrated into
    /// `eq_bands` at startup, then ignored.
    #[serde(default)]
    pub eq: [f32; 5],
    /// Parametric EQ: 1..=10 peaking bands, each with its own frequency.
    /// The UI adds/moves/removes points directly on the response curve.
    #[serde(default)]
    pub eq_bands: Vec<EqBandCfg>,
    /// Executables (e.g. "Spotify.exe") whose audio Windows routes into this
    /// line's virtual cable — set by drag-and-drop in the UI.
    #[serde(default)]
    pub apps: Vec<String>,
    /// MMDevice id of the cable's RENDER side (what apps play into). Cached
    /// because MixFlow renames that endpoint after the line ("Game (VB-Audio
    /// Virtual Cable)"), which invalidates name-based lookup.
    #[serde(default)]
    pub cable_render_id: Option<String>,
    #[serde(default)]
    pub routes: Vec<Route>,
    /// How fast this line's ducking side-chain reacts when it is the
    /// SOURCE of a rule: "douce" | "normale" | "rapide". Governs the
    /// envelope-follower decay in the capture callback.
    #[serde(default = "default_reactivity")]
    pub duck_reactivity: String,
}

/// An output bus bound to a physical render device (headphones, speakers...).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutputConfig {
    pub id: String,
    pub name: String,
    /// cpal device name of the render endpoint. Empty = unassigned (dormant).
    #[serde(default)]
    pub device: String,
    /// Master fader of the bus, linear [0.0 .. 1.5].
    #[serde(default = "default_gain")]
    pub gain: f32,
    #[serde(default)]
    pub muted: bool,
}

/// Sidechain ducking rule — the "prioritization" feature.
///
/// While `source_line` is active (signal above the noise floor), every route of
/// `target_line` is attenuated by up to `amount` (0.0 = off, 1.0 = full duck).
/// Typical Sonar-like use: source = Chat, target = Game, amount = 0.5.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DuckRule {
    pub source_line: String,
    pub target_line: String,
    pub amount: f32,
}

/// One parametric EQ band: peaking biquad at `freq` Hz, `gain` dB (±12).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EqBandCfg {
    pub freq: f32,
    pub gain: f32,
}

/// A saved EQ preset (user-created; built-ins live in the frontend).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EqPreset {
    pub name: String,
    /// LEGACY fixed-band gains — migrated into `bands` at startup.
    #[serde(default)]
    pub gains: Vec<f32>,
    #[serde(default)]
    pub bands: Vec<EqBandCfg>,
}

/// One line's settings captured into a [`Profile`] — everything a profile
/// can restore, keyed by the line's (stable) id. Output devices are stored
/// by NAME rather than bus id: buses are recreated on demand (see
/// `set_line_outputs`), so a name survives the bus being pruned/recreated
/// between when the profile was saved and when it's applied.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LineSnapshot {
    pub line_id: String,
    pub gain: f32,
    pub muted: bool,
    pub eq_bands: Vec<EqBandCfg>,
    pub output_devices: Vec<String>,
}

/// A saved mix state, optionally auto-applied when `trigger_exe` becomes the
/// foreground app (e.g. a game launcher). Lines that no longer exist when
/// the profile is applied are skipped rather than erroring — profiles are a
/// convenience snapshot, not a strict topology contract.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub trigger_exe: Option<String>,
    #[serde(default)]
    pub lines: Vec<LineSnapshot>,
    #[serde(default)]
    pub ducking: Vec<DuckRule>,
    #[serde(default = "default_gain")]
    pub master_gain: f32,
}

/// Bump when a migration needs to distinguish "field was never set" from
/// "field was intentionally cleared" — the shape-based checks in `main.rs`'s
/// startup migrations (`eq_bands.is_empty()`, etc.) are self-guarding and
/// don't currently need this, but a future migration that must run exactly
/// once regardless of data shape can gate on `schema_version < N`.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub lines: Vec<LineConfig>,
    #[serde(default)]
    pub outputs: Vec<OutputConfig>,
    #[serde(default)]
    pub ducking: Vec<DuckRule>,
    /// Global MASTER: multiplies every output bus — the ceiling of the whole
    /// mix, linear [0.0 .. 1.0].
    #[serde(default = "default_gain")]
    pub master_gain: f32,
    /// User-saved EQ presets.
    #[serde(default)]
    pub eq_presets: Vec<EqPreset>,
    /// User-saved mix profiles (see [`Profile`]).
    #[serde(default)]
    pub profiles: Vec<Profile>,
    /// Absent/0 on any config saved before this field existed. Set to
    /// `CURRENT_SCHEMA_VERSION` once startup migrations have run.
    #[serde(default)]
    pub schema_version: u32,
}

fn default_gain() -> f32 {
    1.0
}
fn default_kind() -> String {
    "app".into()
}
fn default_color() -> String {
    "#8b5cf6".into()
}
fn default_reactivity() -> String {
    "normale".into()
}

/// Envelope-decay coefficient (per audio block) for a ducking reactivity
/// level. Lower = faster release (the side-chain gate lets go sooner).
pub fn reactivity_decay(level: &str) -> f32 {
    match level {
        "rapide" => 0.75,
        "douce" => 0.96,
        _ => 0.90, // "normale" — the original hardcoded value
    }
}

/// Payload returned by `list_devices`.
#[derive(Clone, Debug, Serialize)]
pub struct DeviceList {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

/// Payload of the `levels` event (VU meters), emitted ~20×/s.
#[derive(Clone, Debug, Serialize)]
pub struct LevelsPayload {
    /// line_id -> peak level [0..1+]
    pub lines: std::collections::HashMap<String, f32>,
    /// output_id -> peak level [0..1+]
    pub outputs: std::collections::HashMap<String, f32>,
}

/// Generates a short unique id without pulling a uuid dependency.
pub fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{prefix}-{:x}-{:x}",
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

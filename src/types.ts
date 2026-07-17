/**
 * TypeScript mirror of the Rust config model (src-tauri/src/audio/model.rs).
 * Field names use snake_case because they cross the IPC boundary as-is.
 */

export interface Route {
  output_id: string;
  /** Per-route gain, linear [0 .. 1.5]. */
  gain: number;
}

export interface LineConfig {
  id: string;
  name: string;
  /** "app" (application audio via virtual cable) or "mic" (physical capture). */
  kind: string;
  /** Accent color (hex) used across the UI. */
  color: string;
  /** cpal device name of the capture endpoint; null = dormant line. */
  input_device: string | null;
  /** Line fader, linear [0 .. 1.5]. */
  gain: number;
  muted: boolean;
  /** LEGACY fixed 5-band gains (migrated by the backend, unused). */
  eq: number[];
  /** Parametric EQ: 1..10 peaking points, editable on the curve. */
  eq_bands: EqBand[];
  /** Executables routed into this line's cable via drag-and-drop. */
  apps: string[];
  /** MMDevice id of the cable's render side (backend cache). */
  cable_render_id: string | null;
  routes: Route[];
  /** Ducking side-chain reactivity when this line is a rule's SOURCE. */
  duck_reactivity: "douce" | "normale" | "rapide" | string;
}

/** Result of assign_app_to_line: config + optional non-fatal notice. */
export interface AssignResult {
  config: AppConfig;
  notice: string | null;
}

export interface OutputConfig {
  id: string;
  name: string;
  /** cpal device name of the render endpoint; "" = unassigned. */
  device: string;
  gain: number;
  muted: boolean;
}

export interface DuckRule {
  source_line: string;
  target_line: string;
  /** 0 = off, 1 = full duck while the source is talking. */
  amount: number;
}

/** One parametric EQ point: peaking filter at `freq` Hz, `gain` dB. */
export interface EqBand {
  freq: number;
  gain: number;
}

export interface EqPreset {
  name: string;
  bands: EqBand[];
}

export interface AppConfig {
  lines: LineConfig[];
  outputs: OutputConfig[];
  ducking: DuckRule[];
  /** Global MASTER ceiling applied to every output bus [0..1]. */
  master_gain: number;
  /** User-saved EQ presets (built-ins live in BUILTIN_EQ_PRESETS). */
  eq_presets: EqPreset[];
  /** User-saved mix profiles — see `Profile`. */
  profiles: Profile[];
  /** Config schema version, bumped by the backend when a migration needs it. */
  schema_version: number;
}

/** One line's settings captured into a `Profile`, by (stable) line id. */
export interface LineSnapshot {
  line_id: string;
  gain: number;
  muted: boolean;
  eq_bands: EqBand[];
  output_devices: string[];
}

/** A saved mix state, optionally auto-applied when `trigger_exe` gets focus. */
export interface Profile {
  id: string;
  name: string;
  trigger_exe: string | null;
  lines: LineSnapshot[];
  ducking: DuckRule[];
  master_gain: number;
}

const b = (freq: number, gain: number): EqBand => ({ freq, gain });

/** Courbe neutre : les 5 bandes classiques à 0 dB (bouton Réinitialiser). */
export const DEFAULT_EQ_BANDS: EqBand[] = [
  b(80, 0),
  b(250, 0),
  b(1000, 0),
  b(4000, 0),
  b(12000, 0),
];

/** Factory EQ curves (parametric). */
export const BUILTIN_EQ_PRESETS: EqPreset[] = [
  { name: "Plat", bands: DEFAULT_EQ_BANDS },
  {
    name: "Clear Voice",
    bands: [b(120, -2), b(400, 1), b(1000, 2), b(3000, 4), b(9000, 3)],
  },
  { name: "Bass Boost", bands: [b(60, 6), b(150, 3), b(12000, 1)] },
  {
    name: "Cinéma",
    bands: [b(50, 4), b(120, 2), b(2000, 3), b(6000, 1), b(12000, 2)],
  },
  {
    name: "FPS",
    bands: [b(80, -3), b(300, 4), b(1500, 2), b(4000, 5), b(8000, 2)],
  },
  {
    name: "Musique",
    bands: [b(60, 4), b(250, 2), b(1000, -1), b(4000, 2), b(12000, 4)],
  },
  {
    name: "Jeu",
    bands: [b(60, 3), b(250, 1), b(2000, 2), b(6000, 3), b(12000, 2)],
  },
  {
    name: "Podcast",
    bands: [b(100, -4), b(400, 2), b(2500, 4), b(8000, 2)],
  },
  { name: "Nuit", bands: [b(60, -6), b(200, -2), b(4000, 1)] },
];

/** Presets dédiés au MICRO (traitement de voix captée, pas d'écoute). */
export const BUILTIN_MIC_EQ_PRESETS: EqPreset[] = [
  { name: "Plat", bands: DEFAULT_EQ_BANDS },
  {
    name: "Broadcast",
    bands: [b(80, -5), b(200, 2), b(700, 1), b(3000, 4), b(10000, 3)],
  },
  {
    name: "Clarté",
    bands: [b(120, -4), b(400, -2), b(3000, 4), b(9000, 3)],
  },
  {
    name: "Voix profonde",
    bands: [b(100, 4), b(250, 2), b(3000, 2), b(8000, 1)],
  },
  {
    name: "Talkie-walkie",
    bands: [b(60, -12), b(150, -8), b(1200, 6), b(2500, 4), b(6000, -10), b(12000, -12)],
  },
  {
    name: "Téléphone",
    bands: [b(100, -10), b(500, 2), b(1500, 4), b(5000, -8), b(12000, -12)],
  },
];

export interface DeviceList {
  inputs: string[];
  outputs: string[];
}

export interface LevelsPayload {
  lines: Record<string, number>;
  outputs: Record<string, number>;
}

export interface EngineStatus {
  warnings: string[];
  active_captures: number;
  active_renders: number;
}

/** One application owning a Windows audio session. */
export interface AppInfo {
  exe: string;
  label: string;
  pid: number;
  active: boolean;
  /** `data:image/bmp;base64,...` small icon, when extraction succeeded. */
  icon: string | null;
}

/** Labels of the 5 EQ bands (must match dsp::EQ_FREQS on the Rust side). */
export const EQ_BANDS = ["80", "250", "1k", "4k", "12k"];

/**
 * Heuristic: does this device name look like a virtual cable rather than
 * physical hardware? Used to group the device dropdowns so users stop
 * confusing "CABLE Output" (a capture endpoint) with a real microphone.
 */
export const isVirtualDevice = (name: string): boolean =>
  /cable|vb-audio|virtual|voicemeeter|blackhole/i.test(name);

/** Preset accent colors cycled through when creating lines. */
export const LINE_COLORS = [
  "#7c3aed",
  "#22d3ee",
  "#f59e0b",
  "#10b981",
  "#ef4444",
  "#ec4899",
  "#3b82f6",
  "#a3e635",
];

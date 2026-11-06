/**
 * Miroir TypeScript de l'état exposé par le backend Linux
 * (src-tauri/src/linux/mod.rs — `LinuxState`).
 */

import type { EqBand, EqPreset } from "../types";

/** Une application qui joue (ou a joué) du son. */
export interface AppView {
  /** Clé stable : binaire de l'appli (ex. "firefox-esr") ou son nom. */
  key: string;
  name: string;
  /** Icône réelle de l'appli (data-URI SVG/PNG), si trouvée. */
  icon: string | null;
  color: string;
  /** A des réglages MixFlow (et donc sa propre chaîne PipeWire). */
  configured: boolean;
  /** Au moins un flux en cours de lecture. */
  playing: boolean;
  /** Nombre de flux audio ouverts. */
  streams: number;
  /** La chaîne (volume + EQ) existe dans PipeWire. */
  chain_ready: boolean;
  /** Volume linéaire [0 .. 1.5]. */
  gain: number;
  muted: boolean;
  /** `node.name` de la sortie ; "" = sortie par défaut du système. */
  output: string;
  eq_bands: EqBand[];
}

/** Une sortie physique ou Bluetooth (HDMI, casque, enceinte…). */
export interface OutputView {
  /** `node.name` PipeWire. */
  name: string;
  label: string;
  bluetooth: boolean;
  /** Sortie par défaut du système (celle que suivent les applis non réglées). */
  is_default: boolean;
  /** Échelle de `wpctl` (1 = 100 %, max 1,5), null si non réglable. */
  volume: number | null;
  muted: boolean;
}

export interface LinuxState {
  apps: AppView[];
  outputs: OutputView[];
  default_output: string;
  eq_presets: EqPreset[];
  pipewire_ok: boolean;
  notice: string | null;
}

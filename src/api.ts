/**
 * Thin typed wrappers over the Tauri commands.
 *
 * Topology commands return the updated AppConfig (the backend is the source
 * of truth for ids); live commands are fire-and-forget — the UI already
 * shows the new value optimistically.
 */

import { invoke } from "@tauri-apps/api/core";
import type { AppConfig, AppInfo, AssignResult, DeviceList, DuckRule, EqBand } from "./types";

// -- queries ----------------------------------------------------------------

export const listDevices = () => invoke<DeviceList>("list_devices");
export const getConfig = () => invoke<AppConfig>("get_config");
export const listApps = () => invoke<AppInfo[]>("list_apps");

// -- app drag-and-drop routing ------------------------------------------------

export const assignApp = (lineId: string, exe: string) =>
  invoke<AssignResult>("assign_app_to_line", { lineId, exe });

export const unassignApp = (lineId: string, exe: string) =>
  invoke<AppConfig>("unassign_app_from_line", { lineId, exe });

// -- topology (returns fresh config, triggers engine rebuild) ----------------

export const addLine = (name: string, color: string, kind: "app" | "mic") =>
  invoke<AppConfig>("add_line", { name, color, kind });

export const removeLine = (id: string) => invoke<AppConfig>("remove_line", { id });

export const setLineInput = (id: string, device: string | null) =>
  invoke<AppConfig>("set_line_input", { id, device });

/**
 * Bind a line to a SET of physical outputs at once (fan-out — e.g. monitor
 * on headphones AND speakers simultaneously). Buses are managed invisibly:
 * pass the full list of currently-selected device names.
 */
export const setLineOutputs = (lineId: string, devices: string[]) =>
  invoke<AppConfig>("set_line_outputs", { lineId, devices });

// -- cosmetic -----------------------------------------------------------------

export const updateLineMeta = (id: string, name: string, color: string) =>
  invoke<void>("update_line_meta", { id, name, color });

// -- live parameters (no rebuild, glitch-free) -------------------------------

export const setLineGain = (id: string, gain: number) =>
  invoke<void>("set_line_gain", { id, gain });

export const setLineMuted = (id: string, muted: boolean) =>
  invoke<void>("set_line_muted", { id, muted });

export const setLineEqBands = (id: string, bands: EqBand[]) =>
  invoke<void>("set_line_eq_bands", { id, bands });

export const saveEqPreset = (name: string, bands: EqBand[]) =>
  invoke<AppConfig>("save_eq_preset", { name, bands });

export const deleteEqPreset = (name: string) => invoke<AppConfig>("delete_eq_preset", { name });

export const setDuckRules = (rules: DuckRule[]) => invoke<void>("set_duck_rules", { rules });

export const setMasterGain = (gain: number) => invoke<void>("set_master_gain", { gain });

export const setRouteGain = (lineId: string, outputId: string, gain: number) =>
  invoke<void>("set_route_gain", { lineId, outputId, gain });

export const setLineDuckReactivity = (id: string, level: "douce" | "normale" | "rapide") =>
  invoke<void>("set_line_duck_reactivity", { id, level });

// -- démarrage avec Windows ---------------------------------------------------

export const getAutostartEnabled = () => invoke<boolean>("get_autostart_enabled");

export const setAutostartEnabled = (enabled: boolean) =>
  invoke<void>("set_autostart_enabled", { enabled });

// -- export / import de la config ---------------------------------------------

/** Opens a native save dialog; returns the chosen path, or null if cancelled. */
export const exportConfig = () => invoke<string | null>("export_config");

/** Opens a native open dialog; returns the loaded config, or null if cancelled. */
export const importConfig = () => invoke<AppConfig | null>("import_config");

// -- profils --------------------------------------------------------------------

export const saveProfile = (name: string, triggerExe: string | null) =>
  invoke<AppConfig>("save_profile", { name, triggerExe });

export const applyProfile = (id: string) => invoke<AppConfig>("apply_profile", { id });

export const deleteProfile = (id: string) => invoke<AppConfig>("delete_profile", { id });

export const setProfileTrigger = (id: string, triggerExe: string | null) =>
  invoke<AppConfig>("set_profile_trigger", { id, triggerExe });

// -- mode streamer ----------------------------------------------------------------

export const enableStreamerMode = (device: string) =>
  invoke<AppConfig>("enable_streamer_mode", { device });

// -- mise à jour ------------------------------------------------------------------

/** Rejects with an explanatory message until a real release endpoint exists. */
export const checkForUpdate = () => invoke<string | null>("check_for_update");

/**
 * Commandes Tauri du backend Linux (préfixe `lx_`).
 *
 * Les réglages live (volume, mute, EQ) ne renvoient rien : l'UI les applique
 * de façon optimiste. Le backend pousse l'état complet via l'événement
 * `lx_state` à chaque changement du graphe PipeWire.
 */

import { invoke } from "@tauri-apps/api/core";
import type { EqBand } from "../types";
import type { LinuxState } from "./types";

export const getState = () => invoke<LinuxState>("lx_get_state");

export const setGain = (key: string, gain: number) => invoke<void>("lx_set_gain", { key, gain });

export const setMuted = (key: string, muted: boolean) =>
  invoke<void>("lx_set_muted", { key, muted });

export const setEq = (key: string, bands: EqBand[]) => invoke<void>("lx_set_eq", { key, bands });

/** `output` = `node.name` de la sortie, ou "" pour la sortie par défaut. */
export const setOutput = (key: string, output: string) =>
  invoke<LinuxState>("lx_set_output", { key, output });

export const forgetApp = (key: string) => invoke<LinuxState>("lx_forget_app", { key });

export const savePreset = (name: string, bands: EqBand[]) =>
  invoke<LinuxState>("lx_save_preset", { name, bands });

export const deletePreset = (name: string) => invoke<LinuxState>("lx_delete_preset", { name });

/** Volume d'une sortie (`node.name`), 0..1,5 (échelle des mélangeurs du bureau). */
export const setOutputVolume = (output: string, volume: number) =>
  invoke<void>("lx_set_output_volume", { output, volume });

export const setOutputMuted = (output: string, muted: boolean) =>
  invoke<void>("lx_set_output_muted", { output, muted });

/** Change la sortie par défaut du système (`node.name`). */
export const setDefaultOutput = (output: string) =>
  invoke<LinuxState>("lx_set_default_output", { output });

export const dismissNotice = () => invoke<void>("lx_dismiss_notice");

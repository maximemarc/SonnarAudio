/**
 * VU levels store — deliberately OUTSIDE React state.
 *
 * The backend emits a "levels" event 20×/s; re-rendering the whole tree at
 * that rate would be wasteful. Instead the payload lands in this mutable
 * module store and each <VuMeter> reads it inside its own rAF loop, writing
 * directly to a DOM node's style.
 */

import { listen } from "@tauri-apps/api/event";
import type { LevelsPayload } from "./types";

export const levelStore: LevelsPayload = { lines: {}, outputs: {} };

let started = false;

/** Idempotent — call once at app mount. */
export function initLevels(): void {
  if (started) return;
  started = true;
  void listen<LevelsPayload>("levels", (event) => {
    levelStore.lines = event.payload.lines;
    levelStore.outputs = event.payload.outputs;
  });
}

/**
 * Map a linear peak to a display fraction [0..1].
 * sqrt stretches the low end so quiet signals stay visible (pseudo-dB feel).
 */
export function levelToFraction(peak: number): number {
  return Math.min(1, Math.sqrt(Math.max(0, peak)));
}

/**
 * Raw peak for one meter — `id === "*"` aggregates every meter of the kind
 * (used by the MASTER strip: the loudest output bus).
 */
export function peakOf(kind: "lines" | "outputs", id: string): number {
  if (id !== "*") return levelStore[kind][id] ?? 0;
  let max = 0;
  for (const v of Object.values(levelStore[kind])) {
    if (v > max) max = v;
  }
  return max;
}

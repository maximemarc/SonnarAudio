/**
 * Réponse en fréquence de l'EQ paramétrique — mêmes biquads peaking (RBJ)
 * que le moteur Rust (dsp.rs), évalués en JS pour dessiner les courbes.
 * Partagé entre la grande page Égaliseur et les mini-aperçus des tranches.
 */

import type { EqBand } from "./types";

export const EQ_Q = 1.0;
const FS = 48000;
export const FREQ_MIN = 20;
export const FREQ_MAX = 20000;
export const DB_RANGE = 14; // ± affiché

/** Gain (dB) apporté par UNE bande peaking à la fréquence f. */
export function bandDbAt(f: number, band: EqBand): number {
  if (Math.abs(band.gain) < 0.05) return 0;
  const A = Math.pow(10, band.gain / 40);
  const w0 = (2 * Math.PI * band.freq) / FS;
  const alpha = Math.sin(w0) / (2 * EQ_Q);
  const cos0 = Math.cos(w0);
  const b0 = 1 + alpha * A;
  const b1 = -2 * cos0;
  const b2 = 1 - alpha * A;
  const a0 = 1 + alpha / A;
  const a1 = -2 * cos0;
  const a2 = 1 - alpha / A;
  const w = (2 * Math.PI * f) / FS;
  const cw = Math.cos(w);
  const sw = Math.sin(w);
  const c2 = Math.cos(2 * w);
  const s2 = Math.sin(2 * w);
  const nRe = b0 + b1 * cw + b2 * c2;
  const nIm = -(b1 * sw + b2 * s2);
  const dRe = a0 + a1 * cw + a2 * c2;
  const dIm = -(a1 * sw + a2 * s2);
  const mag = Math.sqrt((nRe * nRe + nIm * nIm) / (dRe * dRe + dIm * dIm));
  return 20 * Math.log10(mag);
}

/** Somme des bandes à la fréquence f, en dB. */
export function curveDbAt(f: number, bands: EqBand[]): number {
  let db = 0;
  for (const b of bands) db += bandDbAt(f, b);
  return db;
}

/** x [0..w] ↔ fréquence, échelle logarithmique. */
export const xToFreq = (x: number, w: number): number =>
  FREQ_MIN * Math.pow(FREQ_MAX / FREQ_MIN, x / w);
export const freqToX = (f: number, w: number): number =>
  (Math.log(f / FREQ_MIN) / Math.log(FREQ_MAX / FREQ_MIN)) * w;

/** y [0..h] ↔ gain dB (±DB_RANGE), 8 px de marge en haut/bas. */
export const dbToY = (db: number, h: number): number =>
  h / 2 - (Math.max(-DB_RANGE, Math.min(DB_RANGE, db)) / DB_RANGE) * (h / 2 - 8);
export const yToDb = (y: number, h: number): number =>
  Math.max(-12, Math.min(12, ((h / 2 - y) / (h / 2 - 8)) * DB_RANGE));

/** Points SVG "x,y x,y…" de la courbe cumulée. */
export function curvePoints(bands: EqBand[], w: number, h: number, n = 110): string {
  const pts: string[] = [];
  for (let i = 0; i <= n; i++) {
    const f = xToFreq((i / n) * w, w);
    const y = dbToY(curveDbAt(f, bands), h);
    pts.push(`${((i / n) * w).toFixed(1)},${y.toFixed(1)}`);
  }
  return pts.join(" ");
}

/** Étiquette de fréquence compacte : 80, 250, 1k, 4.2k… */
export function freqLabel(f: number): string {
  if (f < 1000) return `${Math.round(f)}`;
  const k = f / 1000;
  return `${k < 10 ? k.toFixed(1).replace(/\.0$/, "") : Math.round(k)}k`;
}

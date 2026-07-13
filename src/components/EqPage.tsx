/**
 * Onglet Égaliseur — EQ paramétrique éditable directement sur la courbe :
 * - glisser un point   = changer sa fréquence ET son gain,
 * - double-clic vide   = ajouter un point (max 10),
 * - clic droit / ×     = supprimer un point (min 1),
 * - presets d'usine + presets custom persistés.
 * Chaque point est un biquad peaking identique à ceux du moteur Rust.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import type { AppConfig, EqBand, EqPreset } from "../types";
import { BUILTIN_EQ_PRESETS, BUILTIN_MIC_EQ_PRESETS, DEFAULT_EQ_BANDS } from "../types";
import { curvePoints, dbToY, freqToX, xToFreq, yToDb } from "../eqMath";
import { MicIcon } from "./Icons";
import { pickIcon } from "./pickIcon";

const W = 720;
const H = 230;
const MAX_BANDS = 10;
const SEND_MS = 60;

/**
 * One point's numeric freq/dB fields, precise keyboard entry as an
 * alternative to dragging. Buffers text locally and only commits (parses,
 * clamps, re-sorts) on blur/Enter — same pattern as the channel-name input
 * elsewhere in the app — so the field doesn't reflow or lose the caret
 * while the user is mid-keystroke.
 */
function EqPointChip({
  band,
  canDelete,
  onCommitFreq,
  onCommitGain,
  onDelete,
}: {
  band: EqBand;
  canDelete: boolean;
  onCommitFreq: (freq: number) => void;
  onCommitGain: (gain: number) => void;
  onDelete: () => void;
}) {
  const [freqText, setFreqText] = useState(String(Math.round(band.freq)));
  const [gainText, setGainText] = useState(band.gain.toFixed(1));
  useEffect(() => setFreqText(String(Math.round(band.freq))), [band.freq]);
  useEffect(() => setGainText(band.gain.toFixed(1)), [band.gain]);

  const commitFreq = () => {
    const parsed = Number(freqText);
    const freq = Number.isFinite(parsed) ? Math.min(20000, Math.max(20, parsed)) : band.freq;
    setFreqText(String(Math.round(freq)));
    if (Math.abs(freq - band.freq) > 0.001) onCommitFreq(freq);
  };
  const commitGain = () => {
    const parsed = Number(gainText);
    const gain = Number.isFinite(parsed) ? Math.min(12, Math.max(-12, parsed)) : band.gain;
    setGainText(gain.toFixed(1));
    if (Math.abs(gain - band.gain) > 0.001) onCommitGain(gain);
  };
  const blurOnEnter = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") (e.target as HTMLInputElement).blur();
  };

  return (
    <span className="eq-point-chip">
      <input
        className="eq-point-input"
        inputMode="numeric"
        value={freqText}
        title="Fréquence (Hz)"
        onChange={(e) => setFreqText(e.target.value)}
        onBlur={commitFreq}
        onKeyDown={blurOnEnter}
      />
      <span className="eq-point-unit">Hz</span>
      <input
        className="eq-point-input eq-point-input-gain"
        inputMode="decimal"
        value={gainText}
        title="Gain (dB)"
        onChange={(e) => setGainText(e.target.value)}
        onBlur={commitGain}
        onKeyDown={blurOnEnter}
      />
      <span className="eq-point-unit">dB</span>
      <button
        className="strip-app-x"
        title="Supprimer ce point"
        disabled={!canDelete}
        onClick={onDelete}
      >
        ×
      </button>
    </span>
  );
}

interface Props {
  config: AppConfig;
  selectedId: string;
  onSelect: (lineId: string) => void;
  /** La courbe change (drag, ajout, suppression, preset). */
  onBands: (lineId: string, bands: EqBand[]) => void;
  onSavePreset: (name: string, bands: EqBand[]) => void;
  onDeletePreset: (name: string) => void;
}

export default function EqPage({
  config,
  selectedId,
  onSelect,
  onBands,
  onSavePreset,
  onDeletePreset,
}: Props) {
  const [presetName, setPresetName] = useState("");
  const [dragIdx, setDragIdx] = useState<number | null>(null);
  // Position live du point pendant le drag — découplée du round-trip backend
  // throttlé, sinon un drag rapide "gèle" ou se perd entièrement (le point
  // revient à sa position de départ si le pointerup tombe dans la même
  // fenêtre de throttle que le dernier pointermove envoyé).
  const [dragBands, setDragBands] = useState<EqBand[] | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const lastSent = useRef(0);

  const line = config.lines.find((l) => l.id === selectedId) ?? config.lines[0];
  const savedBands = useMemo(() => line?.eq_bands ?? [], [line]);
  const bands = dragBands ?? savedBands;
  const path = useMemo(() => curvePoints(bands, W, H), [bands]);
  // Un micro ne s'égalise pas comme un canal d'écoute : presets dédiés.
  const builtinPresets = line?.kind === "mic" ? BUILTIN_MIC_EQ_PRESETS : BUILTIN_EQ_PRESETS;

  if (!line) return <p className="empty-hint">Aucun canal.</p>;

  /** Coordonnées SVG d'un événement pointeur. */
  const svgPos = (e: React.PointerEvent | React.MouseEvent) => {
    const rect = svgRef.current!.getBoundingClientRect();
    return {
      x: ((e.clientX - rect.left) / rect.width) * W,
      y: ((e.clientY - rect.top) / rect.height) * H,
    };
  };

  /** Envoie la courbe (throttle pendant le drag, immédiat sinon). */
  const send = (next: EqBand[], force: boolean) => {
    const now = performance.now();
    if (!force && now - lastSent.current < SEND_MS) return;
    lastSent.current = now;
    onBands(line.id, next);
  };

  const sameCurve = (p: EqPreset) =>
    p.bands.length === bands.length &&
    p.bands.every(
      (b, i) => Math.abs(b.gain - bands[i].gain) < 0.05 && Math.abs(b.freq - bands[i].freq) < 1,
    );

  return (
    <div className="eq-page">
      {/* Sélecteur de canal */}
      <div className="eq-channels">
        {config.lines.map((l) => (
          <button
            key={l.id}
            className={`eq-chan ${l.id === line.id ? "on" : ""}`}
            style={{ "--accent": l.color } as React.CSSProperties}
            onClick={() => onSelect(l.id)}
          >
            <span className="strip-icon">{l.kind === "mic" ? <MicIcon /> : pickIcon(l.name)}</span>
            {l.name}
          </button>
        ))}
      </div>

      <div className="eq-editor" style={{ "--accent": line.color } as React.CSSProperties}>
        <p className="eq-help">
          Glisse les points • double-clic sur la courbe : ajouter • clic droit sur un point :
          supprimer
        </p>

        {/* Courbe interactive */}
        <svg
          ref={svgRef}
          className={`eq-curve ${dragIdx !== null ? "dragging" : ""}`}
          viewBox={`0 0 ${W} ${H}`}
          preserveAspectRatio="none"
          onDoubleClick={(e) => {
            if (bands.length >= MAX_BANDS) return;
            const { x, y } = svgPos(e);
            const next = [...bands, { freq: xToFreq(x, W), gain: yToDb(y, H) }].sort(
              (a, b) => a.freq - b.freq,
            );
            send(next, true);
          }}
          onPointerMove={(e) => {
            if (dragIdx === null) return;
            const { x, y } = svgPos(e);
            const next = bands.map((b, i) =>
              i === dragIdx
                ? {
                    freq: Math.min(20000, Math.max(20, xToFreq(x, W))),
                    gain: yToDb(y, H),
                  }
                : b,
            );
            // Feedback visuel instantané (jamais throttlé)…
            setDragBands(next);
            // …le round-trip backend reste throttlé pour ne pas spammer l'IPC.
            send(next, false);
          }}
          onPointerUp={() => {
            if (dragIdx !== null) {
              setDragIdx(null);
              // Envoie la VRAIE dernière position pointée (dragBands), pas
              // la valeur "bands" potentiellement périmée si le dernier
              // pointermove est tombé dans la fenêtre de throttle.
              const final = [...(dragBands ?? bands)].sort((a, b) => a.freq - b.freq);
              send(final, true);
              setDragBands(null);
            }
          }}
        >
          {/* grille 0 / ±6 dB */}
          <line x1={0} y1={H / 2} x2={W} y2={H / 2} className="eq-grid-zero" />
          {[6, -6].map((db) => (
            <line key={db} x1={0} y1={dbToY(db, H)} x2={W} y2={dbToY(db, H)} className="eq-grid" />
          ))}
          {/* repères fréquence 100 / 1k / 10k */}
          {[100, 1000, 10000].map((f) => (
            <line key={f} x1={freqToX(f, W)} y1={0} x2={freqToX(f, W)} y2={H} className="eq-grid" />
          ))}
          <polyline points={`0,${H / 2} ${path} ${W},${H / 2}`} className="eq-fill" />
          <polyline points={path} className="eq-line" />
          {/* points éditables */}
          {bands.map((b, i) => (
            <g key={i}>
              <circle
                cx={freqToX(b.freq, W)}
                cy={dbToY(b.gain, H)}
                r={14}
                className="eq-dot-hit"
                onPointerDown={(e) => {
                  e.stopPropagation();
                  (e.target as Element).setPointerCapture(e.pointerId);
                  setDragIdx(i);
                }}
                onContextMenu={(e) => {
                  e.preventDefault();
                  if (bands.length <= 1) return;
                  send(
                    bands.filter((_, j) => j !== i),
                    true,
                  );
                }}
              />
              <circle cx={freqToX(b.freq, W)} cy={dbToY(b.gain, H)} r={5.5} className="eq-dot" />
            </g>
          ))}
        </svg>

        {/* Détail des points — saisie précise au clavier, alternative au glisser. */}
        <div className="eq-points">
          {bands.map((b, i) => (
            <EqPointChip
              key={i}
              band={b}
              canDelete={bands.length > 1}
              onCommitFreq={(freq) => {
                const next = bands
                  .map((x, j) => (j === i ? { ...x, freq } : x))
                  .sort((a, c) => a.freq - c.freq);
                send(next, true);
              }}
              onCommitGain={(gain) => {
                const next = bands.map((x, j) => (j === i ? { ...x, gain } : x));
                send(next, true);
              }}
              onDelete={() =>
                send(
                  bands.filter((_, j) => j !== i),
                  true,
                )
              }
            />
          ))}
          {bands.length < MAX_BANDS && (
            <button
              className="eq-preset"
              title="Ajouter un point à 1 kHz"
              onClick={() =>
                send(
                  [...bands, { freq: 1000, gain: 0 }].sort((a, b) => a.freq - b.freq),
                  true,
                )
              }
            >
              + Point
            </button>
          )}
          <button
            className="eq-preset eq-reset"
            title="Revenir à la courbe neutre (5 points à 0 dB)"
            onClick={() =>
              send(
                DEFAULT_EQ_BANDS.map((p) => ({ ...p })),
                true,
              )
            }
          >
            ↺ Réinitialiser
          </button>
        </div>
      </div>

      {/* Presets */}
      <div className="eq-presets">
        <span className="strip-label">Presets</span>
        <div className="eq-preset-row">
          {builtinPresets.map((p) => (
            <button
              key={p.name}
              className={`eq-preset ${sameCurve(p) ? "on" : ""}`}
              onClick={() => send([...p.bands], true)}
            >
              {p.name}
            </button>
          ))}
          {config.eq_presets.map((p) => (
            <span key={p.name} className={`eq-preset eq-preset-custom ${sameCurve(p) ? "on" : ""}`}>
              <button onClick={() => send([...p.bands], true)}>{p.name}</button>
              <button
                className="strip-app-x"
                title="Supprimer ce preset"
                onClick={() => onDeletePreset(p.name)}
              >
                ×
              </button>
            </span>
          ))}
        </div>
        <div className="eq-save">
          <input
            className="eq-save-name"
            placeholder="Nom du preset…"
            value={presetName}
            onChange={(e) => setPresetName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && presetName.trim()) {
                onSavePreset(presetName.trim(), bands);
                setPresetName("");
              }
            }}
            spellCheck={false}
          />
          <button
            className="btn-ghost"
            disabled={!presetName.trim()}
            onClick={() => {
              onSavePreset(presetName.trim(), bands);
              setPresetName("");
            }}
          >
            Enregistrer la courbe
          </button>
        </div>
      </div>
    </div>
  );
}

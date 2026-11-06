/**
 * Bandeau « Sorties » (Linux), en haut de la console : une ligne par sortie
 * (HDMI, casque, Bluetooth…) avec son volume (jusqu'à 150 %), son mute, son
 * VU réel et le choix de la sortie par défaut.
 *
 * La sortie par défaut est mise en avant : c'est le « son général », celui
 * que suivent les applis non réglées et le bouton de volume du bureau.
 * Toujours affiché, même quand aucune appli ne joue.
 */

import { useEffect, useRef, useState } from "react";
import { SpeakerIcon, SpeakerOffIcon } from "../components/Icons";
import { levelToFraction, peakOf } from "../levels";
import type { OutputView } from "./types";

const MAX_VOLUME = 1.5;
const THROTTLE_MS = 50;

interface Props {
  outputs: OutputView[];
  onVolume: (output: string, volume: number) => void;
  onMute: (output: string, muted: boolean) => void;
  onMakeDefault: (output: string) => void;
}

export default function OutputsBar({ outputs, onVolume, onMute, onMakeDefault }: Props) {
  if (outputs.length === 0) {
    return (
      <p className="panel-hint">
        Aucune sortie audio détectée (écran HDMI en veille, casque Bluetooth éteint ?).
      </p>
    );
  }
  // La sortie par défaut en premier, puis l'ordre alphabétique du backend.
  const sorted = [...outputs].sort((a, b) => Number(b.is_default) - Number(a.is_default));
  return (
    <div className="lx-outputs">
      {sorted.map((o) => (
        <OutputRow
          key={o.name}
          output={o}
          onVolume={(v) => onVolume(o.name, v)}
          onMute={(m) => onMute(o.name, m)}
          onMakeDefault={() => onMakeDefault(o.name)}
        />
      ))}
    </div>
  );
}

function OutputRow({
  output,
  onVolume,
  onMute,
  onMakeDefault,
}: {
  output: OutputView;
  onVolume: (v: number) => void;
  onMute: (m: boolean) => void;
  onMakeDefault: () => void;
}) {
  const { volume, muted, is_default } = output;
  // Valeur locale pendant le glissé : l'état du backend arrive en retard.
  const [local, setLocal] = useState(volume ?? 0);
  const dragging = useRef(false);
  const lastSent = useRef(0);
  useEffect(() => {
    if (!dragging.current && volume !== null) setLocal(volume);
  }, [volume]);

  const change = (v: number, force: boolean) => {
    setLocal(v);
    const now = performance.now();
    if (force || now - lastSent.current >= THROTTLE_MS) {
      lastSent.current = now;
      onVolume(v);
    }
  };

  return (
    <div className={`lx-out ${is_default ? "lx-out-default" : ""} ${muted ? "lx-out-muted" : ""}`}>
      <div className="lx-out-name" title={output.name}>
        <span className="lx-out-label">
          {output.bluetooth ? "Bluetooth · " : ""}
          {output.label}
        </span>
        {is_default ? (
          <span className="lx-badge lx-badge-default" title="Sortie utilisée par défaut">
            Par défaut
          </span>
        ) : (
          <button
            className="lx-out-make-default"
            title="En faire la sortie par défaut du système"
            onClick={onMakeDefault}
          >
            Utiliser par défaut
          </button>
        )}
      </div>

      <button
        className={`btn-mute ${muted ? "on" : ""}`}
        title={muted ? "Réactiver cette sortie" : "Couper cette sortie"}
        onClick={() => onMute(!muted)}
      >
        {muted ? <SpeakerOffIcon /> : <SpeakerIcon />}
      </button>

      <div className="lx-out-fader">
        {volume === null ? (
          <span className="lx-out-novol">Volume non réglable</span>
        ) : (
          <>
            <input
              type="range"
              className="lx-range"
              min={0}
              max={MAX_VOLUME}
              step={0.01}
              value={local}
              style={{ "--fill": `${(local / MAX_VOLUME) * 100}%` } as React.CSSProperties}
              title="Volume (double-clic : 100 %) — au-delà de 100 %, le son est amplifié"
              onPointerDown={() => (dragging.current = true)}
              onPointerUp={(e) => {
                dragging.current = false;
                change(Number((e.target as HTMLInputElement).value), true);
              }}
              onChange={(e) => change(Number(e.target.value), false)}
              onDoubleClick={() => change(1, true)}
            />
            {/* Repère 100 % : au-delà, on amplifie. */}
            <span className="lx-range-mark" style={{ left: `${(1 / MAX_VOLUME) * 100}%` }} />
          </>
        )}
        <HMeter id={output.name} />
      </div>

      <span className={`lx-out-value ${local > 1 ? "boost" : ""}`}>
        {volume === null ? "—" : `${Math.round(local * 100)}%`}
      </span>
    </div>
  );
}

/** VU horizontal, animé hors React (rAF) comme `LevelBars`. */
function HMeter({ id }: { id: string }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let raf = 0;
    let shown = 0;
    const tick = () => {
      const level = levelToFraction(peakOf("outputs", id));
      shown = level > shown ? level : shown * 0.9;
      if (ref.current) ref.current.style.transform = `scaleX(${shown.toFixed(3)})`;
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [id]);
  return (
    <div className="lx-hmeter" aria-hidden>
      <div className="lx-hmeter-fill" ref={ref} />
    </div>
  );
}

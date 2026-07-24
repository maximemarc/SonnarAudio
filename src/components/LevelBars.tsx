/**
 * Les trois barres de VU posées à côté du fader, comme dans la maquette.
 *
 * La maquette les anime en CSS (`meterPulse`) : c'est décoratif, ça bouge même
 * sans audio. Ici elles sont pilotées par les VRAIS niveaux, avec trois
 * vitesses de relâchement différentes (rapide / moyenne / lente) — ce qui
 * donne le même grappe vivante à l'œil tout en restant une mesure honnête.
 *
 * Comme VSlider, tout passe par requestAnimationFrame hors de React : aucun
 * re-render au rythme du métering.
 */

import { useEffect, useRef } from "react";
import { levelToFraction, peakOf } from "../levels";

interface Props {
  kind: "lines" | "outputs";
  /** id de la ligne / du bus, ou "*" pour l'agrégat. */
  id: string;
}

/** Coefficient de décroissance par frame, du plus vif au plus posé. */
const RELEASE = [0.82, 0.9, 0.95];
/** Hauteurs extrêmes de la maquette (px), pour un conteneur de 34px. */
const MIN_H = 6;
const MAX_H = 30;

export default function LevelBars({ kind, id }: Props) {
  const refs = useRef<(HTMLDivElement | null)[]>([]);

  useEffect(() => {
    let raf = 0;
    const shown = [0, 0, 0];
    const tick = () => {
      const target = levelToFraction(peakOf(kind, id));
      for (let i = 0; i < 3; i++) {
        // Attaque instantanée, relâchement propre à chaque barre.
        shown[i] = target > shown[i] ? target : shown[i] * RELEASE[i];
        const el = refs.current[i];
        if (el) el.style.height = `${(MIN_H + shown[i] * (MAX_H - MIN_H)).toFixed(1)}px`;
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [kind, id]);

  return (
    <div className="level-bars" aria-hidden>
      {[0, 1, 2].map((i) => (
        <div
          key={i}
          className="level-bar"
          ref={(el) => {
            refs.current[i] = el;
          }}
        />
      ))}
    </div>
  );
}

/**
 * Les trois barres de VU posées à côté du fader, comme dans la maquette.
 *
 * La maquette les anime en CSS (`meterPulse`) : purement décoratif, ça bouge
 * même sans audio. Ici elles montrent les VRAIS niveaux, avec trois vitesses
 * de relâchement (vif / moyen / posé) qui donnent la même grappe vivante.
 *
 * Au repos, une respiration lente de faible amplitude prend le relais quand
 * la ligne est ACTIVE mais silencieuse : sans elle, un canal branché mais
 * sans son est indiscernable d'une interface figée. Une ligne sans source
 * (`idle=false`) reste franchement à plat — c'est une information, pas une
 * panne, et l'animer serait mentir.
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
  /** La ligne capture-t-elle quelque chose ? Pilote la respiration au repos. */
  idle?: boolean;
}

/** Coefficient de décroissance par frame, du plus vif au plus posé. */
const RELEASE = [0.82, 0.9, 0.95];
/** Hauteurs extrêmes de la maquette (px), pour un conteneur de 34px. */
const MIN_H = 6;
const MAX_H = 30;
/** Amplitude de la respiration au repos, en fraction de la course. */
const IDLE_AMPLITUDE = 0.18;
/** Périodes (ms) décalées par barre, pour éviter un clignotement synchrone. */
const IDLE_PERIOD = [1700, 2300, 2900];

export default function LevelBars({ kind, id, idle = false }: Props) {
  const refs = useRef<(HTMLDivElement | null)[]>([]);
  // Lu dans la boucle rAF sans la relancer quand la ligne change d'état.
  const idleRef = useRef(idle);
  idleRef.current = idle;

  useEffect(() => {
    let raf = 0;
    const shown = [0, 0, 0];
    const start = performance.now();
    const tick = (now: number) => {
      const level = levelToFraction(peakOf(kind, id));
      for (let i = 0; i < 3; i++) {
        // Attaque instantanée, relâchement propre à chaque barre.
        shown[i] = level > shown[i] ? level : shown[i] * RELEASE[i];
        // La respiration ne s'ajoute JAMAIS au signal : elle sert de plancher
        // quand il n'y a rien à montrer, donc un vrai niveau la masque dès
        // qu'il la dépasse.
        const breath = idleRef.current
          ? (1 - Math.cos((2 * Math.PI * (now - start)) / IDLE_PERIOD[i])) / 2
          : 0;
        const shownHeight = Math.max(shown[i], breath * IDLE_AMPLITUDE);
        const el = refs.current[i];
        if (el) el.style.height = `${(MIN_H + shownHeight * (MAX_H - MIN_H)).toFixed(1)}px`;
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

/**
 * Drives the "signal present" glow of a strip: a rAF loop reads the level
 * store and writes the smoothed value into the element's `--live` CSS
 * variable (0..1). The glow overlay uses `opacity: var(--live)`, so the
 * whole effect costs zero React re-renders.
 */

import { useEffect, useRef } from "react";
import { levelToFraction, peakOf } from "./levels";

export function useLiveLevel(kind: "lines" | "outputs", id: string) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let raf = 0;
    let shown = 0;
    const tick = () => {
      const raw = peakOf(kind, id);
      const target = levelToFraction(raw);
      shown = target > shown ? target : shown * 0.9;
      ref.current?.style.setProperty("--live", Math.min(1, shown * 1.4).toFixed(3));
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [kind, id]);

  return ref;
}

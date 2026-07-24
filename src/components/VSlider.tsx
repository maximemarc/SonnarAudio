/**
 * Vertical slider (console fader / EQ band), pointer-driven for full visual
 * control. Local state keeps the drag fluid; backend calls are throttled to
 * ~20 Hz with a guaranteed final send on release. Double-click resets to
 * `resetValue` when provided.
 *
 * Le VU ne vit plus dans la piste : la maquette le place à CÔTÉ du fader,
 * sous la forme de trois barres (voir `LevelBars`). La piste ne porte donc
 * plus que le remplissage de gain.
 */

import { useEffect, useRef, useState } from "react";

interface Props {
  value: number;
  min: number;
  max: number;
  onChange: (v: number) => void;
  accent?: string;
  /** Fill from the vertical center (bipolar EQ) instead of the bottom. */
  center?: boolean;
  /** Double-click snaps back to this value. */
  resetValue?: number;
  /** "vs-fader" (large) or "vs-eq" (mini). */
  variant: "fader" | "eq";
  title?: string;
}

const THROTTLE_MS = 50;

export default function VSlider({
  value,
  min,
  max,
  onChange,
  accent,
  center,
  resetValue,
  variant,
  title,
}: Props) {
  const [local, setLocal] = useState(value);
  const dragging = useRef(false);
  const trackRef = useRef<HTMLDivElement>(null);
  const lastSent = useRef(0);
  const pending = useRef<number | null>(null);

  // Adopt external updates when the user isn't holding the thumb.
  useEffect(() => {
    if (!dragging.current) setLocal(value);
  }, [value]);

  const send = (v: number, force: boolean) => {
    const now = performance.now();
    if (force || now - lastSent.current >= THROTTLE_MS) {
      lastSent.current = now;
      pending.current = null;
      onChange(v);
    } else {
      pending.current = v;
    }
  };

  // Trailing flush for the throttle.
  useEffect(() => {
    const t = setInterval(() => {
      if (pending.current !== null) {
        const v = pending.current;
        pending.current = null;
        lastSent.current = performance.now();
        onChange(v);
      }
    }, THROTTLE_MS);
    return () => clearInterval(t);
  }, [onChange]);

  const valueFromPointer = (clientY: number): number => {
    const rect = trackRef.current!.getBoundingClientRect();
    const frac = 1 - (clientY - rect.top) / rect.height;
    return Math.min(max, Math.max(min, min + frac * (max - min)));
  };

  const frac = (local - min) / (max - min);
  const fill = center
    ? {
        bottom: `${Math.min(frac, 0.5) * 100}%`,
        height: `${Math.abs(frac - 0.5) * 100}%`,
      }
    : { bottom: "0%", height: `${frac * 100}%` };

  return (
    <div
      className={`vs ${variant === "fader" ? "vs-fader" : "vs-eq"}`}
      title={title}
      style={accent ? ({ "--accent": accent } as React.CSSProperties) : undefined}
      onDoubleClick={() => {
        if (resetValue !== undefined) {
          setLocal(resetValue);
          send(resetValue, true);
        }
      }}
      onPointerDown={(e) => {
        dragging.current = true;
        (e.target as HTMLElement).setPointerCapture(e.pointerId);
        const v = valueFromPointer(e.clientY);
        setLocal(v);
        send(v, false);
      }}
      onPointerMove={(e) => {
        if (!dragging.current) return;
        const v = valueFromPointer(e.clientY);
        setLocal(v);
        send(v, false);
      }}
      onPointerUp={() => {
        dragging.current = false;
        send(local, true);
      }}
      tabIndex={0}
      role="slider"
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={local}
      onKeyDown={(e) => {
        const step = (max - min) / 30;
        if (e.key === "ArrowUp" || e.key === "ArrowDown") {
          e.preventDefault();
          const v = Math.min(max, Math.max(min, local + (e.key === "ArrowUp" ? step : -step)));
          setLocal(v);
          send(v, true);
        }
      }}
    >
      <div className="vs-track" ref={trackRef}>
        {center && <div className="vs-zero" />}
        <div className="vs-fill" style={fill} />
        <div className="vs-thumb" style={{ bottom: `calc(${frac * 100}% - 6px)` }} />
      </div>
    </div>
  );
}

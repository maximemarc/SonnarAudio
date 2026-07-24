/**
 * MASTER strip (Sonar-style): one big fader that scales the WHOLE mix
 * (applied to every output bus in the engine), an aggregated VU, and the
 * "À router" pool — applications still playing on the default output,
 * waiting to be dragged onto a channel.
 */

import type { AppInfo } from "../types";
import { useLiveLevel } from "../useLiveLevel";
import LevelBars from "./LevelBars";
import VSlider from "./VSlider";
import { MasterIcon } from "./Icons";

const MASTER_ACCENT = "#5b8def";

interface Props {
  /** Global master gain [0..1]. */
  value: number;
  onChange: (gain: number) => void;
  /** Detected apps not routed to any channel yet. */
  unrouted: AppInfo[];
  onRefresh: () => void;
}

export default function MasterStrip({ value, onChange, unrouted, onRefresh }: Props) {
  const glowRef = useLiveLevel("outputs", "*");

  return (
    <div
      ref={glowRef}
      className="strip strip-master"
      style={{ "--accent": MASTER_ACCENT } as React.CSSProperties}
    >
      <div className="strip-cap strip-cap-bus" />
      <div className="strip-glow" />

      {/* La maquette n'a qu'une icône dans la carte : le titre « Master »
          vit à l'extérieur, porté par le <h2> de la section. */}
      <div className="strip-head strip-head-master">
        <span className="strip-icon">
          <MasterIcon />
        </span>
      </div>

      <div className="strip-fader-row">
        {/* Le master agrège tous les bus : il y a toujours une sortie
            derrière, donc il respire au repos. */}
        <LevelBars kind="outputs" id="*" idle />
        <div className="strip-main">
          <VSlider
            variant="fader"
            min={0}
            max={1}
            resetValue={1}
            value={value}
            accent={MASTER_ACCENT}
            onChange={onChange}
            title="Volume global — baisse tout le mix (double-clic : 100 %)"
          />
        </div>
        <span className="strip-value strip-value-master">{Math.round(value * 100)}%</span>
      </div>

      <div className="strip-apps-zone">
        <span className="strip-label">
          À router
          <button
            className="apps-refresh"
            title="Re-scanner les applications audio"
            onClick={onRefresh}
          >
            ⟳
          </button>
        </span>
        <div className="strip-apps">
          {unrouted.length === 0 && <span className="apps-hint">Tout est routé ✓</span>}
          {unrouted.map((app) => (
            <span
              className={`strip-app app-pool ${app.active ? "app-live" : ""}`}
              key={app.exe}
              title={`${app.exe} — joue sur la sortie par défaut. Glisser sur un canal pour router.`}
              draggable
              onDragStart={(e) => {
                e.dataTransfer.setData("mixflow/app", app.exe);
                e.dataTransfer.effectAllowed = "copy";
              }}
            >
              {app.active && <span className="app-dot" />}
              {app.icon && <img className="app-icon" src={app.icon} alt="" />}
              {app.label}
            </span>
          ))}
        </div>
      </div>
    </div>
  );
}

/**
 * Tranche d'une application (Linux) : icône + nom, sortie, mini-courbe
 * d'EQ, fader de volume et mute. Même gabarit visuel que `ChannelStrip`.
 *
 * Une appli encore jamais réglée suit la sortie par défaut, sans
 * traitement : le premier réglage touché crée sa chaîne PipeWire.
 */

import { useMemo } from "react";
import LevelBars from "../components/LevelBars";
import VSlider from "../components/VSlider";
import { ChevronRightIcon, SpeakerIcon, SpeakerOffIcon } from "../components/Icons";
import { pickIcon } from "../components/pickIcon";
import { curvePoints } from "../eqMath";
import type { AppView, OutputView } from "./types";

interface Props {
  app: AppView;
  outputs: OutputView[];
  defaultOutputLabel: string;
  onGain: (gain: number) => void;
  onMute: (muted: boolean) => void;
  onOutput: (output: string) => void;
  onOpenEq: () => void;
  onForget: () => void;
}

export default function AppStrip({
  app,
  outputs,
  defaultOutputLabel,
  onGain,
  onMute,
  onOutput,
  onOpenEq,
  onForget,
}: Props) {
  const miniCurve = useMemo(() => curvePoints(app.eq_bands, 130, 40, 60), [app.eq_bands]);
  const missing = app.output !== "" && !outputs.some((o) => o.name === app.output);

  let status: string;
  if (app.configured && !app.chain_ready) status = "Démarrage de la chaîne…";
  else if (app.playing) status = "En lecture";
  else if (app.streams > 0) status = "En pause";
  else status = "Inactive";

  return (
    <div
      className={`strip lx-strip ${app.streams === 0 ? "strip-dormant" : ""} ${app.muted ? "strip-muted" : ""} ${app.playing ? "lx-playing" : ""}`}
      style={{ "--accent": app.color } as React.CSSProperties}
    >
      <div className="strip-cap" />
      <div className="strip-glow" />

      <div className="strip-head">
        <span className="strip-icon">
          {app.icon ? <img className="lx-app-icon" src={app.icon} alt="" /> : pickIcon(app.name)}
        </span>
        <span className="lx-app-name" title={`${app.name} (${app.key})`}>
          {app.name}
        </span>
        <button
          className={`btn-mute btn-head ${app.muted ? "on" : ""}`}
          title={app.muted ? "Réactiver" : "Couper (mute)"}
          onClick={() => onMute(!app.muted)}
        >
          {app.muted ? <SpeakerOffIcon /> : <SpeakerIcon />}
        </button>
        {app.configured && (
          <button
            className="btn-remove"
            title="Oublier cette appli (supprime ses réglages, retour à la sortie par défaut)"
            onClick={onForget}
          >
            ×
          </button>
        )}
      </div>

      <div className="lx-status">
        <span className={`lx-dot ${app.playing ? "on" : ""}`} />
        {status}
        {!app.configured && <span className="lx-badge">non réglée</span>}
      </div>

      <label className="strip-label">Sortie</label>
      <select
        className="device-select"
        value={app.output}
        title="Sortie audio de cette appli"
        onChange={(e) => onOutput(e.target.value)}
      >
        <option value="">Par défaut — {defaultOutputLabel}</option>
        {outputs.map((o) => (
          <option key={o.name} value={o.name}>
            {o.bluetooth ? "Bluetooth · " : ""}
            {o.label}
          </option>
        ))}
        {missing && <option value={app.output}>{app.output} (déconnectée)</option>}
      </select>

      <button
        className="strip-eq-mini"
        title="Égaliseur — cliquer pour éditer la courbe"
        onClick={onOpenEq}
      >
        <span className="strip-eq-head">
          Égaliseur
          <ChevronRightIcon />
        </span>
        <svg className="strip-eq-curve" viewBox="0 0 130 40" preserveAspectRatio="none" aria-hidden>
          <line x1={0} y1={20} x2={130} y2={20} className="eq-grid-zero" />
          <polyline points={miniCurve} className="eq-line" />
        </svg>
      </button>

      <div className="strip-fader-row">
        <LevelBars kind="lines" id={app.key} idle={app.streams > 0 && !app.muted} />
        <div className="strip-main">
          <VSlider
            variant="fader"
            min={0}
            max={1.5}
            resetValue={1}
            value={app.gain}
            accent={app.color}
            onChange={onGain}
            title="Volume (double-clic : 100 %)"
          />
        </div>
        <span className="strip-value">{Math.round(app.gain * 100)}%</span>
      </div>
    </div>
  );
}

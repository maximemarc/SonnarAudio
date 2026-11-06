/**
 * Tranche « Général » (Linux) : le son de tout le système, c'est-à-dire la
 * sortie par défaut. Volume et mute de cette sortie, choix de la sortie par
 * défaut, et VU réel (crête du moniteur de la sortie, voir `meter.rs`).
 *
 * Toujours affichée, même quand aucune appli ne joue : c'est le réglage de
 * base, celui du bouton de volume du bureau.
 */

import LevelBars from "../components/LevelBars";
import VSlider from "../components/VSlider";
import { MasterIcon, SpeakerIcon, SpeakerOffIcon } from "../components/Icons";
import { useLiveLevel } from "../useLiveLevel";
import type { MasterView, OutputView } from "./types";

const ACCENT = "#5b8def";

interface Props {
  master: MasterView | null;
  outputs: OutputView[];
  onVolume: (volume: number) => void;
  onMute: (muted: boolean) => void;
  onDefaultOutput: (output: string) => void;
}

export default function GeneralStrip({
  master,
  outputs,
  onVolume,
  onMute,
  onDefaultOutput,
}: Props) {
  const glowRef = useLiveLevel("outputs", "*");
  const volume = master?.volume ?? null;
  const muted = master?.muted ?? false;

  return (
    <div
      ref={glowRef}
      className={`strip strip-master lx-strip ${muted ? "strip-muted" : ""}`}
      style={{ "--accent": ACCENT } as React.CSSProperties}
    >
      <div className="strip-cap strip-cap-bus" />
      <div className="strip-glow" />

      <div className="strip-head">
        <span className="strip-icon">
          <MasterIcon />
        </span>
        <span className="lx-app-name" title="Tout le son du système">
          Son général
        </span>
        <button
          className={`btn-mute btn-head ${muted ? "on" : ""}`}
          title={muted ? "Réactiver le son" : "Couper tout le son"}
          disabled={!master}
          onClick={() => onMute(!muted)}
        >
          {muted ? <SpeakerOffIcon /> : <SpeakerIcon />}
        </button>
      </div>

      <div className="lx-status">
        <span className={`lx-dot ${master ? "on" : ""}`} />
        {master ? "Sortie par défaut" : "Aucune sortie détectée"}
      </div>

      <label className="strip-label">Sortie par défaut</label>
      <select
        className="device-select"
        value={master?.name ?? ""}
        title="Sortie utilisée par le système et les applis non réglées"
        disabled={outputs.length === 0}
        onChange={(e) => onDefaultOutput(e.target.value)}
      >
        {!master && <option value="">—</option>}
        {outputs.map((o) => (
          <option key={o.name} value={o.name}>
            {o.bluetooth ? "Bluetooth · " : ""}
            {o.label}
          </option>
        ))}
      </select>

      <div className="strip-fader-row">
        <LevelBars kind="outputs" id="*" idle={!!master && !muted} />
        <div className="strip-main">
          {volume === null ? (
            <p className="panel-hint">Volume non réglable sur cette sortie.</p>
          ) : (
            <VSlider
              variant="fader"
              min={0}
              max={1}
              resetValue={1}
              value={volume}
              accent={ACCENT}
              onChange={onVolume}
              title="Volume général (double-clic : 100 %)"
            />
          )}
        </div>
        <span className="strip-value strip-value-master">
          {volume === null ? "—" : `${Math.round(volume * 100)}%`}
        </span>
      </div>
    </div>
  );
}

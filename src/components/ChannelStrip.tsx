/**
 * One line as a Sonar-style channel strip. Two modes:
 * - "app": fed by application audio through its auto-bound virtual cable.
 *   No device dropdown — just a status row; apps arrive by drag-and-drop
 *   (or via the dock's "Router vers" menu).
 * - "mic": fed by a physical capture device, picked in a mics-only select.
 *
 * Either mode can fan out to SEVERAL physical outputs at once (e.g. monitor
 * on headphones AND speakers simultaneously) — the "Sortie" section shows a
 * chip per selected device plus a dropdown to add another.
 */

import { useEffect, useMemo, useState } from "react";
import type { LineConfig } from "../types";
import { isVirtualDevice } from "../types";
import { curvePoints } from "../eqMath";
import { useLiveLevel } from "../useLiveLevel";
import DeviceSelect from "./DeviceSelect";
import VSlider from "./VSlider";
import {
  ChevronRightIcon,
  HeadphonesIcon,
  LinkIcon,
  MicIcon,
  SpeakerIcon,
  SpeakerOffIcon,
} from "./Icons";
import { pickIcon } from "./pickIcon";

/** "Haut-parleurs (Creative Pebble X Plus)" -> "Haut-parleurs". */
const shortDeviceName = (d: string) => d.split(" (")[0];

interface Props {
  line: LineConfig;
  inputDevices: string[];
  /** Physical render devices (headphones / speakers) — the full catalog. */
  outputDevices: string[];
  /** Devices this channel currently plays to (fan-out; [] = nowhere). */
  selectedOutputs: string[];
  onAddOutput: (device: string) => void;
  onRemoveOutput: (device: string) => void;
  /** Sortie liée à d'autres canaux : ajouter une sortie ici l'ajoute aussi chez eux. */
  synced: boolean;
  onToggleSync: () => void;
  syncPartnerNames: string[];
  /** device -> gain [0..1.5] for THIS line's routes (fan-out balance). */
  outputGains: Record<string, number>;
  onSetOutputGain: (device: string, gain: number) => void;
  onSetInput: (device: string | null) => void;
  onGain: (gain: number) => void;
  onMute: (muted: boolean) => void;
  /** Ouvre l'onglet Égaliseur sur ce canal. */
  onOpenEq: () => void;
  onRename: (name: string) => void;
  onRemove: () => void;
  /** app mode only. */
  onDropApp?: (exe: string) => void;
  onRemoveApp?: (exe: string) => void;
  /** Exécutables actuellement détectés (les apps fermées sont masquées). */
  runningExes?: string[];
  /** exe (minuscules) -> icône `data:image/bmp;base64,...`, quand connue. */
  appIcons?: Record<string, string>;
}

export default function ChannelStrip({
  line,
  inputDevices,
  outputDevices,
  selectedOutputs,
  onAddOutput,
  onRemoveOutput,
  synced,
  onToggleSync,
  syncPartnerNames,
  outputGains,
  onSetOutputGain,
  onSetInput,
  onGain,
  onMute,
  onOpenEq,
  onRename,
  onRemove,
  onDropApp,
  onRemoveApp,
  runningExes = [],
  appIcons = {},
}: Props) {
  const [name, setName] = useState(line.name);
  const [dragOver, setDragOver] = useState(false);
  useEffect(() => setName(line.name), [line.name]);
  const isMic = line.kind === "mic";
  const active = line.input_device !== null;
  const hasCable = !isMic && line.input_device !== null && isVirtualDevice(line.input_device);
  const glowRef = useLiveLevel("lines", line.id);
  const miniCurve = useMemo(() => curvePoints(line.eq_bands ?? [], 130, 40, 60), [line.eq_bands]);
  // Les assignations restent en config, mais seules les apps en cours
  // d'exécution sont affichées sur la tranche.
  const visibleApps = line.apps.filter((e) => runningExes.includes(e.toLowerCase()));
  const addableOutputs = outputDevices.filter((d) => !selectedOutputs.includes(d));

  return (
    <div
      ref={glowRef}
      className={`strip ${active ? "" : "strip-dormant"} ${line.muted ? "strip-muted" : ""} ${dragOver ? "strip-drop" : ""}`}
      style={{ "--accent": line.color } as React.CSSProperties}
      onDragOver={(e) => {
        if (!isMic && e.dataTransfer.types.includes("mixflow/app")) {
          e.preventDefault();
          e.dataTransfer.dropEffect = "copy";
          setDragOver(true);
        }
      }}
      onDragLeave={() => setDragOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        setDragOver(false);
        const exe = e.dataTransfer.getData("mixflow/app");
        if (exe && onDropApp) onDropApp(exe);
      }}
    >
      <div className="strip-cap" />
      <div className="strip-glow" />

      <div className="strip-head">
        <span className="strip-icon">{isMic ? <MicIcon /> : pickIcon(line.name)}</span>
        <input
          className="name-input"
          value={name}
          placeholder="Nom"
          title="Cliquer pour renommer (le périphérique Windows suit)"
          onChange={(e) => setName(e.target.value)}
          onBlur={() => name.trim() && name !== line.name && onRename(name.trim())}
          onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
          spellCheck={false}
        />
        {line.muted && <span className="muted-tag">Muet</span>}
        {isMic ? (
          // Retour micro : s'entendre (ou non) dans la sortie choisie.
          <button
            className={`btn-monitor btn-head ${line.muted ? "" : "on"}`}
            title={
              line.muted
                ? "Activer le retour micro (s'entendre dans la sortie)"
                : "Couper le retour micro"
            }
            onClick={() => onMute(!line.muted)}
          >
            <HeadphonesIcon />
          </button>
        ) : (
          <button
            className={`btn-mute btn-head ${line.muted ? "on" : ""}`}
            title={line.muted ? "Réactiver" : "Couper (mute)"}
            onClick={() => onMute(!line.muted)}
          >
            {line.muted ? <SpeakerOffIcon /> : <SpeakerIcon />}
          </button>
        )}
        <button className="btn-remove" title="Supprimer" onClick={onRemove}>
          ×
        </button>
      </div>

      {isMic && (
        <DeviceSelect
          devices={inputDevices}
          value={line.input_device ?? ""}
          placeholder="— choisir un micro —"
          role="mic"
          onChange={(d) => onSetInput(d === "" ? null : d)}
        />
      )}
      {/* Le câble lié est de la plomberie : on ne l'affiche que s'il manque. */}
      {!isMic && !hasCable && (
        <div
          className="strip-status"
          title="Aucun câble virtuel libre — installe VB-Cable A+B ou libère un canal"
        >
          en attente d'un câble…
        </div>
      )}

      <div className="strip-outputs-head">
        <label className="strip-label">Sortie{selectedOutputs.length > 1 ? "s" : ""}</label>
        <button
          type="button"
          className={`sync-btn ${synced ? "on" : ""}`}
          title="Synchroniser la sortie avec d'autres canaux"
          onClick={onToggleSync}
        >
          <LinkIcon />
          Sync
        </button>
      </div>
      <div className="strip-outputs">
        {selectedOutputs.length === 0 && <span className="apps-hint">Aucune sortie</span>}
        {selectedOutputs.map((d) => (
          <span className="strip-app" key={d} title={d}>
            {shortDeviceName(d)}
            <button
              className="strip-app-x"
              title="Retirer cette sortie"
              onClick={() => onRemoveOutput(d)}
            >
              ×
            </button>
          </span>
        ))}
      </div>
      <DeviceSelect
        devices={addableOutputs}
        value=""
        placeholder={
          selectedOutputs.length === 0 ? "— choisir casque / HP —" : "— ajouter une sortie —"
        }
        role="speaker"
        onChange={(d) => d && onAddOutput(d)}
      />
      {/* Équilibre entre sorties (casque vs enceintes, ou mix perso vs
          stream) — n'a de sens qu'à partir de 2 sorties simultanées. */}
      {selectedOutputs.length > 1 && (
        <div className="route-gains">
          {selectedOutputs.map((d) => (
            <div className="route-gain-row" key={d}>
              <span className="route-gain-label" title={d}>
                {shortDeviceName(d)}
              </span>
              <input
                type="range"
                min={0}
                max={150}
                step={5}
                value={Math.round((outputGains[d] ?? 1) * 100)}
                onChange={(e) => onSetOutputGain(d, Number(e.target.value) / 100)}
              />
              <span className="route-gain-value">{Math.round((outputGains[d] ?? 1) * 100)}%</span>
            </div>
          ))}
        </div>
      )}
      {syncPartnerNames.length > 0 && (
        <div className="sync-hint">Synchronisé avec {syncPartnerNames.join(", ")}</div>
      )}

      {/* Aperçu de la courbe EQ — clic = ouvre l'onglet Égaliseur. */}
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

      <div className="strip-main">
        <VSlider
          variant="fader"
          min={0}
          max={1.5}
          resetValue={1}
          value={line.gain}
          accent={line.color}
          meter={{ kind: "lines", id: line.id }}
          onChange={onGain}
          title="Volume (double-clic : 100 %)"
        />
      </div>

      <div className="strip-foot">
        <span className="strip-value">{Math.round(line.gain * 100)}%</span>
      </div>

      {!isMic && (
        <div className="strip-apps-zone">
          <span className="strip-label">Applications</span>
          <div className="strip-apps">
            {visibleApps.length === 0 && <span className="apps-hint">Déposer une app ici</span>}
            {visibleApps.map((exe) => (
              <span
                className="strip-app"
                key={exe}
                title={`${exe} — glisser vers un autre canal pour déplacer`}
                draggable
                onDragStart={(e) => {
                  e.dataTransfer.setData("mixflow/app", exe);
                  e.dataTransfer.effectAllowed = "copy";
                }}
              >
                {appIcons[exe.toLowerCase()] && (
                  <img className="app-icon" src={appIcons[exe.toLowerCase()]} alt="" />
                )}
                {exe.replace(/\.exe$/i, "")}
                <button
                  className="strip-app-x"
                  title="Rendre l'app à la sortie par défaut"
                  onClick={() => onRemoveApp?.(exe)}
                >
                  ×
                </button>
              </span>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

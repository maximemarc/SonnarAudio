/**
 * Mix profiles: snapshot the current state (per-line gain/mute/EQ/outputs +
 * ducking + master) under a name, optionally bound to an app — the backend's
 * profile-watch thread auto-applies it when that app gets focus.
 */

import { useState } from "react";
import type { AppConfig, AppInfo } from "../types";

interface Props {
  config: AppConfig;
  apps: AppInfo[];
  onSave: (name: string) => void;
  onApply: (id: string) => void;
  onDelete: (id: string) => void;
  onSetTrigger: (id: string, triggerExe: string | null) => void;
}

export default function ProfilesPanel({
  config,
  apps,
  onSave,
  onApply,
  onDelete,
  onSetTrigger,
}: Props) {
  const [name, setName] = useState("");

  return (
    <div className="profiles">
      {config.profiles.length === 0 && (
        <p className="empty-hint">
          Aucun profil. Sauvegarde l'état actuel (sorties, gains, EQ, priorité) puis lie-le à un jeu
          pour qu'il s'applique tout seul quand ce jeu passe au premier plan.
        </p>
      )}
      {config.profiles.map((p) => (
        <div className="profile-row" key={p.id}>
          <span className="profile-name">{p.name}</span>
          <select
            className="device-select"
            value={p.trigger_exe ?? ""}
            title="Application déclenchant l'application automatique de ce profil"
            onChange={(e) => onSetTrigger(p.id, e.target.value || null)}
          >
            <option value="">— pas de déclencheur —</option>
            {apps.map((a) => (
              <option key={a.exe} value={a.exe}>
                {a.label}
              </option>
            ))}
          </select>
          <button className="btn-ghost" onClick={() => onApply(p.id)}>
            Appliquer
          </button>
          <button className="btn-remove" title="Supprimer ce profil" onClick={() => onDelete(p.id)}>
            ×
          </button>
        </div>
      ))}
      <div className="profile-add">
        <input
          className="eq-save-name"
          placeholder="Nom du profil…"
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && name.trim()) {
              onSave(name.trim());
              setName("");
            }
          }}
          spellCheck={false}
        />
        <button
          className="btn-add"
          disabled={!name.trim()}
          onClick={() => {
            onSave(name.trim());
            setName("");
          }}
        >
          + Sauvegarder l'état actuel
        </button>
      </div>
    </div>
  );
}

/**
 * MixFlow — console de mixage.
 *
 * State strategy:
 * - `config` mirrors the backend AppConfig. Topology commands return the
 *   fresh config (backend owns the ids); live commands are applied
 *   optimistically here and fire-and-forget to the backend.
 * - VU levels bypass React entirely (see levels.ts / VSlider.tsx's meter prop).
 */

import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import * as api from "./api";
import { initLevels } from "./levels";
import type { AppConfig, AppInfo, DeviceList, DuckRule, EngineStatus } from "./types";
import { LINE_COLORS } from "./types";
import ChannelStrip from "./components/ChannelStrip";
import MasterStrip from "./components/MasterStrip";
import ChatMix from "./components/ChatMix";
import DuckingPanel from "./components/DuckingPanel";
import EqPage from "./components/EqPage";
import ProfilesPanel from "./components/ProfilesPanel";
import DeviceSelect from "./components/DeviceSelect";
import logo from "./assets/logo.png";

export default function App() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [devices, setDevices] = useState<DeviceList>({ inputs: [], outputs: [] });
  const [apps, setApps] = useState<AppInfo[]>([]);
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<"console" | "eq">("console");
  const [eqLineId, setEqLineId] = useState<string>("");
  // Canaux dont la sortie est "liée" : ajouter une sortie à l'un l'ajoute
  // aussi aux autres (pratique pour rebasculer plusieurs canaux d'un coup
  // vers un nouveau casque). Purement une commodité d'UI, pas persisté.
  const [syncedLines, setSyncedLines] = useState<Set<string>>(() => new Set());
  const [autostart, setAutostart] = useState(false);
  // Live from the backend's health-check thread — a configured device that
  // vanished mid-session (see main.rs's "mixflow-health" thread).
  const [deviceWarnings, setDeviceWarnings] = useState<string[]>([]);

  const refreshApps = useCallback(() => {
    api
      .listApps()
      .then(setApps)
      .catch((e) => setError(`Détection des applications : ${e}`));
  }, []);

  useEffect(() => {
    initLevels();
    void api.getConfig().then(setConfig);
    void api.listDevices().then(setDevices);
    void api.getAutostartEnabled().then(setAutostart);
    // Config illisible mise de côté au démarrage, etc. — sans ça le message
    // ne partait qu'en stderr, invisible dans le binaire de release.
    void api.takeStartupNotice().then((n) => n && setError(n));
    refreshApps();
    // Apps come and go — light periodic rescan + rescan when the window
    // regains focus (the user just launched something).
    const t = setInterval(refreshApps, 10_000);
    window.addEventListener("focus", refreshApps);
    const unlisten = listen<EngineStatus>("engine_status", (e) => setStatus(e.payload));
    // Config changes that don't originate from this window — the global
    // mic-mute hotkey, or a profile the backend auto-switched to.
    const unlistenCfg = listen<AppConfig>("config_updated", (e) => setConfig(e.payload));
    const unlistenWarn = listen<string[]>("device_warnings", (e) => setDeviceWarnings(e.payload));
    return () => {
      clearInterval(t);
      window.removeEventListener("focus", refreshApps);
      void unlisten.then((f) => f());
      void unlistenCfg.then((f) => f());
      void unlistenWarn.then((f) => f());
    };
  }, [refreshApps]);

  const refreshDevices = useCallback(() => {
    void api.listDevices().then(setDevices);
  }, []);

  // Optimistic local patch helper for live params.
  const patch = useCallback((fn: (cfg: AppConfig) => AppConfig) => {
    setConfig((c) => (c ? fn(structuredClone(c)) : c));
  }, []);

  if (!config) {
    return <div className="boot">Chargement…</div>;
  }

  const appLines = config.lines.filter((l) => l.kind !== "mic");
  const micLines = config.lines.filter((l) => l.kind === "mic");
  const appIcons: Record<string, string> = {};
  for (const a of apps) {
    if (a.icon) appIcons[a.exe.toLowerCase()] = a.icon;
  }

  // Périphériques physiques vers lesquels joue une ligne (fan-out possible,
  // via ses bus cachés).
  const outputDevicesOf = (lineId: string): string[] => {
    const line = config.lines.find((l) => l.id === lineId);
    if (!line) return [];
    return line.routes
      .map((r) => config.outputs.find((o) => o.id === r.output_id)?.device)
      .filter((d): d is string => !!d);
  };

  const addOutputTo = (lineId: string, device: string) => {
    const next = Array.from(new Set([...outputDevicesOf(lineId), device]));
    void api.setLineOutputs(lineId, next).then(setConfig);
    // Sync : propager le nouvel appareil aux autres canaux liés (sans
    // toucher à leurs autres sorties existantes).
    if (syncedLines.has(lineId)) {
      for (const otherId of syncedLines) {
        if (otherId === lineId) continue;
        const otherOutputs = outputDevicesOf(otherId);
        if (otherOutputs.includes(device)) continue;
        void api.setLineOutputs(otherId, [...otherOutputs, device]).then(setConfig);
      }
    }
  };
  const removeOutputFrom = (lineId: string, device: string) => {
    const next = outputDevicesOf(lineId).filter((d) => d !== device);
    void api.setLineOutputs(lineId, next).then(setConfig);
  };
  const toggleSync = (lineId: string) => {
    setSyncedLines((prev) => {
      const next = new Set(prev);
      if (next.has(lineId)) next.delete(lineId);
      else next.add(lineId);
      return next;
    });
  };
  const syncPartnerNames = (lineId: string): string[] =>
    syncedLines.has(lineId)
      ? Array.from(syncedLines)
          .filter((id) => id !== lineId)
          .map((id) => config.lines.find((l) => l.id === id)?.name)
          .filter((n): n is string => !!n)
      : [];
  const removeLine = (lineId: string) => {
    setSyncedLines((prev) => {
      if (!prev.has(lineId)) return prev;
      const next = new Set(prev);
      next.delete(lineId);
      return next;
    });
    void api.removeLine(lineId).then(setConfig);
  };
  const toggleAutostart = () => {
    const next = !autostart;
    setAutostart(next); // optimiste : rollback si Windows refuse (rare, pas besoin d'admin)
    api.setAutostartEnabled(next).catch((e) => {
      setAutostart(!next);
      setError(String(e));
    });
  };

  // Périphérique -> gain [0..1.5] pour CETTE ligne (équilibre entre sorties).
  const outputGainsOf = (lineId: string): Record<string, number> => {
    const line = config.lines.find((l) => l.id === lineId);
    if (!line) return {};
    const map: Record<string, number> = {};
    for (const r of line.routes) {
      const dev = config.outputs.find((o) => o.id === r.output_id)?.device;
      if (dev) map[dev] = r.gain;
    }
    return map;
  };
  const setOutputGain = (lineId: string, device: string, gain: number) => {
    const line = config.lines.find((l) => l.id === lineId);
    const outputId = line?.routes.find(
      (r) => config.outputs.find((o) => o.id === r.output_id)?.device === device,
    )?.output_id;
    if (!outputId) return;
    patch((c) => {
      const route = c.lines
        .find((x) => x.id === lineId)
        ?.routes.find((r) => r.output_id === outputId);
      if (route) route.gain = gain;
      return c;
    });
    void api.setRouteGain(lineId, outputId, gain);
  };

  const setSourceReactivity = (lineId: string, level: "douce" | "normale" | "rapide") => {
    patch((c) => {
      const l = c.lines.find((x) => x.id === lineId);
      if (l) l.duck_reactivity = level;
      return c;
    });
    void api.setLineDuckReactivity(lineId, level);
  };

  // -- profils, export/import, mode streamer, mises à jour --------------------

  const handleSaveProfile = (name: string) => void api.saveProfile(name, null).then(setConfig);
  const handleApplyProfile = (id: string) =>
    api
      .applyProfile(id)
      .then(setConfig)
      .catch((e) => setError(String(e)));
  const handleDeleteProfile = (id: string) => void api.deleteProfile(id).then(setConfig);
  const handleSetProfileTrigger = (id: string, triggerExe: string | null) =>
    void api.setProfileTrigger(id, triggerExe).then(setConfig);

  const handleExportConfig = () => {
    api
      .exportConfig()
      .then((path) => path && setError(`Configuration exportée : ${path}`))
      .catch((e) => setError(String(e)));
  };
  const handleImportConfig = () => {
    setError(null);
    api
      .importConfig()
      .then((cfg) => {
        // null = import annulé (dialogue fermé ou confirmation refusée).
        if (!cfg) return;
        setConfig(cfg);
        refreshDevices();
        refreshApps();
        setError(
          "Configuration importée. L'ancienne a été sauvegardée à côté du fichier de config.",
        );
      })
      .catch((e) => setError(String(e)));
  };

  const handleEnableStreamer = (device: string) =>
    void api.enableStreamerMode(device).then(setConfig);

  const handleCheckUpdate = () => {
    api
      .checkForUpdate()
      .then((v) => setError(v ? `Mise à jour disponible : v${v}` : "MixFlow est à jour."))
      .catch((e) => setError(String(e)));
  };

  // "Router vers" (dock) et drag-and-drop passent tous deux par ici.
  const assignAppTo = (lineId: string, exe: string) => {
    setError(null);
    api
      .assignApp(lineId, exe)
      .then((res) => {
        setConfig(res.config);
        if (res.notice) setError(res.notice);
        refreshApps();
        refreshDevices(); // le câble vient d'être renommé
      })
      .catch((e) => setError(String(e)));
  };

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <img className="brand-mark" src={logo} alt="" />
          MixFlow
        </div>
        <nav className="tabs">
          <button
            className={`tab ${view === "console" ? "on" : ""}`}
            onClick={() => setView("console")}
          >
            Console
          </button>
          <button className={`tab ${view === "eq" ? "on" : ""}`} onClick={() => setView("eq")}>
            Égaliseur
          </button>
        </nav>
        <div className="topbar-right">
          <span
            className={`engine-pill ${status && status.warnings.length > 0 ? "warn" : "ok"}`}
            title={status?.warnings.join("\n") || "Moteur audio actif"}
          >
            {status
              ? status.warnings.length > 0
                ? `${status.warnings.length} avertissement(s)`
                : `${status.active_captures} entrée(s) · ${status.active_renders} sortie(s)`
              : "démarrage…"}
          </span>
          <button
            className={`btn-ghost ${autostart ? "on" : ""}`}
            onClick={toggleAutostart}
            title={
              autostart
                ? "Ne plus démarrer avec Windows"
                : "Démarrer automatiquement à l'ouverture de session"
            }
          >
            {autostart ? "✓ Démarrage auto" : "Démarrage auto"}
          </button>
          <button
            className="btn-ghost"
            onClick={refreshDevices}
            title="Re-scanner les périphériques audio"
          >
            ⟳ Périphériques
          </button>
          <DeviceSelect
            devices={devices.outputs}
            value=""
            placeholder="🎥 Mode Streamer…"
            role="stream"
            onChange={(d) => d && handleEnableStreamer(d)}
          />
          <button
            className="btn-ghost"
            onClick={handleExportConfig}
            title="Exporter la configuration"
          >
            Exporter
          </button>
          <button
            className="btn-ghost"
            onClick={handleImportConfig}
            title="Importer une configuration"
          >
            Importer
          </button>
          <button
            className="btn-ghost"
            onClick={handleCheckUpdate}
            title="Vérifier les mises à jour"
          >
            Mises à jour
          </button>
        </div>
      </header>

      {(status?.warnings.length ?? 0) + deviceWarnings.length > 0 && (
        <div className="warn-banner">
          {(status?.warnings ?? []).map((w, i) => (
            <div key={`e${i}`}>{w}</div>
          ))}
          {deviceWarnings.map((w, i) => (
            <div key={`d${i}`}>{w}</div>
          ))}
        </div>
      )}

      {error && (
        <div className="error-banner" onClick={() => setError(null)} title="Cliquer pour fermer">
          {error}
        </div>
      )}

      {view === "eq" ? (
        <main className="content">
          <EqPage
            config={config}
            selectedId={eqLineId || config.lines[0]?.id || ""}
            onSelect={setEqLineId}
            onBands={(lineId, bands) => {
              patch((c) => {
                const l = c.lines.find((x) => x.id === lineId);
                if (l) l.eq_bands = bands.map((b) => ({ ...b }));
                return c;
              });
              void api.setLineEqBands(lineId, bands);
            }}
            onSavePreset={(name, bands) => void api.saveEqPreset(name, bands).then(setConfig)}
            onDeletePreset={(name) => void api.deleteEqPreset(name).then(setConfig)}
          />
        </main>
      ) : (
        <main className="content">
          {/* La console : Master | Canaux | Applications | Micro. */}
          <section className="console">
            <div className="console-group">
              <h2>Master</h2>
              <div className="rail">
                <MasterStrip
                  value={config.master_gain}
                  onChange={(g) => {
                    patch((c) => {
                      c.master_gain = g;
                      return c;
                    });
                    void api.setMasterGain(g);
                  }}
                  unrouted={apps.filter(
                    (a) =>
                      !config.lines.some((l) =>
                        l.apps.some((e) => e.toLowerCase() === a.exe.toLowerCase()),
                      ),
                  )}
                  onRefresh={refreshApps}
                />
              </div>
            </div>

            <div className="console-group">
              <h2>Canaux</h2>
              <div className="rail">
                {appLines.map((line) => (
                  <ChannelStrip
                    key={line.id}
                    line={line}
                    inputDevices={devices.inputs}
                    outputDevices={devices.outputs}
                    selectedOutputs={outputDevicesOf(line.id)}
                    onAddOutput={(d) => addOutputTo(line.id, d)}
                    onRemoveOutput={(d) => removeOutputFrom(line.id, d)}
                    synced={syncedLines.has(line.id)}
                    onToggleSync={() => toggleSync(line.id)}
                    syncPartnerNames={syncPartnerNames(line.id)}
                    outputGains={outputGainsOf(line.id)}
                    onSetOutputGain={(d, g) => setOutputGain(line.id, d, g)}
                    appIcons={appIcons}
                    onSetInput={(d) => void api.setLineInput(line.id, d).then(setConfig)}
                    runningExes={apps.map((a) => a.exe.toLowerCase())}
                    onDropApp={(exe) => assignAppTo(line.id, exe)}
                    onRemoveApp={(exe) => {
                      api
                        .unassignApp(line.id, exe)
                        .then((cfg) => {
                          setConfig(cfg);
                          refreshApps();
                        })
                        .catch((e) => setError(String(e)));
                    }}
                    onGain={(g) => {
                      patch((c) => {
                        const l = c.lines.find((x) => x.id === line.id);
                        if (l) l.gain = g;
                        return c;
                      });
                      void api.setLineGain(line.id, g);
                    }}
                    onMute={(m) => {
                      patch((c) => {
                        const l = c.lines.find((x) => x.id === line.id);
                        if (l) l.muted = m;
                        return c;
                      });
                      void api.setLineMuted(line.id, m);
                    }}
                    onOpenEq={() => {
                      setEqLineId(line.id);
                      setView("eq");
                    }}
                    onRename={(name) => {
                      patch((c) => {
                        const l = c.lines.find((x) => x.id === line.id);
                        if (l) l.name = name;
                        return c;
                      });
                      void api.updateLineMeta(line.id, name, line.color);
                    }}
                    onRemove={() => removeLine(line.id)}
                  />
                ))}
                <button
                  className="strip-add"
                  title="Ajouter un canal d'applications"
                  onClick={() =>
                    void api
                      .addLine(
                        `Canal ${appLines.length + 1}`,
                        LINE_COLORS[config.lines.length % LINE_COLORS.length],
                        "app",
                      )
                      .then(setConfig)
                  }
                >
                  +<small>Canal</small>
                </button>
              </div>
              {/* CHATMIX — équilibre Game/Chat, la signature Sonar. */}
              <ChatMix
                lines={appLines}
                onGain={(lineId, g) => {
                  patch((c) => {
                    const l = c.lines.find((x) => x.id === lineId);
                    if (l) l.gain = g;
                    return c;
                  });
                  void api.setLineGain(lineId, g);
                }}
              />
            </div>

            {/* Section dédiée au(x) micro(s), à droite. */}
            <div className="console-group console-group-mic">
              <h2>Microphone(s)</h2>
              <div className="rail">
                {micLines.map((line) => (
                  <ChannelStrip
                    key={line.id}
                    line={line}
                    inputDevices={devices.inputs}
                    outputDevices={devices.outputs}
                    selectedOutputs={outputDevicesOf(line.id)}
                    onAddOutput={(d) => addOutputTo(line.id, d)}
                    onRemoveOutput={(d) => removeOutputFrom(line.id, d)}
                    synced={syncedLines.has(line.id)}
                    onToggleSync={() => toggleSync(line.id)}
                    syncPartnerNames={syncPartnerNames(line.id)}
                    outputGains={outputGainsOf(line.id)}
                    onSetOutputGain={(d, g) => setOutputGain(line.id, d, g)}
                    appIcons={appIcons}
                    onSetInput={(d) => void api.setLineInput(line.id, d).then(setConfig)}
                    onGain={(g) => {
                      patch((c) => {
                        const l = c.lines.find((x) => x.id === line.id);
                        if (l) l.gain = g;
                        return c;
                      });
                      void api.setLineGain(line.id, g);
                    }}
                    onMute={(m) => {
                      patch((c) => {
                        const l = c.lines.find((x) => x.id === line.id);
                        if (l) l.muted = m;
                        return c;
                      });
                      void api.setLineMuted(line.id, m);
                    }}
                    onOpenEq={() => {
                      setEqLineId(line.id);
                      setView("eq");
                    }}
                    onRename={(name) => {
                      patch((c) => {
                        const l = c.lines.find((x) => x.id === line.id);
                        if (l) l.name = name;
                        return c;
                      });
                      void api.updateLineMeta(line.id, name, line.color);
                    }}
                    onRemove={() => removeLine(line.id)}
                  />
                ))}
                <button
                  className="strip-add"
                  title="Ajouter un micro"
                  onClick={() =>
                    void api.addLine(`Mic ${micLines.length + 1}`, "#fb923c", "mic").then(setConfig)
                  }
                >
                  +<small>Micro</small>
                </button>
              </div>
            </div>
          </section>

          <section className="panel">
            <h2>Priorité — ducking</h2>
            <p className="panel-hint">
              Baisse automatiquement le volume d'un canal quand un autre devient actif — utile pour
              entendre tes amis par-dessus le jeu.
            </p>
            <DuckingPanel
              config={config}
              onChange={(rules: DuckRule[]) => {
                patch((c) => {
                  c.ducking = rules;
                  return c;
                });
                void api.setDuckRules(rules);
              }}
              onSetSourceReactivity={setSourceReactivity}
            />
          </section>

          <section className="panel">
            <h2>Profils</h2>
            <ProfilesPanel
              config={config}
              apps={apps}
              onSave={handleSaveProfile}
              onApply={handleApplyProfile}
              onDelete={handleDeleteProfile}
              onSetTrigger={handleSetProfileTrigger}
            />
          </section>
        </main>
      )}
    </div>
  );
}

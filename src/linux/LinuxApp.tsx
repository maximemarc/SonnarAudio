/**
 * MixFlow sous Linux — mélangeur par application au-dessus de PipeWire.
 *
 * - Onglet Console : le bandeau des sorties (volume, mute, VU, sortie par
 *   défaut), puis une tranche par appli qui joue du son (ou déjà réglée).
 * - Onglet Égaliseur : la page EQ partagée avec Windows, sur la liste des
 *   applis au lieu des canaux.
 *
 * Source de vérité : le backend (`lx_state`, poussé à chaque changement du
 * graphe). Les réglages live sont appliqués ici de façon optimiste.
 */

import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import EqPage from "../components/EqPage";
import type { EqBand } from "../types";
import logo from "../assets/logo.png";
import { initLevels } from "../levels";
import * as api from "./api";
import AppStrip from "./AppStrip";
import OutputsBar from "./OutputsBar";
import type { AppView, LinuxState, OutputView } from "./types";

export default function LinuxApp() {
  const [state, setState] = useState<LinuxState | null>(null);
  const [view, setView] = useState<"console" | "eq">("console");
  const [eqKey, setEqKey] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    initLevels();
    void api.getState().then(setState);
    const unlisten = listen<LinuxState>("lx_state", (e) => setState(e.payload));
    return () => void unlisten.then((f) => f());
  }, []);

  const fail = useCallback((e: unknown) => setError(String(e)), []);

  /** Patch optimiste d'une appli (l'UI réagit sans attendre PipeWire). */
  const patchApp = useCallback((key: string, fn: (a: AppView) => void) => {
    setState((s) => {
      if (!s) return s;
      const next = structuredClone(s);
      const a = next.apps.find((x) => x.key === key);
      if (a) {
        fn(a);
        a.configured = true;
      }
      return next;
    });
  }, []);

  /** Patch optimiste d'une sortie. */
  const patchOutput = useCallback((name: string, fn: (o: OutputView) => void) => {
    setState((s) => {
      if (!s) return s;
      const next = structuredClone(s);
      const o = next.outputs.find((x) => x.name === name);
      if (o) fn(o);
      return next;
    });
  }, []);

  if (!state) return <div className="boot">Chargement…</div>;

  const defaultOut = state.outputs.find((o) => o.name === state.default_output);
  const defaultLabel = defaultOut?.label ?? "sortie du système";
  const active = state.apps.filter((a) => a.streams > 0);
  const idle = state.apps.filter((a) => a.streams === 0);
  const notice = error ?? state.notice;

  const strip = (a: AppView) => (
    <AppStrip
      key={a.key}
      app={a}
      outputs={state.outputs}
      defaultOutputLabel={defaultLabel}
      onGain={(gain) => {
        patchApp(a.key, (x) => (x.gain = gain));
        api.setGain(a.key, gain).catch(fail);
      }}
      onMute={(muted) => {
        patchApp(a.key, (x) => (x.muted = muted));
        api.setMuted(a.key, muted).catch(fail);
      }}
      onOutput={(output) => {
        patchApp(a.key, (x) => (x.output = output));
        api.setOutput(a.key, output).then(setState).catch(fail);
      }}
      onOpenEq={() => {
        setEqKey(a.key);
        setView("eq");
      }}
      onForget={() => api.forgetApp(a.key).then(setState).catch(fail)}
    />
  );

  return (
    <div className="app lx">
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
            className={`engine-pill ${state.pipewire_ok ? "ok" : "warn"}`}
            title={`Sortie par défaut : ${defaultLabel}`}
          >
            {state.pipewire_ok
              ? `PipeWire · ${active.filter((a) => a.playing).length} appli(s) en lecture`
              : "PipeWire indisponible"}
          </span>
        </div>
      </header>

      {notice && (
        <div
          className="notice-banner notice-error"
          title="Cliquer pour fermer"
          onClick={() => {
            setError(null);
            void api.dismissNotice();
            setState((s) => (s ? { ...s, notice: null } : s));
          }}
        >
          {notice}
        </div>
      )}

      {view === "eq" ? (
        <main className="content">
          {state.apps.length === 0 ? (
            <p className="empty-hint">Aucune application à égaliser pour l'instant.</p>
          ) : (
            <EqPage
              channels={state.apps.map((a) => ({ ...a, id: a.key }))}
              customPresets={state.eq_presets}
              selectedId={eqKey || state.apps[0].key}
              onSelect={setEqKey}
              onBands={(key: string, bands: EqBand[]) => {
                patchApp(key, (x) => (x.eq_bands = bands.map((b) => ({ ...b }))));
                api.setEq(key, bands).catch(fail);
              }}
              onSavePreset={(name, bands) => api.savePreset(name, bands).then(setState).catch(fail)}
              onDeletePreset={(name) => api.deletePreset(name).then(setState).catch(fail)}
            />
          )}
        </main>
      ) : (
        <main className="content">
          <section className="console">
            <div className="console-group lx-full">
              <h2>Sorties</h2>
              <OutputsBar
                outputs={state.outputs}
                onVolume={(name, v) => {
                  patchOutput(name, (o) => (o.volume = v));
                  api.setOutputVolume(name, v).catch(fail);
                }}
                onMute={(name, muted) => {
                  patchOutput(name, (o) => (o.muted = muted));
                  api.setOutputMuted(name, muted).catch(fail);
                }}
                onMakeDefault={(name) => api.setDefaultOutput(name).then(setState).catch(fail)}
              />
            </div>
            <div className="console-group">
              <h2>Applications</h2>
              {active.length === 0 ? (
                <p className="panel-hint">
                  Aucune application n'a de son ouvert. Lance une musique, une vidéo ou un jeu :
                  elle apparaîtra ici. Chaque appli a son volume, sa sortie et son égaliseur ; tes
                  réglages sont gardés d'une session à l'autre.
                </p>
              ) : (
                <div className="rail">{active.map(strip)}</div>
              )}
            </div>
            {idle.length > 0 && (
              <div className="console-group">
                <h2>Réglées, inactives</h2>
                <div className="rail">{idle.map(strip)}</div>
              </div>
            )}
          </section>
        </main>
      )}
    </div>
  );
}

/**
 * Prioritization ("the Sonar feature"): sidechain ducking rules.
 * "While SOURCE is talking, lower TARGET by AMOUNT."
 * Every edit sends the complete rule list — the backend swaps it atomically.
 */

import type { AppConfig, DuckRule } from "../types";

interface Props {
  config: AppConfig;
  onChange: (rules: DuckRule[]) => void;
  /** Reactivity is a property of the SOURCE line, not the rule. */
  onSetSourceReactivity: (lineId: string, level: "douce" | "normale" | "rapide") => void;
}

export default function DuckingPanel({ config, onChange, onSetSourceReactivity }: Props) {
  const { lines, ducking } = config;
  const lineName = (id: string) => lines.find((l) => l.id === id)?.name ?? "?";

  const update = (index: number, patch: Partial<DuckRule>) => {
    const next = ducking.map((r, i) => (i === index ? { ...r, ...patch } : r));
    onChange(next);
  };

  const addRule = () => {
    if (lines.length < 2) return;
    // Default: second line ducks the first — the classic Chat-over-Game.
    onChange([...ducking, { source_line: lines[1].id, target_line: lines[0].id, amount: 0.5 }]);
  };

  return (
    <div className="ducking">
      {ducking.length === 0 && (
        <p className="empty-hint">
          Aucune règle de priorité. Exemple : « quand Chat est actif, baisser Game de 50 % ».
        </p>
      )}

      {ducking.map((rule, i) => (
        <div className="duck-rule" key={i}>
          <span className="duck-label">Quand</span>
          <select
            value={rule.source_line}
            onChange={(e) => update(i, { source_line: e.target.value })}
          >
            {lines.map((l) => (
              <option key={l.id} value={l.id}>
                {l.name}
              </option>
            ))}
          </select>
          <span className="duck-label">est actif, baisser</span>
          <select
            value={rule.target_line}
            onChange={(e) => update(i, { target_line: e.target.value })}
          >
            {lines
              .filter((l) => l.id !== rule.source_line)
              .map((l) => (
                <option key={l.id} value={l.id}>
                  {l.name}
                </option>
              ))}
            {/* Keep an invalid selection visible instead of hiding it. */}
            {rule.target_line === rule.source_line && (
              <option value={rule.target_line}>{lineName(rule.target_line)}</option>
            )}
          </select>
          <span className="duck-label">de</span>
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={rule.amount}
            onChange={(e) => update(i, { amount: Number(e.target.value) })}
          />
          <span className="duck-amount">{Math.round(rule.amount * 100)}%</span>
          <select
            className="duck-reactivity"
            title="Réactivité — vitesse à laquelle le duck relâche quand la source se tait"
            value={lines.find((l) => l.id === rule.source_line)?.duck_reactivity ?? "normale"}
            onChange={(e) =>
              onSetSourceReactivity(
                rule.source_line,
                e.target.value as "douce" | "normale" | "rapide",
              )
            }
          >
            <option value="douce">Douce</option>
            <option value="normale">Normale</option>
            <option value="rapide">Rapide</option>
          </select>
          <button
            className="btn-remove"
            title="Supprimer la règle"
            onClick={() => onChange(ducking.filter((_, j) => j !== i))}
          >
            ×
          </button>
        </div>
      ))}

      <button className="btn-add" onClick={addRule} disabled={lines.length < 2}>
        + Règle de priorité
      </button>
    </div>
  );
}

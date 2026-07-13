/**
 * CHATMIX — the Sonar signature balance slider between the Game and Chat
 * lines. Center = both at 100%. Slide toward Chat → Game is progressively
 * lowered (and vice-versa). Double-click recenters.
 *
 * The slider drives the two lines' regular gains through the same live
 * command as their faders, so everything stays consistent and persisted.
 */

import { useRef, useState } from "react";
import type { LineConfig } from "../types";
import { ChatIcon, GamepadIcon } from "./Icons";

interface Props {
  lines: LineConfig[];
  onGain: (lineId: string, gain: number) => void;
}

/** Find the Game/Chat pair: by name first, else the first two lines. */
function pickPair(lines: LineConfig[]): [LineConfig, LineConfig] | null {
  if (lines.length < 2) return null;
  const game = lines.find((l) => /game|jeu/i.test(l.name)) ?? lines[0];
  const chat =
    lines.find((l) => l.id !== game.id && /chat|voc|discord/i.test(l.name)) ??
    lines.find((l) => l.id !== game.id)!;
  return [game, chat];
}

const THROTTLE_MS = 50;

export default function ChatMix({ lines, onGain }: Props) {
  // v ∈ [-1, 1] : -1 = full Game, 0 = balanced, +1 = full Chat.
  const [v, setV] = useState(0);
  const lastSent = useRef(0);

  const pair = pickPair(lines);
  if (!pair) return null;
  const [game, chat] = pair;

  const apply = (value: number, force: boolean) => {
    setV(value);
    const now = performance.now();
    if (!force && now - lastSent.current < THROTTLE_MS) return;
    lastSent.current = now;
    // Crossfade: the side you move away from is attenuated, never boosted.
    onGain(game.id, Math.min(1, 1 - Math.max(0, value)));
    onGain(chat.id, Math.min(1, 1 + Math.min(0, value)));
  };

  return (
    <div className="chatmix" title="ChatMix — équilibre Game / Chat (double-clic : centre)">
      <span className="chatmix-icon" style={{ color: game.color }}>
        <GamepadIcon />
      </span>
      <div className="chatmix-slider">
        <span className="chatmix-label">CHATMIX</span>
        <input
          type="range"
          min={-1}
          max={1}
          step={0.02}
          value={v}
          onChange={(e) => apply(Number(e.target.value), false)}
          onPointerUp={() => apply(v, true)}
          onDoubleClick={() => apply(0, true)}
        />
      </div>
      <span className="chatmix-icon" style={{ color: chat.color }}>
        <ChatIcon />
      </span>
    </div>
  );
}

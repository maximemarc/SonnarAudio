/**
 * CHATMIX — the Sonar signature balance slider between the Game and Chat
 * lines. Center = both at 100%. Slide toward Chat → Game is progressively
 * lowered (and vice-versa). Double-click recenters.
 *
 * The slider drives the two lines' regular gains through the same live
 * command as their faders, so everything stays consistent and persisted.
 */

import { useEffect, useRef, useState } from "react";
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

/**
 * Position du curseur DÉDUITE des gains réels : c'est l'inverse exact du
 * crossfade appliqué par `apply` ci-dessous. Sans ça, l'état local partait
 * du centre à chaque remontage du composant (un aller-retour sur l'onglet
 * Égaliseur démonte la Console) et le premier mouvement réécrivait des
 * gains absolus, détruisant tout réglage fait au fader, par un profil ou
 * par un import.
 */
function balanceOf(game: LineConfig, chat: LineConfig): number {
  // apply(v>0) : game = 1-v, chat = 1   |   apply(v<0) : game = 1, chat = 1+v
  if (game.gain < chat.gain) return Math.min(1, 1 - game.gain);
  if (chat.gain < game.gain) return Math.max(-1, chat.gain - 1);
  return 0;
}

export default function ChatMix({ lines, onGain }: Props) {
  // v ∈ [-1, 1] : -1 = full Game, 0 = balanced, +1 = full Chat.
  const [v, setV] = useState(0);
  const lastSent = useRef(0);
  const dragging = useRef(false);

  const pair = pickPair(lines);
  const game = pair?.[0];
  const chat = pair?.[1];

  // Resynchronisation avec les gains réels, sauf pendant une manipulation.
  const derived = game && chat ? balanceOf(game, chat) : 0;
  useEffect(() => {
    if (!dragging.current) setV(derived);
  }, [derived]);

  if (!game || !chat) return null;

  const apply = (value: number, force: boolean) => {
    setV(value);
    const now = performance.now();
    if (!force && now - lastSent.current < THROTTLE_MS) return;
    lastSent.current = now;
    // Crossfade: the side you move away from is attenuated, never boosted.
    onGain(game.id, Math.min(1, 1 - Math.max(0, value)));
    onGain(chat.id, Math.min(1, 1 + Math.min(0, value)));
  };

  // Libellé chiffré du mockup : les pourcentages effectivement appliqués.
  const gamePct = Math.round(Math.min(1, 1 - Math.max(0, v)) * 100);
  const chatPct = Math.round(Math.min(1, 1 + Math.min(0, v)) * 100);

  return (
    <div className="chatmix" title="ChatMix — équilibre Game / Chat (double-clic : centre)">
      <div className="chatmix-head">
        <span className="chatmix-label">Chatmix</span>
        <span className="chatmix-readout">
          {game.name} {gamePct}% · {chat.name} {chatPct}%
        </span>
      </div>
      <div className="chatmix-row">
        <span className="chatmix-icon" style={{ color: game.color }}>
          <GamepadIcon />
        </span>
        <input
          type="range"
          min={-1}
          max={1}
          step={0.02}
          value={v}
          onChange={(e) => apply(Number(e.target.value), false)}
          onPointerDown={() => (dragging.current = true)}
          onPointerUp={() => {
            dragging.current = false;
            apply(v, true);
          }}
          onDoubleClick={() => apply(0, true)}
        />
        <span className="chatmix-icon" style={{ color: chat.color }}>
          <ChatIcon />
        </span>
      </div>
    </div>
  );
}

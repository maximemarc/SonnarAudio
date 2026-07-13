/**
 * Kept out of Icons.tsx on purpose: a file that exports only components
 * plays nicely with Vite's Fast Refresh (react-refresh/only-export-components).
 */
import { GamepadIcon, ChatIcon, MusicIcon, MicIcon, WaveIcon } from "./Icons";

/** Pick a channel glyph from the line's name (Sonar-style). */
export function pickIcon(name: string) {
  const n = name.toLowerCase();
  if (n.includes("game") || n.includes("jeu")) return <GamepadIcon />;
  if (n.includes("chat") || n.includes("voc") || n.includes("discord")) return <ChatIcon />;
  if (n.includes("media") || n.includes("music") || n.includes("musique")) return <MusicIcon />;
  if (n.includes("mic") || n.includes("voix")) return <MicIcon />;
  return <WaveIcon />;
}

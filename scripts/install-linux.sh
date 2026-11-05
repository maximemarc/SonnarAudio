#!/usr/bin/env bash
# Compile MixFlow et l'installe pour l'utilisateur courant (sans sudo) :
#   ~/.local/bin/mixflow
#   ~/.local/share/applications/mixflow.desktop   (menu, Launchpad…)
#   ~/.local/share/icons/hicolor/256x256/apps/mixflow.png
#
# Prérequis : sudo bash scripts/install-linux-deps.sh (une fois), puis Rust
# (rustup) et Node dans le PATH.
#
#   bash scripts/install-linux.sh            # compile + installe
#   bash scripts/install-linux.sh --no-build # réinstalle le dernier binaire
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:$HOME/.local/opt/node/bin:$PATH"

if [[ "${1:-}" != "--no-build" ]]; then
  [[ -d node_modules ]] || npm ci
  npm run tauri build -- --no-bundle
fi

BIN="src-tauri/target/release/mixflow"
[[ -x "$BIN" ]] || { echo "Binaire introuvable : $BIN" >&2; exit 1; }

install -Dm755 "$BIN" "$HOME/.local/bin/mixflow"
install -Dm644 src/assets/logo.png "$HOME/.local/share/icons/hicolor/256x256/apps/mixflow.png"
install -Dm644 /dev/stdin "$HOME/.local/share/applications/mixflow.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=MixFlow
GenericName=Mélangeur audio
Comment=Volume, sortie et égaliseur pour chaque application
Exec=$HOME/.local/bin/mixflow
Icon=mixflow
Terminal=false
Categories=AudioVideo;Audio;Mixer;
Keywords=son;audio;volume;sortie;égaliseur;eq;mixeur;pipewire;
StartupWMClass=mixflow
StartupNotify=true
EOF

command -v update-desktop-database >/dev/null && update-desktop-database -q "$HOME/.local/share/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$HOME/.local/share/icons/hicolor" || true

echo "✅ MixFlow installé : lance-le depuis le Launchpad (catégorie Multimédia)."

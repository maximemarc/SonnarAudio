#!/usr/bin/env bash
# Dépendances système pour compiler MixFlow sous Linux (Debian 13 / Ubuntu 24.04+).
# À lancer une seule fois : sudo bash scripts/install-linux-deps.sh
# Rust (rustup) et Node s'installent ensuite côté utilisateur, sans sudo.
set -euo pipefail

if [[ $EUID -ne 0 ]]; then
  echo "Ce script doit être lancé avec sudo : sudo bash $0" >&2
  exit 1
fi

apt-get update
apt-get install -y --no-install-recommends \
  build-essential pkg-config curl file xz-utils \
  libssl-dev \
  libgtk-3-dev \
  libwebkit2gtk-4.1-dev \
  libsoup-3.0-dev \
  libjavascriptcoregtk-4.1-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libxdo-dev \
  pipewire-bin wireplumber

echo
echo "✅ Dépendances installées. Tu peux revenir à Claude."

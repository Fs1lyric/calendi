#!/usr/bin/env bash
# Builds Calendi and installs it for this user: the binaries in ~/.local/bin,
# the launcher entry and icon under ~/.local/share, and the booker as a
# systemd user service on 127.0.0.1:8787.
set -euo pipefail
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release -p lyric-cal -p lyric-cal-booker
data="${XDG_DATA_HOME:-$HOME/.local/share}"
install -Dm755 target/release/calendi "$HOME/.local/bin/calendi"
# The app was called Lyric Cal before; drop its old launcher files.
rm -f "$HOME/.local/bin/lyric-cal" "${XDG_DATA_HOME:-$HOME/.local/share}/applications/lyric-cal.desktop" \
  "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps/lyric-cal.svg"
install -Dm755 target/release/lyric-cal-booker "$HOME/.local/bin/lyric-cal-booker"
install -Dm644 packaging/lyric-cal-booker.service "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/lyric-cal-booker.service"
install -Dm644 packaging/calendi.desktop "$data/applications/calendi.desktop"
install -Dm644 packaging/calendi.svg "$data/icons/hicolor/scalable/apps/calendi.svg"
update-desktop-database -q "$data/applications" 2>/dev/null || true
gtk-update-icon-cache -q "$data/icons/hicolor" 2>/dev/null || true
systemctl --user daemon-reload
systemctl --user enable lyric-cal-booker.service
systemctl --user restart lyric-cal-booker.service
echo "installed: $HOME/.local/bin/calendi, booking page on http://127.0.0.1:8787/<link>"

#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"

cargo build --release

bin_dir="$HOME/.local/bin"
apps_dir="$HOME/.local/share/applications"
autostart_dir="$HOME/.config/autostart"
install_bin="$bin_dir/simple-reminders"

install -Dm755 target/release/simple-reminders "$install_bin"

mkdir -p "$apps_dir" "$autostart_dir"

sed "s|Exec=simple-reminders|Exec=$install_bin|" \
    data/io.github.alielsmadi.SimpleReminders.desktop \
    > "$apps_dir/io.github.alielsmadi.SimpleReminders.desktop"

sed "s|Exec=simple-reminders|Exec=$install_bin|" \
    data/io.github.alielsmadi.SimpleReminders-autostart.desktop \
    > "$autostart_dir/io.github.alielsmadi.SimpleReminders-autostart.desktop"

update-desktop-database "$apps_dir" || true

echo "Installed. Log out and back in (or run 'simple-reminders --background &' now) to start the background scheduler."

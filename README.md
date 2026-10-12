# Simple Reminders

Provides a simple reminders tool for Linux, built with Ubuntu and Gnome in mind.

simple-reminders is not the only option available out there, however it is:
- The most stable
- The most straight forward reminders app (as close as possible to the macOS native one)
- The one with a quality build using Rust + gtk4-rs + libadwaita

## Installation

Requires Rust (stable) and GTK4/libadwaita dev packages (e.g. on Ubuntu: `sudo apt install libgtk-4-dev libadwaita-1-dev`).

```sh
./install.sh
```

This builds a release binary, installs it to `~/.local/bin/simple-reminders`, and registers desktop + autostart entries. Log out and back in, or run `simple-reminders --background &`, to start the scheduler.

## Testing

```sh
cargo test
```

Runs unit tests for the `core` crate (scheduler, recurrence, db, missed reminders, formatting, sound).

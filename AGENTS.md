# AGENTS.md

Instructions for AI coding agents working in this repository.

## Project overview

Simple Reminders is a Linux reminders app built with Ubuntu/GNOME in mind, aiming to be
as close as possible to the macOS native Reminders app. It is written in Rust using
GTK4 (`gtk4-rs`) and `libadwaita` for the UI, and ships as a single binary that serves
both the GUI and a background daemon (`--background` flag, see `app/src/main.rs`). The
background mode never calls `Application::run()` — this is a deliberate single-instance
design (see the comment in `app/src/main.rs`).

### Workspace layout

Cargo workspace with two members (`Cargo.toml` at the root):

- `core/` — crate `simple-reminders-core`: all business logic, with no GTK dependency.
  - `db.rs` — SQLite persistence via `rusqlite` (bundled).
  - `model.rs` — `Reminder`, `NewReminder`, `RecurrenceRule` types.
  - `recurrence.rs` — recurrence rule calculation.
  - `scheduler.rs` — scheduling/timing logic for when reminders fire.
  - `missed.rs` — handling of reminders missed while the app wasn't running.
  - `format.rs` — display formatting helpers.
  - `sound.rs` — sound-related logic.
  - `paths.rs` — standard app data/config path resolution.
  - `error.rs` — crate-wide `Error`/`Result`.
  - Unit-tested; run with `cargo test`.
- `app/` — binary crate `simple-reminders`: the GTK4/libadwaita UI and the background
  daemon entry point.
  - `main.rs` — entry point; decides GUI vs. `--background` daemon mode.
  - `application.rs` — GTK `Application` setup, background runner.
  - `notifications.rs` — desktop notification integration.
  - `audio.rs` — audio playback for reminder alerts.
  - `ui/` — GTK widgets: `window.rs` (main window), `edit_dialog.rs` (add/edit reminder),
    `settings_dialog.rs` (app settings).

`data/` holds `.desktop` entries (app + autostart) and `style.css` for the UI, installed
by `install.sh`.

## Build & verification

System dependencies (Ubuntu): `sudo apt install libgtk-4-dev libadwaita-1-dev`.

**Always verify changes with a compile and build before considering a task done.** At minimum:

```sh
cargo build --workspace
```

Prefer also running (and must run before marking non-trivial changes complete):

```sh
cargo build --workspace --release   # matches what install.sh actually ships
cargo test                          # core crate's unit tests (scheduler, recurrence, db, missed, format, sound)
```

If `cargo build` or `cargo test` fails, treat the change as incomplete — fix it before
reporting success, don't just describe what should work.

There is no separate lint/CI config in this repo currently; `cargo build` and `cargo test`
are the verification bar.

**Never launch the UI for visual verification, and never take or request a screenshot of
the screen.** Do not run the built binary (GUI or `--background` mode) to "see" a change
working. Verification is strictly limited to `cargo build`/`cargo build --release` and
`cargo test`. Manual/visual testing of the running app is left entirely to the human coder.

## Git workflow

**Never automatically `git add`/stage or `git commit` changes.** Staging and committing
is done manually by the coder (the human). Leave the working tree with your edits
present but unstaged unless explicitly told otherwise.

## Keeping this file up to date

**Keep AGENTS.md up to date as the project changes.** When you add/remove a crate,
module, major dependency, build step, or workflow convention, update the relevant
section above in the same change. Treat a stale AGENTS.md as a bug.

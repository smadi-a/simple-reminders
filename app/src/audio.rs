//! Plays a notification sound by shelling out to a system player, rather than bundling a
//! Rust audio crate — see the project plan for why: both tools below are present on stock
//! Ubuntu/GNOME, handle the .oga/.ogg/.wav files GNOME ships, and respect the user's
//! existing volume/routing automatically.

use std::path::Path;
use std::process::Command;

pub fn play_sound(path: &Path) {
    let path = path.to_path_buf();
    // Fire-and-forget: a notification sound failing to play should never block or crash
    // the reminder flow, so errors are swallowed here deliberately.
    std::thread::spawn(move || {
        let canberra = Command::new("canberra-gtk-play").arg("-f").arg(&path).status();
        if canberra.map(|s| s.success()).unwrap_or(false) {
            return;
        }
        let _ = Command::new("paplay").arg(&path).status();
    });
}

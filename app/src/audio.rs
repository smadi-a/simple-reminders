//! Plays a notification sound by shelling out to a system player, rather than bundling a
//! Rust audio crate — see the project plan for why: these tools are present on stock
//! Ubuntu/GNOME, handle the .oga/.ogg/.wav files GNOME ships, and respect the user's
//! existing volume/routing automatically.
//!
//! `pw-play` and `paplay` are tried before `canberra-gtk-play` deliberately: the latter
//! honors the desktop-wide `org.gnome.desktop.sound event-sounds` toggle and refuses to
//! play anything at all when the user has turned that off, which has nothing to do with
//! whether our own reminder sound should fire. `pw-play`/`paplay` play the file
//! unconditionally, so keep them first — don't "simplify" this back to canberra-first.

use std::path::Path;
use std::process::Command;

pub fn play_sound(path: &Path) {
    let path = path.to_path_buf();
    // Fire-and-forget: a notification sound failing to play should never block or crash
    // the reminder flow, so failures are logged (not propagated) here.
    std::thread::spawn(move || {
        let candidates: [(&str, &[&str]); 3] = [
            ("pw-play", &[]),
            ("paplay", &[]),
            ("canberra-gtk-play", &["-f"]),
        ];
        for (program, extra_args) in candidates {
            match Command::new(program).args(extra_args).arg(&path).status() {
                Ok(status) if status.success() => return,
                Ok(status) => eprintln!(
                    "play_sound: '{program}' exited with {status} while playing {}",
                    path.display()
                ),
                Err(err) => eprintln!(
                    "play_sound: failed to launch '{program}' for {}: {err}",
                    path.display()
                ),
            }
        }
        eprintln!(
            "play_sound: all backends failed for {} (tried pw-play, paplay, canberra-gtk-play); no sound played",
            path.display()
        );
    });
}

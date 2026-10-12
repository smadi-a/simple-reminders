//! Discovering existing Ubuntu/GNOME system notification sounds the user can pick from.

use std::path::{Path, PathBuf};

const AUDIO_EXTENSIONS: &[&str] = &["oga", "ogg", "wav"];

/// The default places Ubuntu/GNOME ship notification sound files.
pub fn default_sound_dirs() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/share/sounds/freedesktop/stereo"),
        PathBuf::from("/usr/share/sounds/Yaru/stereo"),
    ]
}

/// Scans the given directories (non-recursively) for playable sound files, sorted by
/// filename. Takes the directories to scan as a parameter (rather than hard-coding the
/// real system paths) so this is deterministically testable against a fixture directory;
/// `default_sound_dirs()` supplies the real paths for actual use.
pub fn discover_system_sounds_in(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = dirs
        .iter()
        .filter(|dir| dir.is_dir())
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| is_audio_file(path))
        .collect();
    found.sort();
    found
}

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| AUDIO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn discover_system_sounds_filters_to_known_audio_extensions() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("bell.oga"), b"").unwrap();
        fs::write(dir.path().join("complete.ogg"), b"").unwrap();
        fs::write(dir.path().join("alert.wav"), b"").unwrap();
        fs::write(dir.path().join("readme.txt"), b"").unwrap();
        fs::write(dir.path().join("cover.png"), b"").unwrap();

        let found = discover_system_sounds_in(&[dir.path().to_path_buf()]);
        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["alert.wav", "bell.oga", "complete.ogg"]);
    }

    #[test]
    fn discover_system_sounds_returns_empty_for_missing_directory_without_erroring() {
        let missing = PathBuf::from("/this/path/does/not/exist/simple-reminders-test");
        assert_eq!(discover_system_sounds_in(&[missing]), Vec::<PathBuf>::new());
    }
}

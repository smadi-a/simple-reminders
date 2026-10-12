use std::path::PathBuf;

/// Resolves the SQLite database path: `$XDG_DATA_HOME/simple-reminders/reminders.db`,
/// falling back to `~/.local/share/simple-reminders/reminders.db`. Creates the containing
/// directory if it doesn't exist yet.
pub fn db_path() -> std::io::Result<PathBuf> {
    let data_dir = match std::env::var_os("XDG_DATA_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => {
            let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "HOME is not set")
            })?;
            home.join(".local/share")
        }
    };
    let app_dir = data_dir.join("simple-reminders");
    std::fs::create_dir_all(&app_dir)?;
    Ok(app_dir.join("reminders.db"))
}

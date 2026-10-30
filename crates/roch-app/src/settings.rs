//! Saved theme preference: `theme.txt` ("Light" or "Dark") in the local data folder.

use std::path::PathBuf;

fn theme_path() -> PathBuf {
    roch_core::paths::data_directory().join("theme.txt")
}

/// Dark unless the saved preference is exactly "Light" (whitespace trimmed).
pub fn load_light_theme() -> bool {
    std::fs::read_to_string(theme_path()).map(|t| t.trim() == "Light").unwrap_or(false)
}

/// Returns false when the preference could not be saved.
pub fn save_light_theme(light: bool) -> bool {
    let path = theme_path();
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return false;
        }
    }
    std::fs::write(path, if light { "Light" } else { "Dark" }).is_ok()
}

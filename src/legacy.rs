//! One-time move of the data saved while the app was called Lazy.

use std::path::Path;

/// Former and current data folders, relative to the home folder.
const RENAMED_DIRS: [(&str, &str); 2] = [
    (
        "Library/Application Support/Lazy",
        "Library/Application Support/Meow",
    ),
    (".config/lazy", ".config/meow"),
];

/// Moves each former data folder to its new name, unless the new one already
/// exists. Returns what could not be moved.
pub fn migrate_data(home: &Path) -> Vec<String> {
    RENAMED_DIRS
        .iter()
        .filter_map(|(old, new)| {
            let (old, new) = (home.join(old), home.join(new));
            if !old.is_dir() || new.exists() {
                return None;
            }
            let moved = new
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::rename(&old, &new));
            moved
                .err()
                .map(|e| format!("Could not move {} to {}: {e}", old.display(), new.display()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(name: &str) -> std::path::PathBuf {
        let home = std::env::temp_dir().join(format!("meow-legacy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    #[test]
    fn moves_old_folders_with_their_files() {
        let home = temp_home("move");
        let old = home.join("Library/Application Support/Lazy");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("tasks.json"), "{}").unwrap();
        std::fs::create_dir_all(home.join(".config/lazy")).unwrap();

        assert!(migrate_data(&home).is_empty());
        assert!(!old.exists());
        let moved = home.join("Library/Application Support/Meow/tasks.json");
        assert_eq!(std::fs::read_to_string(moved).unwrap(), "{}");
        assert!(home.join(".config/meow").is_dir());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn never_overwrites_existing_new_folders() {
        let home = temp_home("keep");
        let old = home.join(".config/lazy");
        let new = home.join(".config/meow");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("actions.json"), "[]").unwrap();

        assert!(migrate_data(&home).is_empty());
        assert!(old.exists(), "old data left alone");
        assert_eq!(
            std::fs::read_to_string(new.join("actions.json")).unwrap(),
            "[]"
        );
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn nothing_to_do_without_old_folders() {
        let home = temp_home("none");
        assert!(migrate_data(&home).is_empty());
        assert!(!home.join(".config/meow").exists());
        std::fs::remove_dir_all(home).unwrap();
    }
}

//! Location of the GUI's persisted TOML files (settings, profiles, slot configs).
//!
//! - macOS: `~/Library/Application Support/com.mc5000.charger/` — named after the app's
//!   bundle identifier (`IDENTIFIER` in `packaging/macos/create_app_bundle.sh`).
//! - Linux: `~/.config/mc5000/`
//! - Windows: `%APPDATA%\mc5000\config\`
//!
//! Before the macOS folder matched the bundle identifier, files lived in
//! `~/Library/Application Support/rs.mc5000/`. They are moved on first access.

use serde::{de::DeserializeOwned, Serialize};
use std::path::PathBuf;

/// Name passed to `confy`; decides the Linux and Windows folders.
const APP: &str = "mc5000";

/// Must match `IDENTIFIER` in `packaging/macos/create_app_bundle.sh`.
#[cfg(any(target_os = "macos", test))]
const MACOS_BUNDLE_ID: &str = "com.mc5000.charger";

/// Full path of the config file `<name>.toml`.
pub fn path(name: &str) -> Result<PathBuf, confy::ConfyError> {
    let default_path = confy::get_configuration_file_path(APP, name)?;

    #[cfg(target_os = "macos")]
    {
        Ok(migrated(default_path.clone(), in_dir(&default_path, MACOS_BUNDLE_ID)))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(default_path)
    }
}

/// Loads `<name>.toml`, falling back to `T::default()` if it is missing or unreadable.
pub fn load<T: Serialize + DeserializeOwned + Default>(name: &str) -> T {
    path(name).and_then(confy::load_path).unwrap_or_default()
}

/// Writes `<name>.toml`.
pub fn store<T: Serialize>(name: &str, value: &T) -> Result<(), confy::ConfyError> {
    confy::store_path(path(name)?, value)
}

/// The same file name, in a sibling directory called `dir_name`.
#[cfg(any(target_os = "macos", test))]
fn in_dir(file: &std::path::Path, dir_name: &str) -> PathBuf {
    let base = file
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap_or_else(|| std::path::Path::new(""));
    let file_name = file.file_name().unwrap_or_default();
    base.join(dir_name).join(file_name)
}

/// Moves `old` to `new` if only `old` exists, and returns the path to use.
///
/// If the move fails, `old` is returned so existing settings are still read.
#[cfg(any(target_os = "macos", test))]
fn migrated(old: PathBuf, new: PathBuf) -> PathBuf {
    if new.exists() || !old.exists() {
        return new;
    }
    let moved = new
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::rename(&old, &new));
    match moved {
        Ok(()) => {
            log::info!("Moved config {} -> {}", old.display(), new.display());
            // Removes the old folder once its last file has moved; fails harmlessly otherwise.
            if let Some(old_dir) = old.parent() {
                let _ = std::fs::remove_dir(old_dir);
            }
            new
        }
        Err(e) => {
            log::warn!("Could not move config {} -> {}: {e}", old.display(), new.display());
            old
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A fresh, empty directory under the system temp dir.
    fn scratch_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("mc5000-config-store-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn in_dir_replaces_only_the_parent_folder() {
        let old = PathBuf::from("/Users/u/Library/Application Support/rs.mc5000/settings.toml");
        assert_eq!(
            in_dir(&old, MACOS_BUNDLE_ID),
            PathBuf::from("/Users/u/Library/Application Support/com.mc5000.charger/settings.toml"),
        );
    }

    #[test]
    fn migrated_moves_existing_file_and_removes_empty_old_dir() {
        let root = scratch_dir("moves");
        let old = root.join("rs.mc5000/settings.toml");
        let new = root.join("com.mc5000.charger/settings.toml");
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::write(&old, "theme = \"Light\"\n").unwrap();

        assert_eq!(migrated(old.clone(), new.clone()), new);
        assert_eq!(fs::read_to_string(&new).unwrap(), "theme = \"Light\"\n");
        assert!(!old.exists());
        assert!(!old.parent().unwrap().exists());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn migrated_keeps_old_dir_while_other_files_remain() {
        let root = scratch_dir("partial");
        let old = root.join("rs.mc5000/settings.toml");
        let other = root.join("rs.mc5000/profiles.toml");
        let new = root.join("com.mc5000.charger/settings.toml");
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::write(&old, "a").unwrap();
        fs::write(&other, "b").unwrap();

        assert_eq!(migrated(old, new.clone()), new);
        assert!(other.exists());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn migrated_never_overwrites_an_existing_new_file() {
        let root = scratch_dir("no-overwrite");
        let old = root.join("rs.mc5000/settings.toml");
        let new = root.join("com.mc5000.charger/settings.toml");
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::create_dir_all(new.parent().unwrap()).unwrap();
        fs::write(&old, "old").unwrap();
        fs::write(&new, "new").unwrap();

        assert_eq!(migrated(old.clone(), new.clone()), new);
        assert_eq!(fs::read_to_string(&new).unwrap(), "new");
        assert_eq!(fs::read_to_string(&old).unwrap(), "old");

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn migrated_uses_new_path_when_nothing_exists_yet() {
        let root = scratch_dir("fresh");
        let new = root.join("com.mc5000.charger/settings.toml");
        assert_eq!(migrated(root.join("rs.mc5000/settings.toml"), new.clone()), new);
        fs::remove_dir_all(&root).unwrap();
    }
}

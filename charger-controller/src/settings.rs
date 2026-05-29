//! Persistent settings stored under the platform's standard config dir
//! (`confy` chooses: macOS → `~/Library/Application Support/mc5000/`,
//! Linux → `~/.config/mc5000/`, Windows → `%APPDATA%\mc5000\`).

use serde::{Deserialize, Serialize};

const APP: &str = "mc5000";
const CFG: &str = "settings";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AppTheme {
    Dark,
}

impl std::fmt::Display for AppTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppTheme::Dark => write!(f, "Dark"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub theme: AppTheme,
    pub save_last_device: bool,
    pub last_device_id: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: AppTheme::Dark,
            save_last_device: true,
            last_device_id: None,
        }
    }
}

pub fn load() -> Settings {
    confy::load(APP, CFG).unwrap_or_default()
}

pub fn save(s: &Settings) -> Result<(), confy::ConfyError> {
    confy::store(APP, CFG, s)
}

//! Persistent settings. See `config_store` for where the file lives.

use serde::{Deserialize, Serialize};

const CFG: &str = "settings";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AppTheme {
    Light,
    Dark,
}

impl std::fmt::Display for AppTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppTheme::Light => write!(f, "{}", crate::i18n::t!("theme.light")),
            AppTheme::Dark => write!(f, "{}", crate::i18n::t!("theme.dark")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub theme: AppTheme,
    pub language: String,
    pub save_last_device: bool,
    pub last_device_id: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: AppTheme::Dark,
            language: crate::i18n::detect_system_language(),
            save_last_device: true,
            last_device_id: None,
        }
    }
}

pub fn load() -> Settings {
    crate::config_store::load(CFG)
}

pub fn save(s: &Settings) -> Result<(), confy::ConfyError> {
    crate::config_store::store(CFG, s)
}

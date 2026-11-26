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

/// What the graph shows: the last N seconds, or everything recorded.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum GraphTimeMode {
    Roll,
    #[default]
    Infinite,
}

/// Default Roll window, in seconds.
pub const DEFAULT_GRAPH_WINDOW_S: u32 = 600;

fn default_graph_window_s() -> u32 {
    DEFAULT_GRAPH_WINDOW_S
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub theme: AppTheme,
    pub language: String,
    pub save_last_device: bool,
    pub last_device_id: Option<String>,
    #[serde(default)]
    pub graph_time_mode: GraphTimeMode,
    /// Roll window in seconds.
    #[serde(default = "default_graph_window_s")]
    pub graph_window_s: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: AppTheme::Dark,
            language: crate::i18n::detect_system_language(),
            save_last_device: true,
            last_device_id: None,
            graph_time_mode: GraphTimeMode::default(),
            graph_window_s: DEFAULT_GRAPH_WINDOW_S,
        }
    }
}

pub fn load() -> Settings {
    crate::config_store::load(CFG)
}

pub fn save(s: &Settings) -> Result<(), confy::ConfyError> {
    crate::config_store::store(CFG, s)
}

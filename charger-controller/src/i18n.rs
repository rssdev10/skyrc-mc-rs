//! Tiny i18n layer. Locales live in `charger-controller/locales/{en,ru,zh,es,hi}.json`.
//! Loaded at runtime via `rust-i18n`. Keys default to the English value.
//!
//! NB: the `rust_i18n::i18n!("locales", fallback = "en");` invocation lives
//! in `main.rs` (must be at the crate root for the macro to expand correctly).

pub use rust_i18n::t;

pub fn set_language(lang: &str) {
    rust_i18n::set_locale(lang);
}

pub const LANGUAGES: &[(&str, &str, &str)] = &[
    ("en", "English", "English"),
    ("ru", "Русский", "Russian"),
    ("es", "Español", "Spanish"),
    ("zh", "中文", "Chinese"),
    ("hi", "हिन्दी", "Hindi"),
];

/// Returns display names for the language selector, including native names.
/// e.g. "English (English)", "Russian (Русский)"
pub fn language_display_names() -> Vec<(String, String)> {
    LANGUAGES
        .iter()
        .map(|(code, native, english)| {
            (code.to_string(), format!("{} ({})", english, native))
        })
        .collect()
}

/// Detect the system locale and return a matching supported language code,
/// falling back to "en" if none matches.
pub fn detect_system_language() -> String {
    if let Some(locale) = sys_locale::get_locale() {
        let lang = locale.split(['-', '_']).next().unwrap_or("en");
        if LANGUAGES.iter().any(|(code, _, _)| *code == lang) {
            return lang.to_string();
        }
    }
    "en".into()
}

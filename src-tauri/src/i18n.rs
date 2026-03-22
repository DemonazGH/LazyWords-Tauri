use serde_json::Value;
use std::fs;
use std::path::PathBuf;

pub struct I18n {
    strings: Value,
    pub current_locale: String,
    locales_dir: PathBuf,
}

impl I18n {
    pub fn new(locales_dir: PathBuf) -> Self {
        let mut i18n = Self {
            strings: serde_json::json!({}),
            current_locale: "en".to_string(),
            locales_dir,
        };
        i18n.set_locale("en");
        i18n
    }

    pub fn resolve_code(code: &str) -> &'static str {
        if code == "auto" || code.is_empty() {
            // Detect OS locale
            if let Some(locale) = sys_locale::get_locale() {
                if locale.starts_with("ru") {
                    return "ru";
                }
            }
            return "en";
        }
        if code.starts_with("ru") {
            return "ru";
        }
        "en"
    }

    pub fn set_locale(&mut self, code: &str) {
        let resolved = Self::resolve_code(code);
        let file = self.locales_dir.join(format!("{}.json", resolved));
        
        if let Ok(data) = fs::read_to_string(&file) {
            if let Ok(v) = serde_json::from_str(&data) {
                self.strings = v;
                self.current_locale = resolved.to_string();
                return;
            }
        }
        
        // Fallback to English
        let fallback = self.locales_dir.join("en.json");
        if let Ok(data) = fs::read_to_string(&fallback) {
            if let Ok(v) = serde_json::from_str(&data) {
                self.strings = v;
                self.current_locale = "en".to_string();
                return;
            }
        }
        
        self.strings = serde_json::json!({});
        self.current_locale = "en".to_string();
    }

    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.strings.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or(key)
    }

    pub fn get_strings(&self) -> Value {
        self.strings.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn test_i18n() -> I18n {
        let locales_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/locales");
        I18n::new(locales_dir)
    }

    #[test]
    fn set_locale_en_switches_to_english() {
        let mut i18n = test_i18n();
        i18n.set_locale("en");
        assert_eq!(i18n.current_locale, "en");
    }

    #[test]
    fn set_locale_ru_switches_to_russian() {
        let mut i18n = test_i18n();
        i18n.set_locale("ru");
        assert_eq!(i18n.current_locale, "ru");
    }

    #[test]
    fn t_returns_correct_string_for_locale() {
        let mut i18n = test_i18n();
        i18n.set_locale("en");
        assert_eq!(i18n.t("app.name"), "LazyWords");
        i18n.set_locale("ru");
        assert_eq!(i18n.t("stats.streak"), "дней подряд");
    }

    #[test]
    fn t_returns_raw_string_with_placeholder() {
        let mut i18n = test_i18n();
        i18n.set_locale("en");
        // Substitution is handled in JS; Rust returns the raw template string intact
        assert_eq!(i18n.t("settings.import.success"), "Imported: {n} words");
    }

    #[test]
    fn get_locale_returns_current_locale_after_set() {
        let mut i18n = test_i18n();
        i18n.set_locale("ru");
        assert_eq!(i18n.current_locale, "ru");
        i18n.set_locale("en");
        assert_eq!(i18n.current_locale, "en");
    }

    #[test]
    fn t_returns_key_itself_for_nonexistent_key() {
        let i18n = test_i18n();
        assert_eq!(i18n.t("nonexistent.key"), "nonexistent.key");
    }
}

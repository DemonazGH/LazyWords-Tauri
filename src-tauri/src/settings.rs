use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub position: String,
    pub font_size: f32,
    pub show_duration: f32,
    pub interval: f32,
    pub fade_duration: f32,
    pub active_dictionary: String,
    pub auto_start: bool,
    pub first_launch: bool,
    pub language: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            position: "center".to_string(),
            font_size: 22.0,
            show_duration: 4.0,
            interval: 3.0,
            fade_duration: 0.5,
            active_dictionary: String::new(),
            auto_start: true,
            first_launch: true,
            language: "auto".to_string(),
        }
    }
}

pub fn load_settings(path: &Path) -> Settings {
    if path.exists() {
        if let Ok(data) = fs::read_to_string(path) {
            if let Ok(settings) = serde_json::from_str::<Settings>(&data) {
                return settings;
            }
        }
    }
    let default = Settings::default();
    save_settings(path, &default);
    default
}

pub fn save_settings(path: &Path, settings: &Settings) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(data) = serde_json::to_string_pretty(settings) {
        let _ = fs::write(path, data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_have_expected_values() {
        let s = Settings::default();
        assert_eq!(s.interval, 3.0);
        assert_eq!(s.font_size, 22.0);
        assert_eq!(s.show_duration, 4.0);
        assert_eq!(s.active_dictionary, "");
        assert_eq!(s.position, "center");
        assert_eq!(s.language, "auto");
        assert!(s.auto_start);
        assert!(s.first_launch);
    }

    #[test]
    fn settings_serialize_deserialize_without_loss() {
        let original = Settings::default();
        let json = serde_json::to_string(&original).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.interval, original.interval);
        assert_eq!(restored.font_size, original.font_size);
        assert_eq!(restored.show_duration, original.show_duration);
        assert_eq!(restored.fade_duration, original.fade_duration);
        assert_eq!(restored.active_dictionary, original.active_dictionary);
        assert_eq!(restored.position, original.position);
        assert_eq!(restored.language, original.language);
        assert_eq!(restored.auto_start, original.auto_start);
        assert_eq!(restored.first_launch, original.first_launch);
    }
}

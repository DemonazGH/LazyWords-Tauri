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
    #[serde(default)]
    pub msix_startup_initialized: bool,
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
            msix_startup_initialized: false,
            first_launch: true,
            language: "auto".to_string(),
        }
    }
}

impl Settings {
    pub fn needs_onboarding(&self) -> bool {
        // Recover profiles stranded by older builds clearing firstLaunch too early.
        self.first_launch || self.active_dictionary.is_empty()
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
    if let Err(error) = try_save_settings(path, settings) {
        eprintln!("Failed to save settings: {error}");
    }
}

pub fn try_save_settings(path: &Path, settings: &Settings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let data = serde_json::to_string_pretty(settings).map_err(|error| error.to_string())?;
    fs::write(path, data).map_err(|error| error.to_string())
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
        assert_eq!(restored.msix_startup_initialized, original.msix_startup_initialized);
        assert_eq!(restored.first_launch, original.first_launch);
    }

    #[test]
    fn legacy_settings_migrate_msix_startup_only_once() {
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        json.as_object_mut().unwrap().remove("msixStartupInitialized");
        let mut settings: Settings = serde_json::from_value(json).unwrap();
        assert!(!settings.msix_startup_initialized);
        settings.msix_startup_initialized = true;
        let restored: Settings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(restored.msix_startup_initialized);
    }

    #[test]
    fn interrupted_first_launch_still_needs_onboarding() {
        let settings = Settings::default();
        let restored: Settings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(restored.first_launch);
        assert!(restored.needs_onboarding());
    }

    #[test]
    fn recovers_legacy_empty_dictionary_profile() {
        let mut settings = Settings::default();
        settings.first_launch = false;
        assert!(settings.needs_onboarding());
    }

    #[test]
    fn completed_selection_does_not_repeat_onboarding() {
        let mut settings = Settings::default();
        settings.active_dictionary = "en-ru".into();
        settings.first_launch = false;
        assert!(!settings.needs_onboarding());
    }

    #[test]
    fn explicit_first_launch_is_respected_with_a_dictionary() {
        let mut settings = Settings::default();
        settings.active_dictionary = "en-ru".into();
        assert!(settings.needs_onboarding());
    }

    #[test]
    fn settings_write_failure_is_reported() {
        assert!(try_save_settings(Path::new("."), &Settings::default()).is_err());
    }
}

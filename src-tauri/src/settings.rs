use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
// New fields need an explicit deserialization default for existing profiles.
pub struct Settings {
    pub position: String,
    pub font_size: f32,
    pub show_duration: f32,
    #[serde(default, deserialize_with = "deserialize_answer_delay")]
    pub answer_delay: u8,
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
            answer_delay: 0,
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

fn deserialize_answer_delay<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u8, D::Error> {
    u8::deserialize(deserializer).map(|seconds| seconds.min(5))
}

impl Settings {
    pub fn needs_onboarding(&self) -> bool {
        // Recover profiles stranded by older builds clearing firstLaunch too early.
        self.first_launch || self.active_dictionary.is_empty()
    }
}

pub fn load_settings(path: &Path) -> io::Result<Settings> {
    match fs::read(path) {
        Ok(data) => serde_json::from_slice(&data)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(error) => Err(error),
    }
}

pub fn save_settings(path: &Path, settings: &Settings) {
    if let Err(error) = try_save_settings(path, settings) {
        eprintln!("Failed to save settings: {error}");
    }
}

pub fn try_save_settings(path: &Path, settings: &Settings) -> Result<(), String> {
    let parent = path.parent().filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let data = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    // Replace the destination only after a complete write in the same directory.
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    temp.write_all(&data).map_err(|error| error.to_string())?;
    temp.as_file().sync_all().map_err(|error| error.to_string())?;
    temp.persist(path).map_err(|error| error.error.to_string())?;
    Ok(())
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
        assert_eq!(s.answer_delay, 0);
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
    fn old_profiles_default_answer_delay_to_off_and_keep_onboarding_complete() {
        let mut settings = Settings::default();
        settings.active_dictionary = "en-ru".into();
        settings.first_launch = false;
        let mut json = serde_json::to_value(&settings).unwrap();
        json.as_object_mut().unwrap().remove("answerDelay");
        let restored: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(restored.answer_delay, 0);
        assert_eq!(restored.active_dictionary, settings.active_dictionary);
        assert!(!restored.needs_onboarding());
    }

    #[test]
    fn answer_delay_is_persisted_and_bounded() {
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        json["answerDelay"] = serde_json::json!(3);
        let settings: Settings = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(settings).unwrap()["answerDelay"], 3);
        json["answerDelay"] = serde_json::json!(12);
        assert_eq!(serde_json::from_value::<Settings>(json).unwrap().answer_delay, 5);
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
    fn missing_settings_use_first_launch_defaults_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data/settings.json");
        let settings = load_settings(&path).unwrap();
        assert!(settings.needs_onboarding());
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn existing_profile_keeps_preferences_and_does_not_restart_onboarding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut profile = serde_json::json!({
            "position": "bottom-right", "fontSize": 28.0, "showDuration": 9.0,
            "interval": 6.0, "fadeDuration": 0.75, "activeDictionary": "my-deck",
            "autoStart": false, "firstLaunch": false, "language": "ru",
            "msixStartupInitialized": true
        });
        for startup_initialized in [true, false] {
            if !startup_initialized {
                profile.as_object_mut().unwrap().remove("msixStartupInitialized");
            }
            let original = serde_json::to_vec(&profile).unwrap();
            fs::write(&path, &original).unwrap();
            let settings = load_settings(&path).unwrap();
            assert!(!settings.needs_onboarding());
            assert_eq!(settings.msix_startup_initialized, startup_initialized);
            let mut restored = serde_json::to_value(settings).unwrap();
            assert_eq!(restored.as_object_mut().unwrap().remove("answerDelay"), Some(0.into()));
            if !startup_initialized {
                restored.as_object_mut().unwrap().remove("msixStartupInitialized");
            }
            assert_eq!(restored, profile);
            assert_eq!(fs::read(&path).unwrap(), original);
        }
    }

    #[test]
    fn invalid_settings_are_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut wrong_type = serde_json::to_value(Settings::default()).unwrap();
        wrong_type["fontSize"] = serde_json::json!("not a number");
        for original in [b"{\"firstLaunch\":".to_vec(), vec![0xff], serde_json::to_vec(&wrong_type).unwrap()] {
            fs::write(&path, &original).unwrap();
            assert_eq!(load_settings(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
            assert_eq!(fs::read(&path).unwrap(), original);
        }
    }

    #[test]
    fn unreadable_settings_are_not_treated_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::create_dir(&path).unwrap();
        assert!(load_settings(&path).is_err());
        assert!(try_save_settings(&path, &Settings::default()).is_err());
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn saves_can_create_and_replace_a_complete_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut settings = Settings::default();
        try_save_settings(&path, &settings).unwrap();
        assert_eq!(load_settings(&path).unwrap().font_size, 22.0);
        settings.font_size = 30.0;
        try_save_settings(&path, &settings).unwrap();
        assert_eq!(load_settings(&path).unwrap().font_size, 30.0);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn blocked_replacement_keeps_the_existing_settings_bytes() {
        use std::os::windows::fs::OpenOptionsExt;
        use winapi::um::winnt::{FILE_SHARE_READ, FILE_SHARE_WRITE};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut settings = Settings::default();
        try_save_settings(&path, &settings).unwrap();
        let original = fs::read(&path).unwrap();
        let _lock = fs::OpenOptions::new().read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE).open(&path).unwrap();
        settings.font_size = 30.0;
        assert!(try_save_settings(&path, &settings).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

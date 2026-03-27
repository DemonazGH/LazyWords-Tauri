use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

use crate::settings::{Settings, load_settings};
use crate::stats::{StatsTracker, load_stats};
use crate::word_engine::{WordEngine, WordWithIndex};

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub stats_tracker: Mutex<StatsTracker>,
    pub learned: Mutex<HashMap<String, Vec<usize>>>,
    pub word_engine: Mutex<WordEngine>,
    pub i18n: Mutex<crate::i18n::I18n>,
    pub is_paused: Mutex<bool>,
    pub is_card_visible: Mutex<bool>,
    pub current_word: Mutex<Option<WordWithIndex>>,
    /// Pending "all-learned" event — cleared by handle_all_learned before it runs.
    pub all_learned: Mutex<bool>,
    /// Set when every dictionary is exhausted — keeps timer in 30 s idle loop.
    pub all_dicts_done: Mutex<bool>,
    /// Fired by mark_learned when it sets all_learned = true, to wake the timer early.
    pub all_learned_notify: tokio::sync::Notify,
    /// Fired by Ctrl+Shift+N / tray "Next" to interrupt the interval wait and reset it.
    pub manual_trigger: tokio::sync::Notify,
    /// Handle to the system tray icon so it can be updated from anywhere.
    pub tray: Mutex<Option<tauri::tray::TrayIcon>>,
}

pub fn init_state(app: &AppHandle) -> AppState {
    let app_dir = app.path().app_data_dir().unwrap().join("data");
    let settings_path = app_dir.join("settings.json");
    let stats_path = app_dir.join("stats.json");
    let learned_path = app_dir.join("learned.json");

    let settings = load_settings(&settings_path);
    let stats = load_stats(&stats_path);
    
    let learned: HashMap<String, Vec<usize>> = if learned_path.exists() {
        if let Ok(data) = std::fs::read_to_string(&learned_path) {
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            HashMap::new()
        }
    } else {
        HashMap::new()
    };

    let engine = WordEngine::new();
    
    let mut locales_dir = app.path().resolve("locales", tauri::path::BaseDirectory::Resource).unwrap_or_default();
    if !locales_dir.exists() {
        let mut p = std::env::current_dir().unwrap();
        p.push("../src/locales");
        locales_dir = p;
    }
    let mut i18n = crate::i18n::I18n::new(locales_dir);
    i18n.set_locale(&settings.language);
    
    AppState {
        settings: Mutex::new(settings),
        stats_tracker: Mutex::new(StatsTracker::new(stats)),
        learned: Mutex::new(learned),
        word_engine: Mutex::new(engine),
        i18n: Mutex::new(i18n),
        is_paused: Mutex::new(false),
        is_card_visible: Mutex::new(false),
        current_word: Mutex::new(None),
        all_learned: Mutex::new(false),
        all_dicts_done: Mutex::new(false),
        all_learned_notify: tokio::sync::Notify::new(),
        manual_trigger: tokio::sync::Notify::new(),
        tray: Mutex::new(None),
    }
}

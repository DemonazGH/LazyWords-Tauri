use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use tauri::AppHandle;

use crate::settings::Settings;
use crate::stats::StatsTracker;
use crate::word_engine::WordEngine;

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub stats_tracker: Mutex<StatsTracker>,
    pub learned: Mutex<HashMap<String, Vec<usize>>>,
    pub word_engine: Mutex<WordEngine>,
    pub scheduling_history: Mutex<crate::scheduler::SchedulingHistory>,
    pub quick_add_draft: Mutex<Option<crate::quick_add::Draft>>,
    pub i18n: Mutex<crate::i18n::I18n>,
    pub is_paused: Mutex<bool>,
    /// Pending "all-learned" event — cleared by handle_all_learned before it runs.
    pub all_learned: Mutex<bool>,
    /// Set when every dictionary is exhausted — keeps timer in 30 s idle loop.
    pub all_dicts_done: Mutex<bool>,
    /// Fired by mark_learned when it sets all_learned = true, to wake the timer early.
    pub all_learned_notify: tokio::sync::Notify,
    /// Fired when a manual card replaces a scheduled card or interrupts the interval.
    pub manual_trigger: tokio::sync::Notify,
    /// Handle to the system tray icon so it can be updated from anywhere.
    pub tray: Mutex<Option<tauri::tray::TrayIcon>>,
}

pub fn init_state(app: &AppHandle, app_dir: &Path, settings: Settings) -> Result<AppState, crate::profile_data::LoadError> {
    let crate::profile_data::ProfileData { learned, stats } =
        crate::profile_data::ProfileData::load(app_dir)?;

    let engine = WordEngine::new();
    
    let i18n = crate::i18n::I18n::for_app(app, &settings.language);
    
    Ok(AppState {
        settings: Mutex::new(settings),
        stats_tracker: Mutex::new(StatsTracker::new(stats)),
        learned: Mutex::new(learned),
        word_engine: Mutex::new(engine),
        scheduling_history: Mutex::new(crate::scheduler::SchedulingHistory::load(
            app_dir.join("progress.json"),
        )),
        quick_add_draft: Mutex::new(None),
        i18n: Mutex::new(i18n),
        is_paused: Mutex::new(false),
        all_learned: Mutex::new(false),
        all_dicts_done: Mutex::new(false),
        all_learned_notify: tokio::sync::Notify::new(),
        manual_trigger: tokio::sync::Notify::new(),
        tray: Mutex::new(None),
    })
}

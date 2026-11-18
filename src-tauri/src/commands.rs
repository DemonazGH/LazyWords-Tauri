use tauri::{AppHandle, State, Manager, Emitter};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::collections::HashMap;
use tauri_plugin_dialog::DialogExt;

use crate::state::AppState;
use crate::settings::Settings;

// ── Persistence helpers ──────────────────────────────────────────────────────

pub fn save_learned_to_disk(app: &AppHandle, learned: &HashMap<String, Vec<usize>>) {
    let path = app.path().app_data_dir().unwrap().join("data").join("learned.json");
    if let Err(error) = crate::profile_data::save_json(&path, learned) {
        eprintln!("Failed to save learned progress ({}): {error}", path.display());
    }
}

pub fn save_stats_to_disk(app: &AppHandle, stats: &crate::stats::Stats) {
    let path = app.path().app_data_dir().unwrap().join("data").join("stats.json");
    if let Err(error) = crate::stats::save_stats(&path, stats) {
        eprintln!("Failed to save statistics ({}): {error}", path.display());
    }
}

// ── Settings ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_settings(app: AppHandle, state: State<'_, AppState>) -> Settings {
    let mut settings = state.settings.lock().unwrap().clone();
    settings.auto_start = crate::autostart::is_enabled(&app).unwrap_or_else(|error| {
        eprintln!("Failed to read autostart: {error}");
        false
    });
    settings
}

#[tauri::command]
pub fn save_settings(app: AppHandle, state: State<'_, AppState>, new_settings: Value) -> bool {
    let startup_requested = new_settings.get("autoStart").and_then(Value::as_bool);
    let mut current = state.settings.lock().unwrap();
    let old_dict = current.active_dictionary.clone();
    let old_lang = current.language.clone();
    let previous = current.clone();

    if let Value::Object(mut current_map) = serde_json::to_value(&*current).unwrap_or(Value::Null) {
        if let Value::Object(new_map) = new_settings {
            for (k, v) in new_map {
                current_map.insert(k, v);
            }
            match serde_json::from_value::<Settings>(Value::Object(current_map)) {
                Ok(patched) => *current = patched,
                Err(error) => {
                    eprintln!("Invalid settings patch: {error}");
                    return false;
                }
            }
        }
    }

    let selected_words = if current.active_dictionary != old_dict {
        load_dictionary_by_id(&app, &current.active_dictionary)
    } else {
        None
    };
    if selected_words.as_ref().is_some_and(|words| !words.is_empty()) {
        // Choosing through Settings after dismissing onboarding also completes setup.
        current.first_launch = false;
    }

    let app_dir = app.path().app_data_dir().unwrap().join("data");
    let saved = crate::autostart::save_change(
        startup_requested,
        || crate::autostart::is_enabled(&app),
        |enabled| crate::autostart::set_enabled(&app, enabled),
        || crate::settings::try_save_settings(&app_dir.join("settings.json"), &current),
    );
    if let Err(error) = saved {
        *current = previous;
        eprintln!("Failed to save settings: {error}");
        return false;
    }

    let new_lang = current.language.clone();
    let new_dict = current.active_dictionary.clone();
    let settings_json = serde_json::to_value(&*current).unwrap_or(Value::Null);
    if new_dict != old_dict {
        if let Some(words) = selected_words {
            let learned_indices = state.learned.lock().unwrap()
                .get(&new_dict).cloned().unwrap_or_default();
            let mut engine = state.word_engine.lock().unwrap();
            engine.load_dictionary(words);
            engine.set_learned(learned_indices);
        }
        *state.all_learned.lock().unwrap() = false;
        *state.all_dicts_done.lock().unwrap() = false;
    }
    drop(current);

    let _ = app.emit("update-settings", &settings_json);

    if new_lang != old_lang {
        {
            let mut i18n = state.i18n.lock().unwrap();
            i18n.set_locale(&new_lang);
            let _ = app.emit("locale-updated", serde_json::json!({
                "locale": i18n.current_locale,
                "strings": i18n.get_strings()
            }));
        }
        crate::tray::refresh_tray(&app);
    }

    if new_dict != old_dict {
        // After onboarding: dict was empty, now it has a value — show first card immediately
        if old_dict.is_empty() {
            let entry = state.word_engine.lock().unwrap().get_random_word();
            if let Some(word) = entry {
                if let Some(window) = app.get_webview_window("main") {
                    let pos = state.settings.lock().unwrap().position.clone();
                    crate::reposition_window(&window, &pos);
                }
                crate::timer::emit_word(&app, &word, &new_dict);
                // Tell timer to reset its interval — prevents it from also firing immediately
                state.manual_trigger.notify_one();
            }
        }
    }

    true
}

#[tauri::command]
pub fn complete_onboarding(app: AppHandle, state: State<'_, AppState>, dict_id: String) -> Result<(), String> {
    let words = load_dictionary_by_id(&app, &dict_id)
        .ok_or_else(|| "Dictionary not found or invalid".to_string())?;
    if dict_id.is_empty() || words.is_empty() {
        return Err("Dictionary is empty or invalid".into());
    }
    if !save_settings(app.clone(), state, serde_json::json!({
        "activeDictionary": dict_id,
        "firstLaunch": false
    })) {
        return Err("Failed to save dictionary selection".into());
    }
    // DOM window.close() only closes the WebView on Windows, leaving a topmost shell.
    if let Some(window) = app.get_webview_window("onboarding") {
        window.destroy().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn get_locale(state: State<'_, AppState>) -> Value {
    let i18n = state.i18n.lock().unwrap();
    serde_json::json!({
        "locale": i18n.current_locale,
        "strings": i18n.get_strings()
    })
}

// ── Stats ─────────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_stats(state: State<'_, AppState>) -> Value {
    let stats = state.stats_tracker.lock().unwrap();
    let settings = state.settings.lock().unwrap();
    let learned = state.learned.lock().unwrap();
    let engine = state.word_engine.lock().unwrap();

    let dict_id = &settings.active_dictionary;
    let learned_count = learned.get(dict_id).map(|v| v.len()).unwrap_or(0);

    let today_key = crate::stats::StatsTracker::today();
    let today = stats.stats.days.get(&today_key).cloned().unwrap_or_default();

    serde_json::json!({
        "today": today,
        "streak": stats.get_streak(),
        "last7": stats.get_last_7_days(),
        "learnedCount": learned_count,
        "totalWords": engine.dictionary.len()
    })
}

// ── Learned words ─────────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_learned_list(state: State<'_, AppState>) -> Value {
    let settings = state.settings.lock().unwrap();
    let dict_id = settings.active_dictionary.clone();
    let learned = state.learned.lock().unwrap();
    let engine = state.word_engine.lock().unwrap();

    let mut words = Vec::new();
    if let Some(indices) = learned.get(&dict_id) {
        for &idx in indices {
            if let Some(w) = engine.dictionary.get(idx) {
                words.push(serde_json::json!({
                    "term": w.term,
                    "definition": w.definition,
                    "index": idx
                }));
            }
        }
    }

    serde_json::json!({ "dictId": dict_id, "words": words })
}

#[tauri::command]
pub fn mark_learned(app: AppHandle, state: State<'_, AppState>, index: usize, dict_id: String) -> bool {
    let all_learned_now = {
        let settings = state.settings.lock().unwrap();
        let mut learned = state.learned.lock().unwrap();
        let mut engine = state.word_engine.lock().unwrap();
        if !can_mark_learned(&settings.active_dictionary, &dict_id, index, engine.dictionary.len(),
            learned.get(&dict_id).map(Vec::as_slice).unwrap_or_default()) {
            return false;
        }
        let indices = learned.entry(dict_id.clone()).or_default();
        indices.push(index);
        engine.set_learned(indices.clone());
        save_learned_to_disk(&app, &learned);
        let all_learned = engine.get_random_word().is_none();
        if all_learned { *state.all_learned.lock().unwrap() = true; }
        all_learned
    };

    // Record learned stat and persist
    {
        let mut tracker = state.stats_tracker.lock().unwrap();
        tracker.record_learned();
        save_stats_to_disk(&app, &tracker.stats);
    }

    let _ = app.emit("stats-updated", ());

    // Signal timer to handle the all-learned flow immediately
    if all_learned_now {
        state.all_learned_notify.notify_one();
    }

    true
}

fn can_mark_learned(active: &str, source: &str, index: usize, count: usize, learned: &[usize]) -> bool {
    !source.is_empty() && active == source && index < count && !learned.contains(&index)
}

#[cfg(test)]
mod mark_learned_tests {
    use super::can_mark_learned;

    #[test]
    fn accepts_unlearned_index_from_active_dictionary() {
        assert!(can_mark_learned("a", "a", 1, 2, &[0]));
    }

    #[test]
    fn rejects_old_dictionary_invalid_index_and_duplicate() {
        assert!(!can_mark_learned("b", "a", 0, 2, &[]));
        assert!(!can_mark_learned("a", "a", 2, 2, &[]));
        assert!(!can_mark_learned("a", "a", 0, 2, &[0]));
        assert!(!can_mark_learned("", "", 0, 2, &[]));
    }
}

#[tauri::command]
pub fn remove_learned(app: AppHandle, state: State<'_, AppState>, index: usize) -> bool {
    let dict_id = state.settings.lock().unwrap().active_dictionary.clone();

    {
        let mut learned = state.learned.lock().unwrap();
        if let Some(indices) = learned.get_mut(&dict_id) {
            indices.retain(|&i| i != index);
        }
        save_learned_to_disk(&app, &learned);
        let learned_indices = learned.get(&dict_id).cloned().unwrap_or_default();
        state.word_engine.lock().unwrap().set_learned(learned_indices);
    }

    {
        let mut tracker = state.stats_tracker.lock().unwrap();
        tracker.decrement_learned();
        save_stats_to_disk(&app, &tracker.stats);
    }

    // Words are available again — clear both flags
    *state.all_learned.lock().unwrap() = false;
    *state.all_dicts_done.lock().unwrap() = false;

    let _ = app.emit("stats-updated", ());
    true
}

#[tauri::command]
pub fn clear_learned(app: AppHandle, state: State<'_, AppState>) -> bool {
    let dict_id = state.settings.lock().unwrap().active_dictionary.clone();

    let count = {
        let mut learned = state.learned.lock().unwrap();
        let count = learned.get(&dict_id).map(|v| v.len()).unwrap_or(0);
        if let Some(indices) = learned.get_mut(&dict_id) {
            indices.clear();
        }
        save_learned_to_disk(&app, &learned);
        state.word_engine.lock().unwrap().set_learned(Vec::new());
        count
    };

    {
        let mut tracker = state.stats_tracker.lock().unwrap();
        for _ in 0..count {
            tracker.decrement_learned();
        }
        save_stats_to_disk(&app, &tracker.stats);
    }

    *state.all_learned.lock().unwrap() = false;
    *state.all_dicts_done.lock().unwrap() = false;
    let _ = app.emit("stats-updated", ());
    true
}

// ── Dictionaries ──────────────────────────────────────────────────────────────

fn get_user_dicts_dir(app: &AppHandle) -> PathBuf {
    let dir = app.path().app_data_dir().unwrap().join("dictionaries");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub(crate) fn get_bundled_dicts_dir(app: &AppHandle) -> PathBuf {
    let mut dir = app.path().resolve("dictionaries", tauri::path::BaseDirectory::Resource).unwrap_or_default();
    if !dir.exists() {
        let mut p = std::env::current_dir().unwrap();
        p.push("../src/dictionaries");
        dir = p;
    }
    dir
}

pub fn dictionary_display_name(app: &AppHandle, id: &str) -> String {
    if id == crate::quick_add::DECK_ID {
        return app.state::<AppState>().i18n.lock().unwrap().t("quickAdd.deck").to_string();
    }
    id.replace('_', " ").to_uppercase()
}

fn scan_dicts(dir: &PathBuf, source: &str) -> Vec<Value> {
    let mut dicts = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if ext == "csv" || ext == "json" || ext == "xlsx" {
                        let id = path.file_stem().unwrap().to_string_lossy().to_string();
                        if crate::quick_add::is_reserved(&id) { continue; }
                        let name = id.replace('_', " ").to_uppercase();
                        dicts.push(serde_json::json!({
                            "id": id,
                            "name": name,
                            "source": source
                        }));
                    }
                }
            }
        }
    }
    dicts
}

pub fn load_dictionary_by_id(app: &AppHandle, id: &str) -> Option<Vec<crate::word_engine::WordEntry>> {
    if crate::quick_add::is_reserved(id) {
        return crate::quick_add::load_deck(app);
    }
    let user_dir = get_user_dicts_dir(app);
    let bundled_dir = get_bundled_dicts_dir(app);
    for dir in &[&user_dir, &bundled_dir] {
        for ext in &["csv", "json", "xlsx"] {
            let path = dir.join(format!("{}.{}", id, ext));
            if path.exists() {
                if let Ok(words) = crate::dictionary_loader::load_dictionary(&path) {
                    return Some(words);
                }
            }
        }
    }
    None
}

/// Returns all dictionary IDs from both bundled and user directories.
pub fn get_all_dict_ids(app: &AppHandle) -> Vec<String> {
    let mut ids = Vec::new();
    for dir in &[get_bundled_dicts_dir(app), get_user_dicts_dir(app)] {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if ["csv", "json", "xlsx"].contains(&ext) {
                            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                                if !crate::quick_add::is_reserved(stem) {
                                    ids.push(stem.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if crate::quick_add::has_deck(app) {
        ids.push(crate::quick_add::DECK_ID.into());
    }
    ids
}

#[tauri::command]
pub fn get_dictionaries(app: AppHandle, _state: State<'_, AppState>) -> Value {
    let mut all = Vec::new();
    all.extend(scan_dicts(&get_bundled_dicts_dir(&app), "bundled"));
    all.extend(scan_dicts(&get_user_dicts_dir(&app), "user"));
    if crate::quick_add::has_deck(&app) {
        let id = crate::quick_add::DECK_ID;
        all.push(serde_json::json!({
            "id": id,
            "name": dictionary_display_name(&app, id),
            "source": "quick-add"
        }));
    }
    serde_json::json!(all)
}

#[tauri::command]
pub async fn import_dictionary(app: AppHandle, window: tauri::WebviewWindow) -> Value {
    tauri::async_runtime::spawn_blocking(move || import_from_picker(&app, &window))
        .await.unwrap_or_else(|error| serde_json::json!({ "error": error.to_string() }))
}

fn import_from_picker(app: &AppHandle, window: &tauri::WebviewWindow) -> Value {
    let picker = app.dialog().file().set_parent(window)
        .add_filter("Dictionary", &["csv", "json", "xlsx"]);
    if let Some(file_path) = picker.blocking_pick_file() {
        let path = file_path.into_path().unwrap();

        let words = match crate::dictionary_loader::load_dictionary(&path) {
            Ok(w) => w,
            Err(e) => return serde_json::json!({ "error": e }),
        };

        if words.is_empty() {
            return serde_json::json!({ "error": "Dictionary is empty or invalid" });
        }

        let user_dir = get_user_dicts_dir(&app);
        let id = path.file_stem().unwrap().to_string_lossy().to_string();
        if crate::quick_add::is_reserved(&id) {
            return serde_json::json!({ "error": "Dictionary name is reserved for Quick Add" });
        }
        let target_path = user_dir.join(path.file_name().unwrap());

        if target_path.exists() {
            return serde_json::json!({ "error": "Dictionary with this name already exists" });
        }

        if let Err(e) = fs::copy(&path, &target_path) {
            return serde_json::json!({ "error": format!("Failed to copy file: {}", e) });
        }

        let _ = app.emit("dict-list-updated", ());
        return serde_json::json!({ "id": id, "wordCount": words.len() });
    }

    serde_json::json!(null)
}

#[tauri::command]
pub fn delete_dictionary(app: AppHandle, state: State<'_, AppState>, id: String) -> Value {
    if crate::quick_add::is_reserved(&id) {
        return serde_json::json!({ "error": "My Words is managed by Quick Add" });
    }
    let user_dir = get_user_dicts_dir(&app);
    let mut deleted = false;

    for ext in ["csv", "json", "xlsx"] {
        let path = user_dir.join(format!("{}.{}", id, ext));
        if path.exists() {
            if let Err(e) = fs::remove_file(path) {
                return serde_json::json!({ "error": format!("Failed to delete: {}", e) });
            }
            deleted = true;
            break;
        }
    }

    if !deleted {
        return serde_json::json!({ "error": "Dictionary not found or cannot be deleted" });
    }

    let (new_active, dict_switched) = {
        let mut settings = state.settings.lock().unwrap();
        let switched = settings.active_dictionary == id;
        if switched {
            // Fall back to first still-available dictionary, or empty string
            let fallback = get_all_dict_ids(&app).into_iter().next().unwrap_or_default();
            settings.active_dictionary = fallback;
            let path = app.path().app_data_dir().unwrap().join("data").join("settings.json");
            crate::settings::save_settings(&path, &settings);
        }
        (settings.active_dictionary.clone(), switched)
    };

    // Remove learned data for deleted dictionary and persist
    {
        let mut learned = state.learned.lock().unwrap();
        learned.remove(&id);
        save_learned_to_disk(&app, &learned);
    }

    // If the active dictionary was deleted, reload engine with fallback
    if dict_switched {
        if let Some(words) = load_dictionary_by_id(&app, &new_active) {
            let learned_indices = state.learned.lock().unwrap()
                .get(&new_active).cloned().unwrap_or_default();
            let mut engine = state.word_engine.lock().unwrap();
            engine.load_dictionary(words);
            engine.set_learned(learned_indices);
        }
        *state.all_learned.lock().unwrap() = false;
        *state.all_dicts_done.lock().unwrap() = false;
    }

    serde_json::json!({ "success": true, "newActive": new_active })
}

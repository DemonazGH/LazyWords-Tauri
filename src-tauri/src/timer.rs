use tauri::{AppHandle, Manager};
use std::time::Duration;
use crate::state::AppState;
use tauri::Emitter;

// ── Fullscreen detection (Windows only) ──────────────────────────────────────

#[cfg(target_os = "windows")]
fn is_fullscreen_app_active() -> bool {
    use winapi::um::winuser::{
        GetForegroundWindow, GetWindowRect, GetDesktopWindow,
        MonitorFromWindow, GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use winapi::shared::windef::RECT;

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() { return false; }
        // Skip the desktop itself
        if hwnd == GetDesktopWindow() { return false; }

        let mut win_rect: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut win_rect) == 0 { return false; }

        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if monitor.is_null() { return false; }

        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut mi as *mut MONITORINFO) == 0 { return false; }

        let mr = mi.rcMonitor;
        win_rect.left == mr.left
            && win_rect.top == mr.top
            && win_rect.right == mr.right
            && win_rect.bottom == mr.bottom
    }
}

#[cfg(not(target_os = "windows"))]
fn is_fullscreen_app_active() -> bool { false }

// ── i18n helper ──────────────────────────────────────────────────────────────

fn t(app: &AppHandle, key: &'static str) -> String {
    let app_state = app.state::<AppState>();
    let i18n = app_state.i18n.lock().unwrap();
    i18n.t(key).to_string()
}

// ── All-learned handler ───────────────────────────────────────────────────────

/// Returns `true` if a new dictionary was found and switched to.
async fn handle_all_learned(app: &AppHandle) -> bool {
    // Clear the pending flag immediately so re-entrance is safe
    *app.state::<AppState>().all_learned.lock().unwrap() = false;

    let current_dict_id = app.state::<AppState>().settings.lock().unwrap().active_dictionary.clone();
    let dict_name = current_dict_id.replace('_', " ").to_uppercase();

    // Show "all words learned" card for current dictionary
    let _ = app.emit("show-all-learned", serde_json::json!({
        "dictName": dict_name,
        "headline": t(app, "card.allLearned"),
        "dictLabel": t(app, "card.dictLabel")
    }));

    let show_secs = app.state::<AppState>().settings.lock().unwrap().show_duration;
    let fade_secs = app.state::<AppState>().settings.lock().unwrap().fade_duration;
    tokio::time::sleep(Duration::from_secs_f32(show_secs)).await;
    let _ = app.emit("hide-word", ());
    // Wait for the CSS fade-out to complete before showing the next card
    tokio::time::sleep(Duration::from_secs_f32(fade_secs)).await;

    // Try to find another dictionary with unlearned words
    let all_dicts = crate::commands::get_all_dict_ids(app);
    let learned_map = app.state::<AppState>().learned.lock().unwrap().clone();

    let mut switched = false;
    for dict_id in &all_dicts {
        if *dict_id == current_dict_id { continue; }

        if let Some(words) = crate::commands::load_dictionary_by_id(app, dict_id) {
            let learned_set: std::collections::HashSet<usize> =
                learned_map.get(dict_id).cloned().unwrap_or_default().into_iter().collect();
            let has_unlearned = words.iter().enumerate().any(|(i, _)| !learned_set.contains(&i));

            if has_unlearned {
                let new_dict_name = dict_id.replace('_', " ").to_uppercase();

                // Update settings
                {
                    let app_state = app.state::<AppState>();
                    let mut settings = app_state.settings.lock().unwrap();
                    settings.active_dictionary = dict_id.clone();
                    let path = app.path().app_data_dir().unwrap().join("data").join("settings.json");
                    crate::settings::save_settings(&path, &settings);
                }

                // Load new dictionary into engine
                {
                    let app_state = app.state::<AppState>();
                    let mut engine = app_state.word_engine.lock().unwrap();
                    engine.load_dictionary(words);
                    let learned_indices = learned_map.get(dict_id).cloned().unwrap_or_default();
                    engine.set_learned(learned_indices);
                }

                // Notify settings window and card
                let _ = app.emit("dict-auto-switched", serde_json::json!({ "newId": dict_id }));
                let _ = app.emit("show-switched-dict", serde_json::json!({
                    "dictName": new_dict_name,
                    "headline": t(app, "card.switchedTo")
                }));

                let show_secs = app.state::<AppState>().settings.lock().unwrap().show_duration;
                let fade_secs = app.state::<AppState>().settings.lock().unwrap().fade_duration;
                tokio::time::sleep(Duration::from_secs_f32(show_secs)).await;
                let _ = app.emit("hide-word", ());
                tokio::time::sleep(Duration::from_secs_f32(fade_secs)).await;

                switched = true;
                break;
            }
        }
    }

    if !switched {
        // Every dictionary is fully learned — enter idle state
        *app.state::<AppState>().all_dicts_done.lock().unwrap() = true;

        let _ = app.emit("show-all-dicts-learned", serde_json::json!({
            "headline": t(app, "card.allDictsLearned"),
            "sub": t(app, "card.restoreInSettings")
        }));
        let show_secs = app.state::<AppState>().settings.lock().unwrap().show_duration;
        tokio::time::sleep(Duration::from_secs_f32(show_secs)).await;
        let _ = app.emit("hide-word", ());
    }

    switched
}

// ── Main loop ─────────────────────────────────────────────────────────────────

pub fn start_loop(app_handle: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut skip_interval = true; // show first card immediately on startup
        loop {
            // ── Idle: no dictionary selected yet (onboarding) ─────────────────
            if app_handle.state::<AppState>().settings.lock().unwrap().active_dictionary.is_empty() {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }

            // ── Idle: all dictionaries exhausted ──────────────────────────────
            if *app_handle.state::<AppState>().all_dicts_done.lock().unwrap() {
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }

            // ── Pending all-learned event (set by mark_learned or previous iter) ──
            if *app_handle.state::<AppState>().all_learned.lock().unwrap() {
                let switched = handle_all_learned(&app_handle).await;
                if switched { skip_interval = true; }
                continue;
            }

            // ── Wait for the configured interval (polls every 1 s) ────────────
            if skip_interval {
                skip_interval = false;
            } else {
                let mut waited = 0.0f32;
                loop {
                    let interval_secs = app_handle.state::<AppState>().settings.lock().unwrap().interval * 60.0;
                    if waited >= interval_secs { break; }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    waited += 1.0;
                }
            }

            // Skip if paused
            if *app_handle.state::<AppState>().is_paused.lock().unwrap() { continue; }

            // Skip if a fullscreen application is active
            if is_fullscreen_app_active() { continue; }

            // Fetch random word
            let entry = app_handle.state::<AppState>().word_engine.lock().unwrap().get_random_word();

            if let Some(word) = entry {
                // Set current word in state (used by Ctrl+Shift+K guard in card.js)
                *app_handle.state::<AppState>().current_word.lock().unwrap() = Some(word.clone());
                *app_handle.state::<AppState>().is_card_visible.lock().unwrap() = true;

                // Reposition card to the monitor under the cursor
                if let Some(window) = app_handle.get_webview_window("main") {
                    let pos = app_handle.state::<AppState>().settings.lock().unwrap().position.clone();
                    crate::reposition_window(&window, &pos);
                }

                // Record shown stat and persist
                {
                    let app_state = app_handle.state::<AppState>();
                    let mut tracker = app_state.stats_tracker.lock().unwrap();
                    tracker.record_shown();
                    crate::commands::save_stats_to_disk(&app_handle, &tracker.stats);
                }
                let _ = app_handle.emit("stats-updated", ());

                // Show word (include index so card.js can pass it back for mark_learned)
                let _ = app_handle.emit("show-word", serde_json::json!({
                    "term": word.entry.term,
                    "definition": word.entry.definition,
                    "index": word.index
                }));

                // Hide after show_duration — but wake early if all_learned is signalled
                let show_secs = app_handle.state::<AppState>().settings.lock().unwrap().show_duration;
                let app_state = app_handle.state::<AppState>();
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs_f32(show_secs)) => {}
                    _ = app_state.all_learned_notify.notified() => {}
                }
                let _ = app_handle.emit("hide-word", ());

                *app_handle.state::<AppState>().is_card_visible.lock().unwrap() = false;
                *app_handle.state::<AppState>().current_word.lock().unwrap() = None;

                // If mark_learned signalled all_learned, handle it immediately
                if *app_handle.state::<AppState>().all_learned.lock().unwrap() {
                    let switched = handle_all_learned(&app_handle).await;
                    if switched { skip_interval = true; }
                }

            } else {
                // No unlearned words — set flag; top of loop will call handle_all_learned
                *app_handle.state::<AppState>().all_learned.lock().unwrap() = true;
            }
        }
    });
}

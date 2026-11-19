use tauri::{AppHandle, Manager};
use std::time::Duration;
use crate::state::AppState;
use crate::word_engine::WordWithIndex;
use tauri::Emitter;
use std::sync::Mutex;
use tokio::sync::Notify;

#[derive(Debug, PartialEq, Eq)]
enum IntervalEvent {
    Tick,
    Manual,
    Completion,
}

async fn wait_for_completion(pending: &Mutex<bool>, notify: &Notify) {
    loop {
        let notification = notify.notified();
        tokio::pin!(notification);
        // Register before checking the flag so a concurrent notification cannot be missed.
        notification.as_mut().enable();
        if *pending.lock().unwrap() {
            return;
        }
        // A notification left over from a completed/cancelled event must not hide a new card.
        notification.await;
    }
}

async fn wait_for_interval_event(
    pending: &Mutex<bool>,
    completion: &Notify,
    manual: &Notify,
    tick: Duration,
) -> IntervalEvent {
    tokio::select! {
        biased;
        _ = wait_for_completion(pending, completion) => IntervalEvent::Completion,
        _ = manual.notified() => IntervalEvent::Manual,
        _ = tokio::time::sleep(tick) => IntervalEvent::Tick,
    }
}

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

pub fn emit_word(app: &AppHandle, word: &WordWithIndex, dict_id: &str) -> Duration {
    let (show_duration, answer_delay) = {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().unwrap();
        (settings.show_duration, settings.answer_delay)
    };
    // The renderer uses the same timing snapshot and owns the visible lifetime.
    let _ = app.emit("show-word", serde_json::json!({
        "term": word.entry.term,
        "definition": word.entry.definition,
        "cardId": word.card_id,
        "dictId": dict_id,
        "showDuration": show_duration,
        "answerDelay": answer_delay
    }));
    Duration::from_secs_f32(show_duration.max(0.0)) + Duration::from_secs(answer_delay.into())
}

// ── All-learned handler ───────────────────────────────────────────────────────

/// Returns `true` if a new dictionary was found and switched to.
async fn handle_all_learned(app: &AppHandle) -> bool {
    // Clear the pending flag immediately so re-entrance is safe
    *app.state::<AppState>().all_learned.lock().unwrap() = false;

    let current_dict_id = app.state::<AppState>().settings.lock().unwrap().active_dictionary.clone();
    let dict_name = crate::commands::dictionary_display_name(app, &current_dict_id);

    // Show "all words learned" card for current dictionary
    let _ = app.emit("show-all-learned", serde_json::json!({
        "dictName": dict_name,
        "headline": t(app, "card.allLearned"),
        "dictLabel": t(app, "card.dictLabel")
    }));

    let show_secs = app.state::<AppState>().settings.lock().unwrap().show_duration;
    let fade_secs = app.state::<AppState>().settings.lock().unwrap().fade_duration;
    tokio::time::sleep(Duration::from_secs_f32(show_secs)).await;
    // Wait for the CSS fade-out to complete before showing the next card
    tokio::time::sleep(Duration::from_secs_f32(fade_secs)).await;

    // Try to find another dictionary with unlearned words
    let all_dicts = crate::commands::get_all_dict_ids(app);
    let learned_map = app.state::<AppState>().learned.lock().unwrap().clone();

    let mut switched = false;
    for dict_id in &all_dicts {
        if *dict_id == current_dict_id { continue; }

        if let Some(words) = crate::commands::load_dictionary_by_id(app, dict_id) {
            let learned_set: std::collections::HashSet<&str> =
                learned_map.ids(dict_id).iter().map(String::as_str).collect();
            let has_unlearned = words.iter().any(|word| !learned_set.contains(crate::scheduler::card_id(word).as_str()));

            if has_unlearned {
                let new_dict_name = crate::commands::dictionary_display_name(app, dict_id);

                // Update settings
                {
                    let app_state = app.state::<AppState>();
                    let mut settings = app_state.settings.lock().unwrap();
                    let learned = app_state.learned.lock().unwrap();
                    let mut engine = app_state.word_engine.lock().unwrap();
                    engine.load_dictionary(words);
                    engine.set_learned_ids(learned.ids(dict_id));
                    settings.active_dictionary = dict_id.clone();
                    let path = app.path().app_data_dir().unwrap().join("data").join("settings.json");
                    crate::settings::save_settings(&path, &settings);
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
    }

    switched
}

// ── Main loop ─────────────────────────────────────────────────────────────────

pub fn start_loop(app_handle: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut skip_interval = true; // show first card immediately on startup
        'timer: loop {
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
                    let app_state = app_handle.state::<AppState>();
                    match wait_for_interval_event(
                        &app_state.all_learned,
                        &app_state.all_learned_notify,
                        &app_state.manual_trigger,
                        Duration::from_secs(1),
                    ).await {
                        IntervalEvent::Tick => waited += 1.0,
                        // Manual cards are already shown; completion is handled at the top.
                        IntervalEvent::Manual | IntervalEvent::Completion => continue 'timer,
                    }
                }
            }

            // Skip if paused
            if *app_handle.state::<AppState>().is_paused.lock().unwrap() { continue; }

            // Skip if a fullscreen application is active
            if is_fullscreen_app_active() { continue; }

            let (dict_id, entry) = crate::scheduler::select_next(&app_handle.state::<AppState>());

            if let Some(word) = entry {
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

                let duration = emit_word(&app_handle, &word, &dict_id);
                let app_state = app_handle.state::<AppState>();
                tokio::select! {
                    biased;
                    _ = wait_for_completion(&app_state.all_learned, &app_state.all_learned_notify) => {}
                    _ = app_state.manual_trigger.notified() => {}
                    _ = tokio::time::sleep(duration) => {}
                }

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

#[cfg(test)]
mod tests {
    use super::*;

    const LONG_WAIT: Duration = Duration::from_secs(30 * 60);
    const TEST_TIMEOUT: Duration = Duration::from_millis(100);

    #[tokio::test]
    async fn completion_interrupts_interval_after_wait_has_started() {
        let pending = Mutex::new(false);
        let completion = Notify::new();
        let manual = Notify::new();
        let wait = wait_for_interval_event(&pending, &completion, &manual, LONG_WAIT);
        tokio::pin!(wait);
        tokio::select! {
            biased;
            event = &mut wait => panic!("unexpected event: {event:?}"),
            _ = async {} => {}
        }

        *pending.lock().unwrap() = true;
        completion.notify_one();
        assert_eq!(tokio::time::timeout(TEST_TIMEOUT, wait).await.unwrap(), IntervalEvent::Completion);
    }

    #[tokio::test]
    async fn completion_has_priority_over_a_pending_manual_trigger() {
        let pending = Mutex::new(true);
        let completion = Notify::new();
        let manual = Notify::new();
        completion.notify_one();
        manual.notify_one();

        let event = wait_for_interval_event(&pending, &completion, &manual, LONG_WAIT).await;
        assert_eq!(event, IntervalEvent::Completion);
    }

    #[tokio::test]
    async fn pending_completion_does_not_require_an_unconsumed_notification() {
        let pending = Mutex::new(true);
        let completion = Notify::new();
        let manual = Notify::new();

        let event = tokio::time::timeout(TEST_TIMEOUT,
            wait_for_interval_event(&pending, &completion, &manual, LONG_WAIT)).await.unwrap();
        assert_eq!(event, IntervalEvent::Completion);
    }

    #[tokio::test]
    async fn manual_next_still_interrupts_the_interval() {
        let pending = Mutex::new(false);
        let completion = Notify::new();
        let manual = Notify::new();
        manual.notify_one();

        assert_eq!(wait_for_interval_event(&pending, &completion, &manual, LONG_WAIT).await,
            IntervalEvent::Manual);
    }

    #[tokio::test]
    async fn stale_completion_does_not_interrupt_an_interval_tick() {
        let pending = Mutex::new(false);
        let completion = Notify::new();
        let manual = Notify::new();
        completion.notify_one();

        assert_eq!(wait_for_interval_event(&pending, &completion, &manual, Duration::ZERO).await,
            IntervalEvent::Tick);
    }

    #[tokio::test]
    async fn stale_notification_does_not_hide_a_restored_card() {
        let pending = Mutex::new(true);
        let completion = Notify::new();
        // Completion can be handled from its flag before mark_learned sends the wakeup.
        wait_for_completion(&pending, &completion).await;
        *pending.lock().unwrap() = false;
        completion.notify_one();

        let wait = wait_for_completion(&pending, &completion);
        tokio::pin!(wait);
        tokio::select! {
            biased;
            _ = &mut wait => panic!("stale completion hid the restored card"),
            _ = async {} => {}
        }
        *pending.lock().unwrap() = true;
        completion.notify_one();
        tokio::time::timeout(TEST_TIMEOUT, wait).await.unwrap();
    }

    #[tokio::test]
    async fn consumed_completion_does_not_wake_the_next_card() {
        let pending = Mutex::new(true);
        let completion = Notify::new();
        completion.notify_one();
        wait_for_completion(&pending, &completion).await;
        *pending.lock().unwrap() = false;

        tokio::select! {
            biased;
            _ = wait_for_completion(&pending, &completion) => panic!("completion woke twice"),
            _ = async {} => {}
        }
    }
}

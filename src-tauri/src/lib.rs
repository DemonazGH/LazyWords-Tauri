mod settings;
mod stats;
mod word_engine;
mod dictionary_loader;
mod state;
mod commands;
mod i18n;
mod timer;
mod tray;

use tauri::Manager;
use tauri::Emitter;

// ── Window positioning (multi-monitor aware) ──────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn reposition_window(window: &tauri::WebviewWindow, position: &str) {
    let win_size = window.inner_size().unwrap_or(tauri::PhysicalSize::new(500, 110));
    let width = win_size.width as i32;
    let height = win_size.height as i32;

    // Prefer the monitor that contains the cursor; fall back to the window's current monitor
    let monitor = {
        let cursor = window.cursor_position().ok();
        if let Some(pos) = cursor {
            window.available_monitors()
                .unwrap_or_default()
                .into_iter()
                .find(|m| {
                    let mp = m.position();
                    let ms = m.size();
                    pos.x >= mp.x as f64
                        && pos.x < (mp.x + ms.width as i32) as f64
                        && pos.y >= mp.y as f64
                        && pos.y < (mp.y + ms.height as i32) as f64
                })
        } else {
            None
        }
    }
    .or_else(|| window.current_monitor().ok().flatten());

    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let mp = monitor.position();
        let ms = monitor.size();
        let screen_w = ms.width as i32;
        let screen_h = ms.height as i32;
        let origin_x = mp.x;
        let origin_y = mp.y;

        let padding = (20.0 * scale) as i32;

        let actual_pos = if position == "random" {
            let options = ["top-left", "top-center", "top-right", "bottom-left", "bottom-right", "center"];
            use std::time::{SystemTime, UNIX_EPOCH};
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos() as usize;
            let hash = nanos.wrapping_mul(1103515245).wrapping_add(12345) >> 16;
            options[hash % options.len()]
        } else {
            position
        };

        let (rel_x, rel_y) = match actual_pos {
            "top-left"     => (padding, padding),
            "top-center"   => ((screen_w - width) / 2, padding),
            "top-right"    => (screen_w - width - padding, padding),
            "bottom-left"  => (padding, screen_h - height - padding),
            "bottom-right" => (screen_w - width - padding, screen_h - height - padding),
            _              => ((screen_w - width) / 2, (screen_h - height) / 2),
        };

        let _ = window.set_position(tauri::Position::Physical(
            tauri::PhysicalPosition::new(origin_x + rel_x, origin_y + rel_y),
        ));
    }
}

// ── Single-instance lock (Windows: named mutex) ───────────────────────────────

#[cfg(target_os = "windows")]
fn acquire_single_instance_lock() -> bool {
    use winapi::um::synchapi::CreateMutexW;
    use winapi::um::errhandlingapi::GetLastError;
    use winapi::shared::winerror::ERROR_ALREADY_EXISTS;

    let name: Vec<u16> = "com.dzmitry.lazywords.single-instance\0"
        .encode_utf16()
        .collect();

    unsafe {
        CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr());
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}

#[cfg(not(target_os = "windows"))]
fn acquire_single_instance_lock() -> bool { true }

// ── App entry point ───────────────────────────────────────────────────────────

pub fn run() {
    // Exit immediately if another instance is already running
    if !acquire_single_instance_lock() {
        eprintln!("LazyWords is already running.");
        std::process::exit(0);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let state = state::init_state(&app.handle());

            // Load active dictionary + apply persisted learned indices.
            // If the saved dict ID can't be found (e.g. renamed file), fall back to the
            // first available dictionary and persist the corrected setting.
            let active_dict = {
                let saved = state.settings.lock().unwrap().active_dictionary.clone();
                if !saved.is_empty() && commands::load_dictionary_by_id(&app.handle(), &saved).is_none() {
                    // Saved dict missing — migrate to first available
                    let fallback = commands::get_all_dict_ids(&app.handle())
                        .into_iter().next().unwrap_or_default();
                    if !fallback.is_empty() {
                        let mut settings = state.settings.lock().unwrap();
                        settings.active_dictionary = fallback.clone();
                        let path = app.path().app_data_dir().unwrap().join("data").join("settings.json");
                        crate::settings::save_settings(&path, &settings);
                    }
                    fallback
                } else {
                    saved
                }
            };
            if let Some(words) = commands::load_dictionary_by_id(&app.handle(), &active_dict) {
                let mut engine = state.word_engine.lock().unwrap();
                engine.load_dictionary(words);
                let learned_indices = state.learned.lock().unwrap()
                    .get(&active_dict).cloned().unwrap_or_default();
                engine.set_learned(learned_indices);
            }

            let first_launch = state.settings.lock().unwrap().first_launch;
            app.manage(state);

            // ── System tray ──
            {
                let tray_icon = tray::create_tray(&app.handle())?;
                let app_state = app.state::<crate::state::AppState>();
                *app_state.tray.lock().unwrap() = Some(tray_icon);
            }

            // ── Card window (small transparent overlay) ──
            let card_window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("windows/card/card.html".into()),
            )
            .title("LazyWords")
            .transparent(true)
            .shadow(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .inner_size(500.0, 110.0)
            .center()
            .build()
            .unwrap();

            let _ = card_window.set_ignore_cursor_events(true);

            // ── Onboarding window on first launch ──
            if first_launch {
                // Mark first_launch = false immediately so it never repeats
                {
                    let app_state = app.state::<crate::state::AppState>();
                    let mut settings = app_state.settings.lock().unwrap();
                    settings.first_launch = false;
                    let path = app.path().app_data_dir().unwrap().join("data").join("settings.json");
                    crate::settings::save_settings(&path, &settings);
                }

                let _ = tauri::WebviewWindowBuilder::new(
                    app,
                    "onboarding",
                    tauri::WebviewUrl::App("windows/onboarding/onboarding.html".into()),
                )
                .title("LazyWords")
                .inner_size(420.0, 320.0)
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .center()
                .resizable(false)
                .build();
            }

            // ── Global shortcuts ──
            use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
            use std::str::FromStr;

            let ctrl_shift_w = Shortcut::from_str("ctrl+shift+w").unwrap();
            let ctrl_shift_p = Shortcut::from_str("ctrl+shift+p").unwrap();
            let ctrl_shift_n = Shortcut::from_str("ctrl+shift+n").unwrap();
            let ctrl_shift_k = Shortcut::from_str("ctrl+shift+k").unwrap();

            let shortcuts = [
                ctrl_shift_w.clone(),
                ctrl_shift_p.clone(),
                ctrl_shift_n.clone(),
                ctrl_shift_k.clone(),
            ];

            for sc in shortcuts {
                let w = ctrl_shift_w.clone();
                let p = ctrl_shift_p.clone();
                let n = ctrl_shift_n.clone();
                let k = ctrl_shift_k.clone();

                app.handle().global_shortcut().on_shortcut(sc, move |app, shortcut, event| {
                    if event.state != ShortcutState::Pressed { return; }

                    if shortcut == &w {
                        // Dismiss onboarding if it's open
                        if let Some(ob) = app.get_webview_window("onboarding") {
                            let _ = ob.close();
                        }
                        if let Some(window) = app.get_webview_window("settings") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                let _ = window.unminimize();
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        } else {
                            let _ = tauri::WebviewWindowBuilder::new(
                                app,
                                "settings",
                                tauri::WebviewUrl::App("windows/settings/settings.html".into()),
                            )
                            .title("LazyWords Settings")
                            .inner_size(800.0, 600.0)
                            .shadow(false)
                            .focused(true)
                            .build();
                        }

                    } else if shortcut == &p {
                        // Dismiss onboarding
                        if let Some(ob) = app.get_webview_window("onboarding") {
                            let _ = ob.close();
                        }
                        {
                            let state = app.state::<crate::state::AppState>();
                            let mut paused = state.is_paused.lock().unwrap();
                            *paused = !*paused;
                        }
                        tray::refresh_tray(app);

                    } else if shortcut == &n {
                        // Dismiss onboarding; implicit unpause
                        if let Some(ob) = app.get_webview_window("onboarding") {
                            let _ = ob.close();
                        }
                        let state = app.state::<crate::state::AppState>();
                        // Implicit unpause so timer resumes
                        *state.is_paused.lock().unwrap() = false;

                        let entry = state.word_engine.lock().unwrap().get_random_word();
                        if let Some(word) = entry {
                            *state.current_word.lock().unwrap() = Some(word.clone());
                            *state.is_card_visible.lock().unwrap() = true;
                            if let Some(window) = app.get_webview_window("main") {
                                let pos = state.settings.lock().unwrap().position.clone();
                                reposition_window(&window, &pos);
                            }
                            let _ = app.emit("show-word", serde_json::json!({
                                "term": word.entry.term,
                                "definition": word.entry.definition,
                                "index": word.index
                            }));
                        }

                    } else if shortcut == &k {
                        // Dismiss onboarding; card.js handles the actual mark-learned
                        if let Some(ob) = app.get_webview_window("onboarding") {
                            let _ = ob.close();
                        }
                        let _ = app.emit("mark-known-shortcut", ());
                    }
                }).unwrap();
            }

            // Start background timer loop
            timer::start_loop(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::get_locale,
            commands::get_stats,
            commands::get_learned_list,
            commands::mark_learned,
            commands::remove_learned,
            commands::clear_learned,
            commands::get_dictionaries,
            commands::import_dictionary,
            commands::delete_dictionary
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

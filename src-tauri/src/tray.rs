use tauri::{
    AppHandle, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{TrayIcon, TrayIconBuilder},
    image::Image,
};
use crate::state::AppState;

// ── Icon helpers ───────────────────────────────────────────────────────────────

/// Load icon.png from the bundle resources (or the dev source tree).
fn load_normal_icon(app: &AppHandle) -> Option<Image<'static>> {
    // Try resource bundle first (production)
    if let Ok(path) = app.path().resolve("icons/icon.png", tauri::path::BaseDirectory::Resource) {
        if path.exists() {
            if let Ok(img) = Image::from_path(&path) {
                return Some(img);
            }
        }
    }
    // Dev fallback — icons/ lives next to src-tauri/
    let mut p = std::env::current_dir().unwrap_or_default();
    p.push("icons/icon.png");
    if p.exists() {
        if let Ok(img) = Image::from_path(&p) {
            return Some(img);
        }
    }
    None
}

/// Build a dimmed (paused) icon by converting icon.png pixels to grayscale.
fn load_paused_icon(app: &AppHandle) -> Option<Image<'static>> {
    // Try resource bundle first
    let path_opt = app
        .path()
        .resolve("icons/icon.png", tauri::path::BaseDirectory::Resource)
        .ok()
        .filter(|p| p.exists())
        .or_else(|| {
            let mut p = std::env::current_dir().unwrap_or_default();
            p.push("icons/icon.png");
            p.exists().then_some(p)
        });

    let path = path_opt?;
    let bytes = std::fs::read(&path).ok()?;

    // Decode PNG to raw RGBA pixels
    let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let width = info.width;
    let height = info.height;

    // Convert to grayscale-ish with reduced opacity so it looks "dimmed"
    let raw: Vec<u8> = buf[..info.buffer_size()]
        .chunks(4)
        .flat_map(|px| {
            let gray = (px[0] as u32 * 77 + px[1] as u32 * 150 + px[2] as u32 * 29) as u8;
            let alpha = (px[3] as u32 * 128 / 255) as u8; // 50 % opacity
            [gray, gray, gray, alpha]
        })
        .collect();

    Some(Image::new_owned(raw, width, height))
}

// ── Menu builder ───────────────────────────────────────────────────────────────

/// Rebuild the tray menu from scratch; called on creation and whenever state changes.
pub fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let is_paused = *app.state::<AppState>().is_paused.lock().unwrap();
    let active_dict = app.state::<AppState>().settings.lock().unwrap().active_dictionary.clone();

    // Collect translated strings while holding the lock, then drop it before building the menu.
    let (lbl_pause, lbl_resume, lbl_next, lbl_dict, lbl_settings, lbl_quit) = {
        let app_state = app.state::<AppState>();
        let i18n = app_state.i18n.lock().unwrap();
        (
            format!("⏸ {}", i18n.t("tray.pause")),
            format!("▶ {}", i18n.t("tray.resume")),
            format!("⏭ {}", i18n.t("tray.next")),
            i18n.t("tray.dictionary").to_string(),
            format!("⚙ {}", i18n.t("tray.settings")),
            format!("✕ {}", i18n.t("tray.quit")),
        )
    };

    let menu = Menu::new(app)?;

    // Pause / Resume (mutually exclusive)
    if is_paused {
        let resume = MenuItem::with_id(app, "resume", &lbl_resume, true, None::<&str>)?;
        menu.append(&resume)?;
    } else {
        let pause = MenuItem::with_id(app, "pause", &lbl_pause, true, None::<&str>)?;
        menu.append(&pause)?;
    }

    let next = MenuItem::with_id(app, "next", &lbl_next, true, None::<&str>)?;
    menu.append(&next)?;

    menu.append(&PredefinedMenuItem::separator(app)?)?;

    // Dictionary submenu
    let dict_label = if active_dict.is_empty() {
        format!("📚 {}", lbl_dict)
    } else {
        format!("📚 {} — {}", lbl_dict, active_dict.replace('_', " ").to_uppercase())
    };
    let dict_submenu = Submenu::new(app, &dict_label, true)?;

    let all_dicts = crate::commands::get_all_dict_ids(app);
    for id in &all_dicts {
        let label = if *id == active_dict {
            format!("✓ {}", id.replace('_', " ").to_uppercase())
        } else {
            format!("  {}", id.replace('_', " ").to_uppercase())
        };
        let item = MenuItem::with_id(app, format!("dict:{}", id), label, true, None::<&str>)?;
        dict_submenu.append(&item)?;
    }
    menu.append(&dict_submenu)?;

    menu.append(&PredefinedMenuItem::separator(app)?)?;

    let settings_item = MenuItem::with_id(app, "settings", &lbl_settings, true, None::<&str>)?;
    menu.append(&settings_item)?;

    menu.append(&PredefinedMenuItem::separator(app)?)?;

    let quit = MenuItem::with_id(app, "quit", &lbl_quit, true, None::<&str>)?;
    menu.append(&quit)?;

    Ok(menu)
}

// ── Tray creation ──────────────────────────────────────────────────────────────

pub fn create_tray(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let menu = build_menu(app)?;
    let icon = load_normal_icon(app).unwrap_or_else(fallback_icon);

    let tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("LazyWords")
        .icon(icon)
        .on_menu_event(|app, event| handle_menu_event(app, event.id.as_ref()))
        .on_tray_icon_event(|_tray, _event| {})
        .build(app)?;

    Ok(tray)
}

// ── Menu event handler ─────────────────────────────────────────────────────────

fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        "pause" => {
            {
                let state = app.state::<AppState>();
                let mut paused = state.is_paused.lock().unwrap();
                *paused = true;
            }
            refresh_tray(app);
        }
        "resume" => {
            {
                let state = app.state::<AppState>();
                let mut paused = state.is_paused.lock().unwrap();
                *paused = false;
            }
            refresh_tray(app);
        }
        "next" => {
            use tauri::Emitter;
            let state = app.state::<AppState>();
            // Implicit unpause
            *state.is_paused.lock().unwrap() = false;

            let entry = state.word_engine.lock().unwrap().get_random_word();
            if let Some(word) = entry {
                *state.current_word.lock().unwrap() = Some(word.clone());
                *state.is_card_visible.lock().unwrap() = true;
                if let Some(window) = app.get_webview_window("main") {
                    let pos = state.settings.lock().unwrap().position.clone();
                    crate::reposition_window(&window, &pos);
                }
                let _ = app.emit("show-word", serde_json::json!({
                    "term": word.entry.term,
                    "definition": word.entry.definition,
                    "index": word.index
                }));
            }
            refresh_tray(app);
        }
        "settings" => {
            if let Some(window) = app.get_webview_window("settings") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
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
        }
        "quit" => {
            // Persist state before exiting
            let app_state = app.state::<AppState>();
            {
                let learned = app_state.learned.lock().unwrap();
                crate::commands::save_learned_to_disk(app, &learned);
            }
            {
                let tracker = app_state.stats_tracker.lock().unwrap();
                crate::commands::save_stats_to_disk(app, &tracker.stats);
            }
            {
                let settings = app_state.settings.lock().unwrap();
                let path = app.path().app_data_dir().unwrap().join("data").join("settings.json");
                crate::settings::save_settings(&path, &settings);
            }
            app.exit(0);
        }
        other if other.starts_with("dict:") => {
            let new_id = other.strip_prefix("dict:").unwrap_or("").to_string();
            if !new_id.is_empty() {
                let state = app.state::<AppState>();
                let old_dict = state.settings.lock().unwrap().active_dictionary.clone();
                if new_id != old_dict {
                    if let Some(words) = crate::commands::load_dictionary_by_id(app, &new_id) {
                        {
                            let mut settings = state.settings.lock().unwrap();
                            settings.active_dictionary = new_id.clone();
                            let path = app.path().app_data_dir().unwrap().join("data").join("settings.json");
                            crate::settings::save_settings(&path, &settings);
                        }
                        {
                            let learned_indices = state.learned.lock().unwrap()
                                .get(&new_id).cloned().unwrap_or_default();
                            let mut engine = state.word_engine.lock().unwrap();
                            engine.load_dictionary(words);
                            engine.set_learned(learned_indices);
                        }
                        *state.all_learned.lock().unwrap() = false;
                        *state.all_dicts_done.lock().unwrap() = false;

                        use tauri::Emitter;
                        let settings_val = serde_json::to_value(&*state.settings.lock().unwrap()).unwrap_or_default();
                        let _ = app.emit("update-settings", &settings_val);
                        let _ = app.emit("dict-auto-switched", serde_json::json!({ "newId": new_id }));
                    }
                }
            }
            refresh_tray(app);
        }
        _ => {}
    }
}

// ── Refresh helper ─────────────────────────────────────────────────────────────

/// Rebuilds the tray menu and updates the icon to reflect current pause state.
/// Call this whenever `is_paused` or the active dictionary changes.
pub fn refresh_tray(app: &AppHandle) {
    let state = app.state::<AppState>();
    let tray_opt = state.tray.lock().unwrap();
    let Some(tray) = tray_opt.as_ref() else { return };

    let is_paused = *state.is_paused.lock().unwrap();

    // Swap icon
    let icon = if is_paused {
        load_paused_icon(app).or_else(|| load_normal_icon(app))
    } else {
        load_normal_icon(app)
    };
    if let Some(img) = icon {
        let _ = tray.set_icon(Some(img));
    }

    // Rebuild menu
    if let Ok(menu) = build_menu(app) {
        let _ = tray.set_menu(Some(menu));
    }
}

// ── Tiny 1×1 transparent fallback ─────────────────────────────────────────────

fn fallback_icon() -> Image<'static> {
    // 1×1 transparent RGBA pixel
    Image::new_owned(vec![0, 0, 0, 0], 1, 1)
}

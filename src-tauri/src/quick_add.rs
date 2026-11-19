use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{App, AppHandle, Emitter, Manager, State, WebviewWindow};

use crate::state::AppState;
use crate::word_engine::WordEntry;

pub const DECK_ID: &str = "quick-add:my-words";
const WINDOW: &str = "quick-add";
const OPENED: &str = "quick-add-opened";
const STORAGE_ERROR: &str = "quickAdd.saveError";
const DUPLICATE: &str = "quickAdd.duplicate";
const TERM_LIMIT: usize = 160;
const DEFINITION_LIMIT: usize = 4000;
const HEADERS: [&str; 2] = ["term", "definition"];

#[derive(Serialize)]
pub struct Draft {
    term: String,
    suggestion: Option<String>,
}

pub fn is_reserved(id: &str) -> bool {
    id.eq_ignore_ascii_case(DECK_ID)
}

fn deck_path(app: &AppHandle) -> tauri::Result<PathBuf> {
    Ok(app.path().app_data_dir()?.join("data/my-words.csv"))
}

pub fn has_deck(app: &AppHandle) -> bool {
    deck_path(app).is_ok_and(|path| path.is_file())
}

pub fn load_deck(app: &AppHandle) -> Option<Vec<WordEntry>> {
    let words = read_deck(&deck_path(app).ok()?).ok()?;
    (!words.is_empty()).then_some(words)
}

fn clean_term(text: &str) -> Option<&str> {
    let term = text.trim();
    (!term.is_empty() && term.chars().count() <= TERM_LIMIT && !term.chars().any(char::is_control))
        .then_some(term)
}

fn clean_card(term: &str, definition: &str) -> Result<WordEntry, &'static str> {
    let term = clean_term(term).ok_or("quickAdd.invalidTerm")?;
    let definition = definition.trim();
    if definition.is_empty()
        || definition.chars().count() > DEFINITION_LIMIT
        || definition
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
    {
        return Err("quickAdd.invalidDefinition");
    }
    Ok(WordEntry {
        term: term.into(),
        definition: definition.into(),
    })
}

fn read_deck(path: &Path) -> Result<Vec<WordEntry>, &'static str> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(vec![]),
        Err(_) => return Err(STORAGE_ERROR),
    };
    let mut reader = csv::Reader::from_reader(file);
    let headers = reader.headers().map_err(|_| STORAGE_ERROR)?;
    if !headers.iter().eq(HEADERS) {
        return Err(STORAGE_ERROR);
    }
    reader
        .deserialize::<WordEntry>()
        .map(|row| {
            let row = row.map_err(|_| STORAGE_ERROR)?;
            clean_card(&row.term, &row.definition).map_err(|_| STORAGE_ERROR)?;
            Ok(row)
        })
        .collect()
}

fn append_card(path: &Path, card: WordEntry) -> Result<Vec<WordEntry>, &'static str> {
    let mut words = read_deck(path)?;
    if words
        .iter()
        .any(|entry| entry.term == card.term && entry.definition == card.definition)
    {
        return Err(DUPLICATE);
    }
    words.push(card);

    let parent = path.parent().ok_or(STORAGE_ERROR)?;
    fs::create_dir_all(parent).map_err(|_| STORAGE_ERROR)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|_| STORAGE_ERROR)?;
    {
        let mut writer = csv::WriterBuilder::new()
            .has_headers(false)
            .from_writer(temp.as_file_mut());
        writer.write_record(HEADERS).map_err(|_| STORAGE_ERROR)?;
        // No old row moves, so existing learned indices still refer to the same cards.
        for word in &words {
            writer
                .write_record([&word.term, &word.definition])
                .map_err(|_| STORAGE_ERROR)?;
        }
        writer.flush().map_err(|_| STORAGE_ERROR)?;
    }
    temp.as_file().sync_all().map_err(|_| STORAGE_ERROR)?;
    temp.persist(path).map_err(|_| STORAGE_ERROR)?;
    Ok(words)
}

#[cfg(windows)]
fn clipboard_term() -> Option<String> {
    let text: String = clipboard_win::get_clipboard(clipboard_win::formats::Unicode).ok()?;
    clean_term(&text).map(str::to_owned)
}

#[cfg(not(windows))]
fn clipboard_term() -> Option<String> {
    None
}

fn find_suggestion(entries: Vec<WordEntry>, term: &str) -> Option<String> {
    let key = term.to_lowercase();
    entries
        .into_iter()
        .find(|word| word.term.to_lowercase() == key)
        .map(|word| word.definition)
}

fn bundled_suggestion(app: &AppHandle, active: &str, term: &str) -> Option<String> {
    if term.is_empty() || !matches!(active, "en-ru" | "en-definitions" | "en-irregular-verbs") {
        return None;
    }
    let path = crate::commands::get_bundled_dicts_dir(app).join(format!("{active}.csv"));
    find_suggestion(crate::dictionary_loader::load_dictionary(&path).ok()?, term)
}

pub fn create_window(app: &App) -> tauri::Result<()> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        WINDOW,
        tauri::WebviewUrl::App("windows/quick-add/quick-add.html".into()),
    )
    .title("LazyWords Quick Add")
    .inner_size(460.0, 450.0)
    .min_inner_size(360.0, 340.0)
    .visible(false)
    .build()?;
    let dialog = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = close_quick_add(dialog.clone(), dialog.state::<AppState>());
        }
    });
    Ok(())
}

pub fn open(app: &AppHandle) {
    let Some(window) = app.get_webview_window(WINDOW) else {
        return;
    };
    if !window.is_visible().unwrap_or(false) {
        let state = app.state::<AppState>();
        let active = state.settings.lock().unwrap().active_dictionary.clone();
        let term = clipboard_term().unwrap_or_default();
        let suggestion = bundled_suggestion(app, &active, &term);
        *state.quick_add_draft.lock().unwrap() = Some(Draft { term, suggestion });
        crate::center_on_cursor_monitor(&window);
        if window.show().is_err() {
            state.quick_add_draft.lock().unwrap().take();
            return;
        }
        let _ = app.emit_to(WINDOW, OPENED, ());
    }
    let _ = window.unminimize();
    let _ = window.set_focus();
}

#[tauri::command]
pub fn take_quick_add_draft(window: WebviewWindow, state: State<'_, AppState>) -> Option<Draft> {
    if window.label() != WINDOW {
        return None;
    }
    state.quick_add_draft.lock().unwrap().take()
}

#[tauri::command]
pub fn close_quick_add(window: WebviewWindow, state: State<'_, AppState>) -> Result<(), String> {
    if window.label() != WINDOW {
        return Err("Quick Add window required".into());
    }
    state.quick_add_draft.lock().unwrap().take();
    window.hide().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_quick_card(
    window: WebviewWindow,
    state: State<'_, AppState>,
    term: String,
    definition: String,
) -> Result<(), String> {
    if window.label() != WINDOW {
        return Err("Quick Add window required".into());
    }
    let card = clean_card(&term, &definition)?;
    let app = window.app_handle();
    let path = deck_path(app).map_err(|_| STORAGE_ERROR)?;
    let active = {
        // Serialize appends with dictionary changes so the engine never gets the wrong deck.
        let settings = state.settings.lock().unwrap();
        let words = append_card(&path, card)?;
        let active = settings.active_dictionary == DECK_ID;
        if active {
            let learned = state.learned.lock().unwrap();
            let mut engine = state.word_engine.lock().unwrap();
            engine.load_dictionary(words);
            engine.set_learned_ids(learned.ids(DECK_ID));
            *state.all_learned.lock().unwrap() = false;
        }
        *state.all_dicts_done.lock().unwrap() = false;
        active
    };
    let _ = app.emit("dict-list-updated", ());
    if active {
        let _ = app.emit("stats-updated", ());
    }
    crate::tray::refresh_tray(app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::word_engine::WordEngine;

    #[test]
    fn appending_keeps_indices_and_quotes_multiline_unicode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my-words.csv");
        append_card(&path, clean_card(" old ", " first ").unwrap()).unwrap();
        let definition = "значение, \"пример\"\nSecond line";
        let words = append_card(&path, clean_card("new", definition).unwrap()).unwrap();
        let reread = read_deck(&path).unwrap();
        assert_eq!(reread[0].term, "old");
        assert_eq!(reread[0].definition, "first");
        assert_eq!(reread[1].definition, definition);
        let mut engine = WordEngine::new();
        engine.set_learned(vec![0]);
        engine.load_dictionary(words);
        assert_eq!(engine.get_random_word().unwrap().index, 1);
    }

    #[test]
    fn duplicates_and_bad_existing_rows_do_not_change_the_deck() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my-words.csv");
        let card = || clean_card("same", "entry").unwrap();
        append_card(&path, card()).unwrap();
        let original = fs::read(&path).unwrap();
        assert_eq!(append_card(&path, card()).unwrap_err(), DUPLICATE);
        assert_eq!(fs::read(&path).unwrap(), original);
        let corrupt = b"term,definition\nmissing-definition\n";
        fs::write(&path, corrupt).unwrap();
        assert_eq!(append_card(&path, card()).unwrap_err(), STORAGE_ERROR);
        assert_eq!(fs::read(&path).unwrap(), corrupt);
    }

    #[test]
    fn lookup_only_suggests_an_exact_term_and_validates_inputs() {
        for text in ["", " ", "first\nsecond", "control\0text"] {
            assert!(clean_term(text).is_none());
        }
        assert!(clean_term(&"a".repeat(TERM_LIMIT + 1)).is_none());
        assert!(clean_card("word", " ").is_err());
        assert!(clean_card("word", "bad\0definition").is_err());
        assert!(clean_card("word", &"a".repeat(DEFINITION_LIMIT + 1)).is_err());
        let entries = || vec![clean_card("Word", "meaning").unwrap()];
        assert_eq!(
            find_suggestion(entries(), "WORD").as_deref(),
            Some("meaning")
        );
        assert!(find_suggestion(entries(), "wor").is_none());
    }
}

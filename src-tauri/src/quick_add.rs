use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Serialize;
use tauri::{App, AppHandle, Emitter, Manager, State, WebviewWindow};

use crate::state::AppState;
use crate::word_engine::WordEntry;
mod suggestions;
use suggestions::{Suggestion, SuggestionIndex};

pub const DECK_ID: &str = "quick-add:my-words";
const WINDOW: &str = "quick-add";
const OPENED: &str = "quick-add-opened";
const STORAGE_ERROR: &str = "quickAdd.saveError";
const DUPLICATE: &str = "quickAdd.duplicate";
const STALE_CARD: &str = "myWords.stale";
const AMBIGUOUS_CARD: &str = "myWords.ambiguous";
const TERM_LIMIT: usize = 160;
const DEFINITION_LIMIT: usize = 4000;
const HEADERS: [&str; 2] = ["term", "definition"];

#[derive(Serialize)]
pub struct Draft {
    term: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SaveOutcome {
    Saved,
    SavedWithoutSwitch,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyWord {
    card_id: String,
    #[serde(flatten)]
    entry: WordEntry,
}

pub fn is_reserved(id: &str) -> bool {
    id.eq_ignore_ascii_case(DECK_ID)
}

fn deck_path(app: &AppHandle) -> tauri::Result<PathBuf> {
    Ok(app.path().app_data_dir()?.join("data/my-words.csv"))
}

pub fn has_deck(app: &AppHandle) -> bool {
    load_deck(app).is_some()
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
    if words.iter().any(|entry| same_card(entry, &card)) {
        return Err(DUPLICATE);
    }
    words.push(card);
    write_deck(path, &words)?;
    Ok(words)
}

fn same_card(a: &WordEntry, b: &WordEntry) -> bool {
    a.term == b.term && a.definition == b.definition
}

fn change_card(
    path: &Path,
    card_id: &str,
    replacement: Option<WordEntry>,
) -> Result<Option<Vec<WordEntry>>, &'static str> {
    let mut words = read_deck(path)?;
    let mut matches = words.iter().enumerate()
        .filter(|(_, word)| crate::scheduler::card_id(word) == card_id);
    let (index, original) = matches.next().ok_or(STALE_CARD)?;
    if matches.next().is_some() {
        return Err(AMBIGUOUS_CARD);
    }

    if let Some(card) = replacement {
        if same_card(original, &card) {
            return Ok(None);
        }
        if words.iter().any(|entry| same_card(entry, &card)) {
            return Err(DUPLICATE);
        }
        words[index] = card;
    } else {
        words.remove(index);
    }

    // Learned markers and scheduling history stay keyed to their original content.
    write_deck(path, &words)?;
    Ok(Some(words))
}

fn write_deck(path: &Path, words: &[WordEntry]) -> Result<(), &'static str> {
    let parent = path.parent().ok_or(STORAGE_ERROR)?;
    fs::create_dir_all(parent).map_err(|_| STORAGE_ERROR)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|_| STORAGE_ERROR)?;
    {
        let mut writer = csv::WriterBuilder::new()
            .has_headers(false)
            .from_writer(temp.as_file_mut());
        writer.write_record(HEADERS).map_err(|_| STORAGE_ERROR)?;
        for word in words {
            writer
                .write_record([&word.term, &word.definition])
                .map_err(|_| STORAGE_ERROR)?;
        }
        writer.flush().map_err(|_| STORAGE_ERROR)?;
    }
    temp.as_file().sync_all().map_err(|_| STORAGE_ERROR)?;
    temp.persist(path).map_err(|_| STORAGE_ERROR)?;
    Ok(())
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

#[tauri::command]
pub fn quick_add_suggestions(
    window: WebviewWindow,
    state: State<'_, AppState>,
    term: String,
) -> Vec<Suggestion> {
    static INDEX: OnceLock<SuggestionIndex> = OnceLock::new();
    if window.label() != WINDOW {
        return vec![];
    }
    let Some(term) = clean_term(&term) else {
        return vec![];
    };
    let index = INDEX.get_or_init(|| {
        SuggestionIndex::load(&crate::commands::get_bundled_dicts_dir(window.app_handle()))
    });
    let i18n = state.i18n.lock().unwrap();
    index.lookup(&i18n.current_locale, term)
}

pub fn create_window(app: &App) -> tauri::Result<()> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        WINDOW,
        tauri::WebviewUrl::App("windows/quick-add/quick-add.html".into()),
    )
    .title("LazyWords Quick Add")
    .inner_size(460.0, 530.0)
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
        let term = clipboard_term().unwrap_or_default();
        *state.quick_add_draft.lock().unwrap() = Some(Draft { term });
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
    practice: Option<bool>,
) -> Result<SaveOutcome, String> {
    if window.label() != WINDOW {
        return Err("Quick Add window required".into());
    }
    let card = clean_card(&term, &definition)?;
    apply_deck_change(window.app_handle(), &state, practice.unwrap_or(false), |path| {
        append_card(path, card).map(Some)
    })
}

#[tauri::command]
pub fn get_my_words(window: WebviewWindow) -> Result<Vec<MyWord>, String> {
    let app = settings_app(&window)?;
    let path = deck_path(app).map_err(|_| STORAGE_ERROR)?;
    Ok(read_deck(&path)?.into_iter().map(|entry| MyWord {
        card_id: crate::scheduler::card_id(&entry),
        entry,
    }).collect())
}

#[tauri::command]
pub fn edit_my_word(
    window: WebviewWindow,
    state: State<'_, AppState>,
    card_id: String,
    term: String,
    definition: String,
) -> Result<(), String> {
    let app = settings_app(&window)?;
    let card = clean_card(&term, &definition)?;
    apply_deck_change(app, &state, false, |path| change_card(path, &card_id, Some(card))).map(|_| ())
}

#[tauri::command]
pub fn delete_my_word(
    window: WebviewWindow,
    state: State<'_, AppState>,
    card_id: String,
) -> Result<(), String> {
    let app = settings_app(&window)?;
    apply_deck_change(app, &state, false, |path| change_card(path, &card_id, None)).map(|_| ())
}

fn settings_app(window: &WebviewWindow) -> Result<&AppHandle, String> {
    if window.label() != "settings" {
        return Err("Settings window required".into());
    }
    Ok(window.app_handle())
}

fn apply_deck_change(
    app: &AppHandle,
    state: &AppState,
    practice: bool,
    change: impl FnOnce(&Path) -> Result<Option<Vec<WordEntry>>, &'static str>,
) -> Result<SaveOutcome, String> {
    let path = deck_path(app).map_err(|_| STORAGE_ERROR)?;
    let (active, switched, outcome) = {
        // Serialize deck writes and active-dictionary changes.
        let mut settings = state.settings.lock().unwrap();
        let Some(mut words) = change(&path)? else { return Ok(SaveOutcome::Saved) };
        let previous = settings.active_dictionary.clone();
        let outcome = if practice {
            select_saved_deck(&path.with_file_name("settings.json"), &mut settings)
        } else {
            SaveOutcome::Saved
        };
        let active = settings.active_dictionary == DECK_ID;
        if active && words.is_empty() {
            let fallback = crate::commands::get_all_dict_ids(app).into_iter().find_map(|id| {
                let words = crate::commands::load_dictionary_by_id(app, &id)?;
                (!words.is_empty()).then_some((id, words))
            });
            let (id, fallback_words) = fallback.unwrap_or_default();
            settings.active_dictionary = id;
            words = fallback_words;
            // The CSV already committed. Startup also falls back if settings could not be written.
            crate::settings::save_settings(&path.with_file_name("settings.json"), &settings);
        }
        if active {
            let learned = state.learned.lock().unwrap();
            let mut engine = state.word_engine.lock().unwrap();
            engine.load_dictionary(words);
            engine.set_learned_ids(learned.ids(&settings.active_dictionary));
            *state.all_learned.lock().unwrap() = false;
        }
        *state.all_dicts_done.lock().unwrap() = false;
        (active, (previous != settings.active_dictionary).then(|| settings.clone()), outcome)
    };
    if let Some(settings) = switched {
        let _ = app.emit("update-settings", &settings);
        let _ = app.emit("dict-auto-switched", serde_json::json!({ "newId": settings.active_dictionary }));
        if practice {
            if let Some(onboarding) = app.get_webview_window("onboarding") {
                let _ = onboarding.destroy();
            }
        }
    }
    let _ = app.emit("dict-list-updated", ());
    if active {
        let _ = app.emit("stats-updated", ());
    }
    crate::tray::refresh_tray(app);
    Ok(outcome)
}

fn select_saved_deck(path: &Path, settings: &mut crate::settings::Settings) -> SaveOutcome {
    if settings.active_dictionary == DECK_ID { return SaveOutcome::Saved; }
    let mut next = settings.clone();
    next.active_dictionary = DECK_ID.into();
    next.first_launch = false;
    // The CSV already committed. A settings failure must not invite a duplicate save.
    if let Err(error) = crate::settings::try_save_settings(path, &next) {
        eprintln!("Card saved but could not select My Words: {error}");
        return SaveOutcome::SavedWithoutSwitch;
    }
    *settings = next;
    SaveOutcome::Saved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::card_id;
    use crate::word_engine::WordEngine;

    #[test]
    fn practice_selection_only_changes_memory_after_settings_are_saved() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.json");
        let deck = dir.path().join("my-words.csv");
        let mut settings = crate::settings::Settings {
            active_dictionary: "original".into(), language: "ru".into(),
            ..Default::default()
        };
        append_card(&deck, clean_card("one", "a number").unwrap()).unwrap();
        fs::create_dir(&settings_path).unwrap();
        assert_eq!(select_saved_deck(&settings_path, &mut settings), SaveOutcome::SavedWithoutSwitch);
        assert_eq!(settings.active_dictionary, "original");
        assert!(settings.first_launch);
        fs::remove_dir(&settings_path).unwrap();
        assert_eq!(select_saved_deck(&settings_path, &mut settings), SaveOutcome::Saved);
        let restored = crate::settings::load_settings(&settings_path).unwrap();
        assert_eq!(restored.active_dictionary, DECK_ID);
        assert_eq!(restored.language, "ru");
        assert!(!restored.first_launch);
        assert_eq!(read_deck(&deck).unwrap().len(), 1);
    }

    #[test]
    fn appending_preserves_card_ids_and_quotes_multiline_unicode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my-words.csv");
        let old = clean_card(" old ", " first ").unwrap();
        let learned = [card_id(&old)];
        append_card(&path, old).unwrap();
        let definition = "значение, \"пример\"\nSecond line";
        let words = append_card(&path, clean_card("new", definition).unwrap()).unwrap();
        let reread = read_deck(&path).unwrap();
        assert_eq!(reread[0].term, "old");
        assert_eq!(reread[0].definition, "first");
        assert_eq!(reread[1].definition, definition);
        let mut engine = WordEngine::new();
        engine.load_dictionary(words);
        engine.set_learned_ids(&learned);
        assert_eq!(engine.get_random_word().unwrap().index, 1);
    }

    #[test]
    fn edits_and_deletes_preserve_other_cards_and_progress_after_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my-words.csv");
        let first = clean_card("first", "delete this").unwrap();
        let second = clean_card("second", "existing learned card").unwrap();
        let third = clean_card("third", "also learned").unwrap();
        let [first_id, second_id, third_id] = [&first, &second, &third].map(card_id);
        for card in [first, second.clone(), third] {
            append_card(&path, card).unwrap();
        }
        let mut learned = crate::learned::Learned::from_legacy(
            [(DECK_ID.to_owned(), vec![1, 42])].into(),
        );
        assert!(learned.mark(DECK_ID, &second_id));
        assert!(learned.mark(DECK_ID, &third_id));
        let learned_path = dir.path().join(crate::learned::FILE_NAME);
        crate::profile_data::save_json(&learned_path, &learned).unwrap();
        let learned_before = fs::read(&learned_path).unwrap();
        let other_files = ["learned.json", "progress.json", "stats.json"].map(|name| {
            let path = dir.path().join(name);
            fs::write(&path, b"{}").unwrap();
            path
        });

        change_card(&path, &first_id, None).unwrap();
        let updated = clean_card("second", "новое, \"значение\"\nMore text").unwrap();
        let updated_id = card_id(&updated);
        change_card(&path, &second_id, Some(updated.clone())).unwrap();
        let words = read_deck(&path).unwrap();
        assert_eq!(words[0].definition, updated.definition);
        assert_eq!(words.iter().map(card_id).collect::<Vec<_>>(), [updated_id.clone(), third_id]);
        let mut engine = WordEngine::new();
        engine.load_dictionary(words);
        engine.set_learned_ids(learned.ids(DECK_ID));
        assert_eq!(engine.get_random_word().unwrap().card_id, updated_id);
        assert_eq!(learned.pending(DECK_ID), &[1, 42]);
        assert_eq!(fs::read(learned_path).unwrap(), learned_before);
        for path in other_files {
            assert_eq!(fs::read(path).unwrap(), b"{}");
        }
        append_card(&path, second).unwrap();
        engine.load_dictionary(read_deck(&path).unwrap());
        engine.set_learned_ids(learned.ids(DECK_ID));
        assert_eq!(engine.learned_indices.len(), 2);
    }

    #[test]
    fn stale_duplicate_and_ambiguous_changes_do_not_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my-words.csv");
        let a = clean_card("same", "first meaning").unwrap();
        let b = clean_card("same", "another meaning").unwrap();
        let id = card_id(&a);
        write_deck(&path, &[a.clone(), b.clone()]).unwrap();
        let original = fs::read(&path).unwrap();
        assert!(change_card(&path, &id, Some(a.clone())).unwrap().is_none());
        assert_eq!(change_card(&path, &id, Some(b.clone())).unwrap_err(), DUPLICATE);
        assert_eq!(change_card(&path, "old-id", None).unwrap_err(), STALE_CARD);
        assert_eq!(fs::read(&path).unwrap(), original);

        write_deck(&path, &[a.clone(), a, b]).unwrap();
        let original = fs::read(&path).unwrap();
        assert_eq!(change_card(&path, &id, None).unwrap_err(), AMBIGUOUS_CARD);
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn deleting_last_card_writes_a_reusable_empty_csv() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my-words.csv");
        let card = clean_card("last", "card").unwrap();
        append_card(&path, card.clone()).unwrap();
        assert!(change_card(&path, &card_id(&card), None).unwrap().unwrap().is_empty());
        assert!(read_deck(&path).unwrap().is_empty());
        assert_eq!(append_card(&path, card).unwrap().len(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn failed_atomic_replacement_keeps_the_old_deck() {
        use std::os::windows::fs::OpenOptionsExt;
        use winapi::um::winnt::{FILE_SHARE_READ, FILE_SHARE_WRITE};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("my-words.csv");
        let card = clean_card("last", "card").unwrap();
        let id = card_id(&card);
        append_card(&path, card).unwrap();
        let original = fs::read(&path).unwrap();
        let _lock = fs::OpenOptions::new().read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE).open(&path).unwrap();
        assert_eq!(change_card(&path, &id, None).unwrap_err(), STORAGE_ERROR);
        assert_eq!(fs::read(&path).unwrap(), original);
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
    fn validates_quick_add_inputs() {
        for text in ["", " ", "first\nsecond", "control\0text"] {
            assert!(clean_term(text).is_none());
        }
        assert!(clean_term(&"a".repeat(TERM_LIMIT + 1)).is_none());
        assert!(clean_card("word", " ").is_err());
        assert!(clean_card("word", "bad\0definition").is_err());
        assert!(clean_card("word", &"a".repeat(DEFINITION_LIMIT + 1)).is_err());
    }
}

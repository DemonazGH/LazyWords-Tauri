use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tauri::{App, AppHandle, Emitter, Manager, State, WebviewWindow};

use crate::learned::Learned;
use crate::scheduler::SchedulingHistory;
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
const RETAINED_PROGRESS: &str = "myWords.retainedProgress";
const DECK_FILE: &str = "my-words.csv";
const EDIT_JOURNAL: &str = "my-words-edit.json";
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
    Ok(app.path().app_data_dir()?.join("data").join(DECK_FILE))
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
    change_card_with(path, card_id, replacement, || Ok(()))
}

/// `before_write` runs once the change is known to be valid, just before the CSV is replaced.
fn change_card_with(
    path: &Path,
    card_id: &str,
    replacement: Option<WordEntry>,
    before_write: impl FnOnce() -> Result<(), &'static str>,
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

    // A deleted card's learned marker and scheduling history stay keyed to its content.
    before_write()?;
    write_deck(path, &words)?;
    Ok(Some(words))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingEdit {
    old: String,
    new: String,
}

fn journal_path(deck: &Path) -> PathBuf {
    deck.with_file_name(EDIT_JOURNAL)
}

/// Settles an edit recorded in the journal, so the CSV, the learned marker and the scheduling
/// history agree on the card's ID. Safe to repeat: it runs after every edit and at startup.
///
/// The CSV decides the outcome. If it holds the new card, the marker and history follow it; if
/// it still holds the old one, the edit never happened and only the journal is removed.
fn settle_edit(
    deck: &Path,
    learned: &mut Learned,
    history: &mut SchedulingHistory,
    save: &dyn Fn(&Learned) -> bool,
) {
    let journal = journal_path(deck);
    let Ok(bytes) = fs::read(&journal) else {
        return;
    };
    let Ok(words) = read_deck(deck) else {
        return;
    };
    let ids: Vec<String> = words.iter().map(crate::scheduler::card_id).collect();
    let mut settled = true;
    if let Ok(edit) = serde_json::from_slice::<PendingEdit>(&bytes) {
        if ids.contains(&edit.new) && !ids.contains(&edit.old) {
            // The CSV is already committed, so memory follows it even if the save fails. The new
            // ID had no marker before the edit, so finding one means an earlier attempt moved it
            // in memory and the file may still be behind.
            learned.replace(DECK_ID, &edit.old, &edit.new);
            if learned.ids(DECK_ID).contains(&edit.new) {
                settled &= save(learned);
            }
            // Likewise for the history: flush whenever it names the new card, because an
            // earlier attempt may have moved it in memory without reaching the file.
            history.rename_card(DECK_ID, &edit.old, &edit.new);
            if history.mentions(DECK_ID, &edit.new) {
                settled &= history.flush();
            }
        }
    }
    if settled {
        let _ = fs::remove_file(journal);
    }
}

pub fn recover_edit(dir: &Path, learned: &mut Learned, history: &mut SchedulingHistory) {
    let path = dir.join(crate::learned::FILE_NAME);
    settle_edit(&dir.join(DECK_FILE), learned, history, &|next| {
        crate::profile_data::save_json(&path, next).is_ok()
    });
}

/// Edits a card and moves its learned marker and scheduling history to the new content.
///
/// The edit is journaled before the CSV changes. A failure or a crash at any later point is
/// finished by `settle_edit`, now or at the next start.
fn edit_card(
    path: &Path,
    learned: &mut Learned,
    history: &mut SchedulingHistory,
    card_id: &str,
    card: WordEntry,
    save: &dyn Fn(&Learned) -> bool,
) -> Result<Option<Vec<WordEntry>>, &'static str> {
    let journal = journal_path(path);
    settle_edit(path, learned, history, save);
    if journal.exists() {
        return Err(STORAGE_ERROR);
    }
    let new_id = crate::scheduler::card_id(&card);
    let words = change_card_with(path, card_id, Some(card), || {
        // Progress saved for a removed card with this text is not merged by guesswork.
        if learned.ids(DECK_ID).contains(&new_id) || history.mentions(DECK_ID, &new_id) {
            return Err(RETAINED_PROGRESS);
        }
        let edit = PendingEdit { old: card_id.to_owned(), new: new_id.clone() };
        crate::profile_data::save_json(&journal, &edit).map_err(|_| STORAGE_ERROR)
    });
    match words {
        Ok(None) => {}
        Ok(Some(_)) => settle_edit(path, learned, history, save),
        Err(_) => {
            let _ = fs::remove_file(&journal);
        }
    }
    words
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
    apply_deck_change(app, &state, false, |path| {
        let mut learned = state.learned.lock().unwrap();
        let mut history = state.scheduling_history.lock().unwrap();
        edit_card(path, &mut learned, &mut history, &card_id, card, &|next| {
            crate::commands::save_learned(app, next)
        })
    })
    .map(|_| ())
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

    struct Profile {
        dir: tempfile::TempDir,
        learned: Learned,
        history: SchedulingHistory,
        typo_id: String,
        other_id: String,
        fixed: WordEntry,
    }

    impl Profile {
        /// A deck with a learned, twice-shown "teh" card and an unlearned "other" card.
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let typo = clean_card("teh", "the definite article").unwrap();
            let other = clean_card("other", "still in rotation").unwrap();
            let [typo_id, other_id] = [&typo, &other].map(card_id);
            for card in [typo, other] {
                append_card(&dir.path().join(DECK_FILE), card).unwrap();
            }
            let mut learned = Learned::from_legacy([(DECK_ID.to_owned(), vec![7])].into());
            learned.mark(DECK_ID, &typo_id);
            let mut profile = Self {
                learned,
                history: SchedulingHistory::load(dir.path().join("progress.json")),
                typo_id,
                other_id,
                fixed: clean_card("the", "the definite article").unwrap(),
                dir,
            };
            assert!(profile.save(&profile.learned));
            profile.history.record_for_test(DECK_ID, &profile.typo_id, 2);
            profile
        }

        fn file(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }

        fn save(&self, learned: &Learned) -> bool {
            crate::profile_data::save_json(&self.file(crate::learned::FILE_NAME), learned).is_ok()
        }

        fn edit(&mut self, id: &str, card: WordEntry, save_works: bool) -> Result<bool, &'static str> {
            let path = self.file(crate::learned::FILE_NAME);
            let deck = self.file(DECK_FILE);
            edit_card(&deck, &mut self.learned, &mut self.history, id, card, &|next| {
                save_works && crate::profile_data::save_json(&path, next).is_ok()
            })
            .map(|words| words.is_some())
        }

        /// Reloads everything from disk the way a restart does, including edit recovery.
        fn restart(&mut self) {
            let bytes = fs::read(self.file(crate::learned::FILE_NAME)).unwrap();
            self.learned = serde_json::from_slice(&bytes).unwrap();
            self.history = SchedulingHistory::load(self.file("progress.json"));
            recover_edit(self.dir.path(), &mut self.learned, &mut self.history);
        }

        fn bytes(&self) -> Vec<Option<Vec<u8>>> {
            [DECK_FILE, crate::learned::FILE_NAME, "progress.json", EDIT_JOURNAL, "stats.json"]
                .map(|name| fs::read(self.file(name)).ok())
                .to_vec()
        }

        /// The deck, the marker and the history all name the corrected card, in memory and on disk.
        fn assert_moved(&mut self) {
            let fixed_id = card_id(&self.fixed);
            for _ in 0..2 {
                let deck: Vec<_> = read_deck(&self.file(DECK_FILE)).unwrap().iter().map(card_id).collect();
                assert_eq!(deck, [fixed_id.clone(), self.other_id.clone()]);
                assert_eq!(self.learned.ids(DECK_ID), [fixed_id.clone()]);
                assert_eq!(self.learned.pending(DECK_ID), &[7]);
                assert_eq!(self.history.shown_for_test(DECK_ID, &fixed_id), Some(2));
                assert_eq!(self.history.shown_for_test(DECK_ID, &self.typo_id), None);
                assert!(!self.file(EDIT_JOURNAL).exists());
                assert!(!self.file("stats.json").exists(), "an edit is not a learning event");
                self.restart();
            }
        }
    }

    #[test]
    fn correcting_a_learned_card_keeps_marker_and_history_across_restart() {
        let mut profile = Profile::new();
        assert_eq!(profile.edit(&profile.typo_id.clone(), profile.fixed.clone(), true), Ok(true));
        profile.assert_moved();
        let mut engine = WordEngine::new();
        engine.load_dictionary(read_deck(&profile.file(DECK_FILE)).unwrap());
        engine.set_learned_ids(profile.learned.ids(DECK_ID));
        assert_eq!(engine.get_random_word().unwrap().card_id, profile.other_id);

        // Unchanged text writes nothing; an unlearned card stays unlearned.
        let before = profile.bytes();
        assert_eq!(profile.edit(&card_id(&profile.fixed), profile.fixed.clone(), false), Ok(false));
        assert_eq!(profile.bytes(), before);
        let renamed = clean_card("another", "still in rotation").unwrap();
        assert_eq!(profile.edit(&profile.other_id.clone(), renamed, false), Ok(true));
        assert_eq!(profile.learned.ids(DECK_ID), [card_id(&profile.fixed)]);
        assert!(!profile.file(EDIT_JOURNAL).exists());
    }

    #[test]
    fn rejected_edits_change_no_file_and_no_memory() {
        let mut profile = Profile::new();
        let before = (profile.bytes(), profile.learned.clone());
        let other = clean_card("other", "still in rotation").unwrap();
        assert_eq!(profile.edit(&profile.typo_id.clone(), other, true), Err(DUPLICATE));
        assert_eq!(profile.edit("missing", profile.fixed.clone(), true), Err(STALE_CARD));
        assert_eq!((profile.bytes(), profile.learned.clone()), before);
    }

    #[test]
    fn text_with_saved_progress_is_refused_instead_of_merged() {
        // A removed card left a learned marker behind.
        let mut profile = Profile::new();
        profile.learned.mark(DECK_ID, &card_id(&profile.fixed));
        assert!(profile.save(&profile.learned));
        let before = (profile.bytes(), profile.learned.clone());
        for id in [profile.typo_id.clone(), profile.other_id.clone()] {
            assert_eq!(profile.edit(&id, profile.fixed.clone(), true), Err(RETAINED_PROGRESS));
        }
        assert_eq!((profile.bytes(), profile.learned.clone()), before);

        // A removed card left only scheduling history behind.
        let mut profile = Profile::new();
        profile.history.record_for_test(DECK_ID, &card_id(&profile.fixed), 5);
        let before = (profile.bytes(), profile.learned.clone());
        assert_eq!(
            profile.edit(&profile.typo_id.clone(), profile.fixed.clone(), true),
            Err(RETAINED_PROGRESS)
        );
        assert_eq!((profile.bytes(), profile.learned.clone()), before);
        assert_eq!(profile.history.shown_for_test(DECK_ID, &card_id(&profile.fixed)), Some(5));
        assert_eq!(profile.history.shown_for_test(DECK_ID, &profile.typo_id), Some(2));
    }

    #[test]
    fn marker_save_failure_after_the_deck_changed_is_finished_on_restart() {
        let mut profile = Profile::new();
        assert_eq!(profile.edit(&profile.typo_id.clone(), profile.fixed.clone(), false), Ok(true));
        // The running app already treats the corrected card as learned.
        assert_eq!(profile.learned.ids(DECK_ID), [card_id(&profile.fixed)]);
        assert!(profile.file(EDIT_JOURNAL).exists());
        // Another edit cannot stack on an unsettled one.
        let renamed = clean_card("another", "still in rotation").unwrap();
        assert_eq!(profile.edit(&profile.other_id.clone(), renamed, false), Err(STORAGE_ERROR));
        profile.restart();
        profile.assert_moved();
    }

    #[test]
    fn a_crash_before_the_deck_changed_is_undone_on_restart() {
        let mut profile = Profile::new();
        let before = profile.bytes();
        let edit = PendingEdit { old: profile.typo_id.clone(), new: card_id(&profile.fixed) };
        crate::profile_data::save_json(&profile.file(EDIT_JOURNAL), &edit).unwrap();
        profile.restart();
        assert_eq!(profile.bytes(), before);
        assert_eq!(profile.learned.ids(DECK_ID), [profile.typo_id.clone()]);

        // A damaged journal is discarded without touching anything else.
        fs::write(profile.file(EDIT_JOURNAL), b"not json").unwrap();
        profile.restart();
        assert_eq!(profile.bytes(), before);
    }

    #[cfg(windows)]
    #[test]
    fn a_failed_deck_write_leaves_marker_history_and_deck_on_the_old_card() {
        use std::os::windows::fs::OpenOptionsExt;
        use winapi::um::winnt::{FILE_SHARE_READ, FILE_SHARE_WRITE};

        let mut profile = Profile::new();
        let before = (profile.bytes(), profile.learned.clone());
        {
            let _lock = fs::OpenOptions::new().read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .open(profile.file(DECK_FILE)).unwrap();
            assert_eq!(
                profile.edit(&profile.typo_id.clone(), profile.fixed.clone(), true),
                Err(STORAGE_ERROR)
            );
        }
        assert_eq!((profile.bytes(), profile.learned.clone()), before);
        profile.restart();
        assert_eq!((profile.bytes(), profile.learned.clone()), before);

        // Adding the proposed correction later gives a card that is still up for practice.
        let words = append_card(&profile.file(DECK_FILE), profile.fixed.clone()).unwrap();
        let mut engine = WordEngine::new();
        engine.load_dictionary(words);
        engine.set_learned_ids(profile.learned.ids(DECK_ID));
        let unlearned: Vec<_> = engine.unlearned_cards().map(|(_, word)| word.term.clone()).collect();
        assert_eq!(unlearned, ["other", "the"]);
    }

    #[cfg(windows)]
    #[test]
    fn a_locked_history_file_delays_the_move_but_never_resets_the_history() {
        use std::os::windows::fs::OpenOptionsExt;
        use winapi::um::winnt::{FILE_SHARE_READ, FILE_SHARE_WRITE};

        let mut profile = Profile::new();
        {
            let _lock = fs::OpenOptions::new().read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .open(profile.file("progress.json")).unwrap();
            assert_eq!(profile.edit(&profile.typo_id.clone(), profile.fixed.clone(), true), Ok(true));
            // The running app already counts the history under the corrected card.
            assert_eq!(profile.history.shown_for_test(DECK_ID, &card_id(&profile.fixed)), Some(2));
            assert!(profile.file(EDIT_JOURNAL).exists());
            // A retry while the file is still locked must not drop the journal either.
            let renamed = clean_card("another", "still in rotation").unwrap();
            assert_eq!(profile.edit(&profile.other_id.clone(), renamed, true), Err(STORAGE_ERROR));
            assert!(profile.file(EDIT_JOURNAL).exists());
        }
        profile.restart();
        profile.assert_moved();
    }

    #[cfg(windows)]
    #[test]
    fn a_history_move_left_in_memory_is_written_by_the_next_edit() {
        use std::os::windows::fs::OpenOptionsExt;
        use winapi::um::winnt::{FILE_SHARE_READ, FILE_SHARE_WRITE};

        let mut profile = Profile::new();
        {
            let _lock = fs::OpenOptions::new().read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .open(profile.file("progress.json")).unwrap();
            assert_eq!(profile.edit(&profile.typo_id.clone(), profile.fixed.clone(), true), Ok(true));
        }
        // No restart: the next edit settles the earlier one first, then goes through.
        let renamed = clean_card("another", "still in rotation").unwrap();
        assert_eq!(profile.edit(&profile.other_id.clone(), renamed, true), Ok(true));
        assert!(!profile.file(EDIT_JOURNAL).exists());
        profile.restart();
        assert_eq!(profile.history.shown_for_test(DECK_ID, &card_id(&profile.fixed)), Some(2));
        assert_eq!(profile.history.shown_for_test(DECK_ID, &profile.typo_id), None);
        assert_eq!(profile.learned.ids(DECK_ID), [card_id(&profile.fixed)]);
    }

    #[test]
    fn a_deleted_cards_mark_or_history_is_not_inherited_by_an_edit() {
        // B is learned and then deleted; unlearned A may not take B's text.
        let mut profile = Profile::new();
        let b = clean_card("teh", "the definite article").unwrap();
        change_card(&profile.file(DECK_FILE), &profile.typo_id, None).unwrap();
        let before = (profile.bytes(), profile.learned.clone());
        assert_eq!(profile.edit(&profile.other_id.clone(), b.clone(), true), Err(RETAINED_PROGRESS));
        assert_eq!((profile.bytes(), profile.learned.clone()), before);
        let mut engine = WordEngine::new();
        engine.load_dictionary(read_deck(&profile.file(DECK_FILE)).unwrap());
        engine.set_learned_ids(profile.learned.ids(DECK_ID));
        assert_eq!(engine.get_random_word().unwrap().card_id, profile.other_id);

        // B was shown seven times and deleted, A once; both counts survive the refused edit.
        let mut profile = Profile::new();
        profile.learned = Learned::from_legacy([(DECK_ID.to_owned(), vec![7])].into());
        assert!(profile.save(&profile.learned));
        profile.history.record_for_test(DECK_ID, &profile.typo_id, 7);
        profile.history.record_for_test(DECK_ID, &profile.other_id, 1);
        change_card(&profile.file(DECK_FILE), &profile.typo_id, None).unwrap();
        let before = profile.bytes();
        assert_eq!(profile.edit(&profile.other_id.clone(), b, true), Err(RETAINED_PROGRESS));
        profile.restart();
        assert_eq!(profile.bytes(), before);
        assert_eq!(profile.history.shown_for_test(DECK_ID, &profile.typo_id), Some(7));
        assert_eq!(profile.history.shown_for_test(DECK_ID, &profile.other_id), Some(1));
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

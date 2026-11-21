use crate::{
    commands, learned::Learned, scheduler::card_id, settings::Settings, word_engine::WordEntry,
};
use std::{collections::HashSet, path::Path};
use tauri::AppHandle;

type Deck = (String, Vec<WordEntry>);

pub(crate) enum Search {
    Completion,
    ReplaceEmpty,
}

pub(crate) fn choose_deck(
    settings: &Settings,
    mut ids: Vec<String>,
    learned: &Learned,
    mut load: impl FnMut(&str) -> Option<Vec<WordEntry>>,
    search: Search,
) -> Option<Deck> {
    let origin = settings
        .my_words_return_to
        .as_deref()
        .filter(|_| settings.active_dictionary == super::DECK_ID);
    // Prioritize an enumerated deck, never use the saved ID as a path.
    if let Some(index) = ids.iter().position(|id| Some(id.as_str()) == origin) {
        ids[..=index].rotate_right(1);
    }

    let mut first_nonempty = None;
    for id in ids
        .into_iter()
        .filter(|id| id != &settings.active_dictionary)
    {
        let Some(words) = load(&id).filter(|w| !w.is_empty()) else {
            continue;
        };
        if origin.is_none() && matches!(search, Search::ReplaceEmpty) {
            return Some((id, words)); // Ordinary deletion keeps its existing fallback order.
        }
        let known: HashSet<_> = learned.ids(&id).iter().map(String::as_str).collect();
        if words.iter().any(|w| !known.contains(card_id(w).as_str())) {
            return Some((id, words));
        }
        if first_nonempty.is_none() && matches!(search, Search::ReplaceEmpty) {
            first_nonempty = Some((id, words));
        }
    }
    first_nonempty
}

pub(crate) fn replacement_for_empty(
    app: &AppHandle,
    settings: &Settings,
    learned: &Learned,
) -> Option<Deck> {
    choose_deck(
        settings,
        commands::get_all_dict_ids(app),
        learned,
        |id| commands::load_dictionary_by_id(app, id),
        Search::ReplaceEmpty,
    )
}

pub(crate) fn finish(
    path: &Path,
    settings: &mut Settings,
    next_id: Option<&str>,
) -> Result<(), String> {
    if next_id.is_none() && settings.my_words_return_to.is_none() {
        return Ok(());
    }
    let mut next = settings.clone();
    next.select_dictionary(next_id.unwrap_or(&settings.active_dictionary).to_owned());
    crate::settings::try_save_settings(path, &next)?;
    *settings = next;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quick_add::{select_saved_deck, SaveOutcome, DECK_ID};
    use crate::settings::{load_settings, try_save_settings};

    fn word(term: &str) -> WordEntry {
        WordEntry {
            term: term.into(),
            definition: "answer".into(),
        }
    }

    fn session(origin: Option<&str>) -> Settings {
        Settings {
            active_dictionary: DECK_ID.into(),
            my_words_return_to: origin.map(str::to_owned),
            ..Settings::default()
        }
    }

    fn candidates() -> Vec<String> {
        ["bundled", DECK_ID, "custom"].map(str::to_owned).into()
    }

    #[test]
    fn save_practice_restart_then_finish_returns_to_original_custom_deck_once() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        let mut settings = Settings {
            active_dictionary: "custom".into(),
            language: "ru".into(),
            ..Settings::default()
        };
        assert_eq!(select_saved_deck(&path, &mut settings), SaveOutcome::Saved);
        assert_eq!(select_saved_deck(&path, &mut settings), SaveOutcome::Saved);
        let mut restarted = load_settings(&path).unwrap();
        assert_eq!(restarted.my_words_return_to.as_deref(), Some("custom"));
        let mut learned = Learned::from_legacy(Default::default());
        learned.mark("custom", &card_id(&word("known")));
        let load = |id: &str| Some(vec![word("known"), word(id)]);
        let (id, words) =
            choose_deck(&restarted, candidates(), &learned, load, Search::Completion).unwrap();
        assert_eq!(id, "custom");
        let mut engine = crate::word_engine::WordEngine::new();
        engine.load_dictionary(words);
        engine.set_learned_ids(learned.ids(&id));
        assert_eq!(
            engine
                .unlearned_cards()
                .map(|(_, w)| w.term.as_str())
                .collect::<Vec<_>>(),
            ["custom"]
        );
        finish(&path, &mut restarted, Some(&id)).unwrap();
        let restored = load_settings(&path).unwrap();
        assert_eq!(restored.active_dictionary, "custom");
        assert_eq!(restored.language, "ru");
        assert!(restored.my_words_return_to.is_none());
        assert!(!restored.needs_onboarding());
    }

    #[test]
    fn unavailable_or_exhausted_origin_falls_back_to_first_playable_deck() {
        let mut learned = Learned::from_legacy(Default::default());
        learned.mark("done", &card_id(&word("done")));
        for origin in ["missing", "broken", "empty", "done", DECK_ID] {
            let settings = session(Some(origin));
            let ids = ["done", "first", "second", "empty", "broken", DECK_ID]
                .map(str::to_owned)
                .into();
            let (id, _) = choose_deck(
                &settings,
                ids,
                &learned,
                |id| match id {
                    "broken" => None,
                    "empty" => Some(vec![]),
                    DECK_ID | "missing" => panic!("must not load current or unlisted deck"),
                    other => Some(vec![word(other)]),
                },
                Search::Completion,
            )
            .unwrap();
            assert_eq!(id, "first", "origin {origin}");
        }
    }

    #[test]
    fn no_playable_deck_clears_return_without_switching_and_write_failure_can_retry() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        let mut settings = session(Some("custom"));
        let mut learned = Learned::from_legacy(Default::default());
        learned.mark("custom", &card_id(&word("known")));
        let next = choose_deck(
            &settings,
            vec!["custom".into()],
            &learned,
            |_| Some(vec![word("known")]),
            Search::Completion,
        );
        assert!(next.is_none());
        for next_id in [Some("custom"), None] {
            assert!(finish(tmp.path(), &mut settings, next_id).is_err());
            assert_eq!(settings.active_dictionary, DECK_ID);
            assert_eq!(settings.my_words_return_to.as_deref(), Some("custom"));
        }
        finish(&path, &mut settings, None).unwrap();
        let restored = load_settings(&path).unwrap();
        assert_eq!(restored.active_dictionary, DECK_ID);
        assert!(restored.my_words_return_to.is_none());
    }

    #[test]
    fn manual_selection_and_first_launch_do_not_invent_a_return_target() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        let learned = Learned::from_legacy(Default::default());
        for mut settings in [Settings::default(), session(Some("custom"))] {
            if settings.active_dictionary == DECK_ID {
                settings.select_dictionary("manual".into());
                settings.select_dictionary(DECK_ID.into());
                try_save_settings(&path, &settings).unwrap();
            }
            assert_eq!(select_saved_deck(&path, &mut settings), SaveOutcome::Saved);
            let restored = load_settings(&path).unwrap();
            assert!(restored.my_words_return_to.is_none());
            assert_eq!(
                choose_deck(
                    &restored,
                    candidates(),
                    &learned,
                    |id| Some(vec![word(id)]),
                    Search::Completion
                )
                .unwrap()
                .0,
                "bundled"
            );
        }
    }

    #[test]
    fn deleting_last_my_words_card_returns_to_origin_only_for_explicit_practice() {
        let learned = Learned::from_legacy(Default::default());
        for (origin, expected) in [(Some("custom"), "custom"), (None, "bundled")] {
            let settings = session(origin);
            assert_eq!(
                choose_deck(
                    &settings,
                    candidates(),
                    &learned,
                    |id| Some(vec![word(id)]),
                    Search::ReplaceEmpty
                )
                .unwrap()
                .0,
                expected
            );
        }
    }
}

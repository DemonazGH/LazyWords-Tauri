use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::{seq::SliceRandom, Rng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::state::AppState;
use crate::word_engine::{WordEngine, WordEntry, WordWithIndex};

const VERSION: u32 = 1;
const ONE_DAY: u64 = 86_400;

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Exposure {
    shown: u32,
    last_shown: u64,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DeckHistory {
    last_card: Option<String>,
    cards: HashMap<String, Exposure>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Progress {
    version: u32,
    decks: HashMap<String, DeckHistory>,
}

pub struct SchedulingHistory {
    path: PathBuf,
    progress: Progress,
    writable: bool,
}

pub fn card_id(word: &WordEntry) -> String {
    let mut hash = Sha256::new();
    // Frame the term before hashing so pairs such as ("ab", "c") and ("a", "bc") stay distinct.
    hash.update((word.term.len() as u64).to_be_bytes());
    hash.update(word.term.as_bytes());
    hash.update(word.definition.as_bytes());
    format!("{:x}", hash.finalize())
}

fn weight(exposure: Option<&Exposure>, now: u64) -> f64 {
    let Some(exposure) = exposure else {
        return 16.0;
    };
    let age = now.saturating_sub(exposure.last_shown).min(ONE_DAY) as f64 / ONE_DAY as f64;
    let practice = 8.0 / (1.0 + f64::from(exposure.shown.min(7)));
    practice * (0.25 + 0.75 * age)
}

impl SchedulingHistory {
    pub fn load(path: PathBuf) -> Self {
        let mut history = Self {
            path,
            progress: Progress {
                version: VERSION,
                decks: HashMap::new(),
            },
            writable: true,
        };
        match fs::read(&history.path) {
            Ok(bytes) => match serde_json::from_slice::<Progress>(&bytes) {
                Ok(progress) if progress.version == VERSION => history.progress = progress,
                _ => history.writable = false,
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => history.writable = false,
        }
        if !history.writable {
            eprintln!("Could not load scheduling history; leaving progress.json untouched");
        }
        history
    }

    fn next<R: Rng + ?Sized>(
        &mut self,
        deck_id: &str,
        engine: &WordEngine,
        now: u64,
        rng: &mut R,
    ) -> Option<WordWithIndex> {
        let deck = self.progress.decks.entry(deck_id.to_owned()).or_default();
        let mut candidates: Vec<_> = engine
            .unlearned_cards()
            .map(|(index, word)| {
                let id = engine.card_id(index);
                (index, word, id, weight(deck.cards.get(id), now))
            })
            .collect();
        if let Some(last) = &deck.last_card {
            if candidates.iter().any(|candidate| candidate.2 != last) {
                candidates.retain(|candidate| candidate.2 != last);
            }
        }
        let (index, word, id, _) = *candidates
            .choose_weighted(rng, |candidate| candidate.3)
            .ok()?;
        let exposure = deck.cards.entry(id.clone()).or_default();
        exposure.shown = exposure.shown.saturating_add(1);
        exposure.last_shown = now;
        deck.last_card = Some(id.clone());
        Some(WordWithIndex {
            entry: word.clone(),
            index,
            card_id: id.clone(),
        })
    }

    fn persist(&self) -> io::Result<()> {
        if !self.writable {
            return Ok(());
        }
        let dir = self
            .path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        fs::create_dir_all(dir)?;
        let bytes = serde_json::to_vec(&self.progress)?;
        let mut temp = tempfile::NamedTempFile::new_in(dir)?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(&self.path).map_err(|error| error.error)?;
        Ok(())
    }
}

pub fn select_next(state: &AppState) -> (String, Option<WordWithIndex>) {
    let settings = state.settings.lock().unwrap();
    let deck_id = settings.active_dictionary.clone();
    let engine = state.word_engine.lock().unwrap();
    if !settings.smart_scheduling || deck_id.is_empty() {
        return (deck_id, engine.get_random_word());
    }

    let mut history = state.scheduling_history.lock().unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let entry = history.next(&deck_id, &engine, now, &mut rand::thread_rng());
    drop(engine);
    drop(settings);
    if entry.is_some() {
        if let Err(error) = history.persist() {
            eprintln!("Could not save scheduling history: {error}");
        }
    }
    (deck_id, entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::StdRng, SeedableRng};

    fn engine(cards: &[(&str, &str)]) -> WordEngine {
        let mut engine = WordEngine::new();
        engine.load_dictionary(
            cards
                .iter()
                .map(|(term, definition)| WordEntry {
                    term: (*term).into(),
                    definition: (*definition).into(),
                })
                .collect(),
        );
        engine
    }

    #[test]
    fn weights_favor_new_less_practiced_and_older_cards() {
        let now = ONE_DAY;
        let recent = Exposure {
            shown: 1,
            last_shown: now,
        };
        let older = Exposure {
            shown: 1,
            last_shown: 0,
        };
        let practiced = Exposure {
            shown: 8,
            last_shown: 0,
        };
        assert!(weight(None, now) > weight(Some(&older), now));
        assert!(weight(Some(&older), now) > weight(Some(&recent), now));
        assert!(weight(Some(&older), now) > weight(Some(&practiced), now));
        assert_eq!(weight(Some(&recent), 0), weight(Some(&recent), now));
        assert!(weight(Some(&practiced), now) > 0.0);
    }

    #[test]
    fn actual_weighted_draws_favor_unseen_cards_with_a_seeded_rng() {
        let dir = tempfile::tempdir().unwrap();
        let mut history = SchedulingHistory::load(dir.path().join("progress.json"));
        let engine = engine(&[("new", "new"), ("old-a", "old"), ("old-b", "old")]);
        let mut rng = StdRng::seed_from_u64(76);
        let mut unseen_draws = 0;
        for _ in 0..1000 {
            let deck = history.progress.decks.entry("deck".into()).or_default();
            deck.last_card = None;
            deck.cards.clear();
            for index in [1, 2] {
                deck.cards.insert(
                    engine.card_id(index).clone(),
                    Exposure {
                        shown: 100,
                        last_shown: 1000,
                    },
                );
            }
            if history.next("deck", &engine, 1000, &mut rng).unwrap().index == 0 {
                unseen_draws += 1;
            }
        }
        assert!(
            unseen_draws > 900,
            "unseen card selected {unseen_draws}/1000 times"
        );
    }

    #[cfg(windows)]
    #[test]
    fn failed_history_replacement_keeps_previous_bytes_and_can_retry() {
        use std::os::windows::fs::OpenOptionsExt;
        use winapi::um::winnt::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let mut history = SchedulingHistory::load(path.clone());
        let engine = engine(&[("one", "1")]);
        let mut rng = StdRng::seed_from_u64(4);
        history.next("deck", &engine, 1, &mut rng).unwrap();
        history.persist().unwrap();
        let original = fs::read(&path).unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&path)
            .unwrap();
        history.next("deck", &engine, 2, &mut rng).unwrap();
        assert!(history.persist().is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        drop(lock);
        history.persist().unwrap();
        assert_eq!(
            SchedulingHistory::load(path).progress.decks["deck"].cards[engine.card_id(0)].shown,
            2
        );
    }

    #[test]
    fn excludes_learned_and_avoids_repeats_unless_only_one_card_remains() {
        let dir = tempfile::tempdir().unwrap();
        let mut history = SchedulingHistory::load(dir.path().join("progress.json"));
        let mut engine = engine(&[("known", "0"), ("one", "1"), ("two", "2")]);
        let mut rng = StdRng::seed_from_u64(12);
        engine.set_learned(vec![0]);
        let first = history.next("deck", &engine, 10, &mut rng).unwrap();
        let second = history.next("deck", &engine, 10, &mut rng).unwrap();
        assert_ne!(first.index, 0);
        assert_ne!(second.index, 0);
        assert_ne!(first.index, second.index);
        engine.set_learned(vec![0, first.index]);
        assert_eq!(
            history.next("deck", &engine, 11, &mut rng).unwrap().index,
            second.index
        );
        engine.set_learned(vec![0, 1, 2]);
        assert!(history.next("deck", &engine, 12, &mut rng).is_none());
    }

    #[test]
    fn saves_counts_by_deck_and_card_content_across_reorder_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        let mut history = SchedulingHistory::load(path.clone());
        let original = engine(&[("term", "first meaning")]);
        let mut rng = StdRng::seed_from_u64(42);
        history.next("deck", &original, 300, &mut rng).unwrap();
        history.next("deck", &original, 301, &mut rng).unwrap();
        history.persist().unwrap();

        let reordered = engine(&[("term", "second meaning"), ("term", "first meaning")]);
        let mut restored = SchedulingHistory::load(path);
        let deck = &restored.progress.decks["deck"];
        let exposure = &deck.cards[reordered.card_id(1)];
        assert_eq!((exposure.shown, exposure.last_shown), (2, 301));
        assert!(!deck.cards.contains_key(reordered.card_id(0)));
        assert_eq!(
            restored
                .next("deck", &reordered, 302, &mut rng)
                .unwrap()
                .index,
            0
        );
        restored.next("other", &original, 302, &mut rng).unwrap();
        assert_eq!(
            restored.progress.decks["other"].cards[original.card_id(0)].shown,
            1
        );
        assert_eq!(
            restored.progress.decks["deck"].cards[original.card_id(0)].shown,
            2
        );
    }

    #[test]
    fn unreadable_or_newer_history_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("progress.json");
        for data in [
            b"{invalid".as_slice(),
            br#"{"version":2,"decks":{}}"#.as_slice(),
        ] {
            fs::write(&path, data).unwrap();
            let mut history = SchedulingHistory::load(path.clone());
            let mut rng = StdRng::seed_from_u64(1);
            assert!(history
                .next("deck", &engine(&[("one", "1")]), 1, &mut rng)
                .is_some());
            history.persist().unwrap();
            assert_eq!(fs::read(&path).unwrap(), data);
        }
    }
}

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

const EN: &str = "en-definitions";
const RU: &str = "en-ru";
const VERBS: &str = "en-irregular-verbs";

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    source: &'static str,
    answer: String,
}

#[derive(Default)]
pub(super) struct SuggestionIndex(HashMap<&'static str, HashMap<String, String>>);

impl SuggestionIndex {
    pub(super) fn load(dir: &Path) -> Self {
        let mut index = Self::default();
        for source in [EN, RU, VERBS] {
            let path = dir.join(format!("{source}.csv"));
            if let Ok(words) = crate::dictionary_loader::load_dictionary(&path) {
                let entries = index.0.entry(source).or_default();
                for word in words {
                    entries
                        .entry(word.term.to_lowercase())
                        .or_insert(word.definition);
                }
            }
        }
        index
    }

    pub(super) fn lookup(&self, active: &str, term: &str) -> Vec<Suggestion> {
        let sources: &[&'static str] = match active {
            VERBS => &[VERBS, EN, RU],
            RU => &[RU, EN],
            _ => &[EN, RU],
        };
        let key = term.trim().to_lowercase();
        sources
            .iter()
            .filter_map(|&source| {
                let answer = self.0.get(source)?.get(&key)?.clone();
                Some(Suggestion { source, answer })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn custom_decks_get_exact_matches_from_both_shipped_dictionaries() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/dictionaries");
        let index = SuggestionIndex::load(&dir);
        for active in [
            "my imported deck",
            crate::quick_add::DECK_ID,
            "",
            "../some-file",
        ] {
            let found = index.lookup(active, " ABOUT ");
            assert_eq!(found.len(), 2);
            assert_eq!(found[0].source, EN);
            assert_eq!(found[0].answer, "concerning or relating to something");
            assert_eq!(found[1].source, RU);
            assert!(found[1].answer.contains("около"));
            assert!(index.lookup(active, "abou").is_empty());
        }
        assert_eq!(index.lookup(RU, "about")[0].source, RU);
    }

    #[test]
    fn missing_sources_and_duplicate_terms_do_not_hide_other_matches() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("en-ru.csv"),
            "term,definition\ngo,first\nGo,second\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("en-irregular-verbs.csv"),
            "term,definition\ngo,went / gone\n",
        )
        .unwrap();
        let index = SuggestionIndex::load(dir.path());
        assert_eq!(
            index.lookup("custom", "go"),
            vec![Suggestion {
                source: RU,
                answer: "first".into()
            }]
        );
        assert_eq!(
            index.lookup(VERBS, "GO"),
            vec![
                Suggestion {
                    source: VERBS,
                    answer: "went / gone".into()
                },
                Suggestion {
                    source: RU,
                    answer: "first".into()
                },
            ]
        );
        assert!(index.lookup("custom", "unknown").is_empty());
    }
}

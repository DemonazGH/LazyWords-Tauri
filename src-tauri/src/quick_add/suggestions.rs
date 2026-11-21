use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

const EN: &str = "en-definitions";
const RU: &str = "en-ru";

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
        for source in [EN, RU] {
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

    pub(super) fn lookup(&self, locale: &str, term: &str) -> Vec<Suggestion> {
        let source = if locale == "ru" { RU } else { EN };
        let key = term.trim().to_lowercase();
        self.0
            .get(source)
            .and_then(|words| words.get(&key))
            .map(|answer| Suggestion {
                source,
                answer: answer.clone(),
            })
            .into_iter()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn resolved_ui_locale_selects_one_exact_match_independent_of_the_deck() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/dictionaries");
        let index = SuggestionIndex::load(&dir);
        for locale in ["en", "other"] {
            let found = index.lookup(locale, " ABOUT ");
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].source, EN);
            assert_eq!(found[0].answer, "concerning or relating to something");
            assert!(index.lookup(locale, "abou").is_empty());
        }
        let found = index.lookup("ru", "About");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, RU);
        assert!(found[0].answer.contains("около"));
    }

    #[test]
    fn missing_source_does_not_substitute_a_translation_or_verb_form() {
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
            index.lookup("ru", "go"),
            vec![Suggestion {
                source: RU,
                answer: "first".into()
            }]
        );
        assert!(index.lookup("en", "go").is_empty());
        assert!(index.lookup("ru", "unknown").is_empty());
    }
}

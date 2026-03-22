use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use rand::seq::SliceRandom;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordEntry {
    pub word: String,
    pub translation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordWithIndex {
    #[serde(flatten)]
    pub entry: WordEntry,
    pub index: usize,
}

pub struct WordEngine {
    pub dictionary: Vec<WordEntry>,
    pub learned_indices: HashSet<usize>,
}

impl WordEngine {
    pub fn new() -> Self {
        Self {
            dictionary: Vec::new(),
            learned_indices: HashSet::new(),
        }
    }

    pub fn load_dictionary(&mut self, words: Vec<WordEntry>) {
        self.dictionary = words;
    }

    pub fn set_learned(&mut self, indices: Vec<usize>) {
        self.learned_indices = indices.into_iter().collect();
    }

    pub fn get_random_word(&self) -> Option<WordWithIndex> {
        let active: Vec<_> = self.dictionary
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.learned_indices.contains(i))
            .collect();

        if active.is_empty() {
            return None;
        }

        let mut rng = rand::thread_rng();
        let (index, entry) = active.choose(&mut rng).unwrap();

        Some(WordWithIndex {
            entry: (*entry).clone(),
            index: *index,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_engine(words: &[(&str, &str)]) -> WordEngine {
        let mut engine = WordEngine::new();
        engine.load_dictionary(words.iter().map(|&(w, t)| WordEntry {
            word: w.to_string(),
            translation: t.to_string(),
        }).collect());
        engine
    }

    #[test]
    fn random_word_returns_from_dictionary() {
        let engine = make_engine(&[("hello", "привет"), ("world", "мир")]);
        let result = engine.get_random_word().unwrap();
        assert!(result.entry.word == "hello" || result.entry.word == "world");
    }

    #[test]
    fn never_returns_learned_word() {
        let mut engine = make_engine(&[("hello", "привет"), ("world", "мир")]);
        engine.set_learned(vec![0]);
        for _ in 0..50 {
            let word = engine.get_random_word().unwrap();
            assert_ne!(word.entry.word, "hello");
            assert_eq!(word.index, 1);
        }
    }

    #[test]
    fn returns_none_when_all_learned() {
        let mut engine = make_engine(&[("hello", "привет"), ("world", "мир")]);
        engine.set_learned(vec![0, 1]);
        assert!(engine.get_random_word().is_none());
    }

    #[test]
    fn returns_none_for_empty_dictionary() {
        let engine = WordEngine::new();
        assert!(engine.get_random_word().is_none());
    }

    #[test]
    fn words_available_again_after_reset_learned() {
        let mut engine = make_engine(&[("hello", "привет")]);
        engine.set_learned(vec![0]);
        assert!(engine.get_random_word().is_none());
        engine.set_learned(vec![]);
        assert!(engine.get_random_word().is_some());
    }
}

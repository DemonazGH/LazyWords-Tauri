use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use rand::seq::SliceRandom;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordEntry {
    #[serde(alias = "word", alias = "front")]
    pub term: String,
    #[serde(alias = "translation", alias = "back")]
    pub definition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordWithIndex {
    #[serde(flatten)]
    pub entry: WordEntry,
    pub index: usize,
    pub card_id: String,
}

pub struct WordEngine {
    pub dictionary: Vec<WordEntry>,
    pub learned_indices: HashSet<usize>,
    card_ids: Vec<String>,
}

impl WordEngine {
    pub fn new() -> Self {
        Self {
            dictionary: Vec::new(),
            learned_indices: HashSet::new(),
            card_ids: Vec::new(),
        }
    }

    pub fn load_dictionary(&mut self, words: Vec<WordEntry>) {
        self.card_ids = words.iter().map(crate::scheduler::card_id).collect();
        self.dictionary = words;
    }

    #[cfg(test)]
    pub fn set_learned(&mut self, indices: Vec<usize>) {
        self.learned_indices = indices.into_iter().collect();
    }

    pub fn set_learned_ids(&mut self, ids: &[String]) {
        let ids: HashSet<_> = ids.iter().collect();
        self.learned_indices = self.card_ids.iter().enumerate()
            .filter_map(|(index, id)| ids.contains(id).then_some(index)).collect();
    }

    pub fn card_ids(&self) -> &[String] {
        &self.card_ids
    }

    pub fn unlearned_cards(&self) -> impl Iterator<Item = (usize, &WordEntry)> {
        self.dictionary
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.learned_indices.contains(i))
    }

    pub fn card_id(&self, index: usize) -> &String {
        &self.card_ids[index]
    }

    pub fn get_random_word(&self) -> Option<WordWithIndex> {
        let active: Vec<_> = self.unlearned_cards().collect();
        let mut rng = rand::thread_rng();
        let (index, entry) = active.choose(&mut rng)?;

        Some(WordWithIndex {
            entry: (*entry).clone(),
            index: *index,
            card_id: self.card_ids[*index].clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_engine(words: &[(&str, &str)]) -> WordEngine {
        let mut engine = WordEngine::new();
        engine.load_dictionary(words.iter().map(|&(t, d)| WordEntry {
            term: t.to_string(),
            definition: d.to_string(),
        }).collect());
        engine
    }

    #[test]
    fn random_word_returns_from_dictionary() {
        let engine = make_engine(&[("hello", "привет"), ("world", "мир")]);
        let result = engine.get_random_word().unwrap();
        assert!(result.entry.term == "hello" || result.entry.term == "world");
    }

    #[test]
    fn never_returns_learned_word() {
        let mut engine = make_engine(&[("hello", "привет"), ("world", "мир")]);
        engine.set_learned(vec![0]);
        for _ in 0..50 {
            let word = engine.get_random_word().unwrap();
            assert_ne!(word.entry.term, "hello");
            assert_eq!(word.index, 1);
        }
    }

    #[test]
    fn stable_learned_id_tracks_a_card_after_reorder_and_deleting_another_row() {
        let mut engine = make_engine(&[("run", "operate"), ("run", "move quickly"), ("other", "other")]);
        let id = engine.card_id(0).clone();
        let distinct = engine.card_id(1).clone();
        assert_ne!(id, distinct);
        let reversed = [engine.dictionary[1].clone(), engine.dictionary[0].clone()];
        engine.load_dictionary(reversed.to_vec());
        engine.set_learned_ids(&[id]);
        assert_eq!(engine.learned_indices, HashSet::from([1]));
        assert_eq!(engine.get_random_word().unwrap().card_id, distinct);
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

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type LegacyMarkers = HashMap<String, Vec<usize>>;

pub const FILE_NAME: &str = "learned-v2.json";
const VERSION: u32 = 2;

pub fn unique_card_ids(ids: &[String]) -> HashSet<&str> {
    let mut occurrences = HashMap::new();
    for id in ids {
        *occurrences.entry(id.as_str()).or_insert(0usize) += 1;
    }
    occurrences.into_iter().filter_map(|(id, n)| (n == 1).then_some(id)).collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Learned {
    version: u32,
    decks: HashMap<String, Vec<String>>,
    pending: LegacyMarkers,
}

impl Learned {
    pub fn from_legacy(pending: LegacyMarkers) -> Self {
        Self {
            version: VERSION,
            decks: HashMap::new(),
            pending,
        }
    }

    pub fn is_supported(&self) -> bool {
        self.version == VERSION
            && self.decks.values().flatten().all(|id| {
                id.len() == 64
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
    }

    pub fn ids(&self, deck: &str) -> &[String] {
        self.decks.get(deck).map(Vec::as_slice).unwrap_or_default()
    }

    pub fn pending(&self, deck: &str) -> &[usize] {
        self.pending
            .get(deck)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.values().map(Vec::len).sum()
    }

    pub fn pending_decks(&self) -> std::collections::BTreeMap<&str, usize> {
        self.pending.iter().filter(|(_, rows)| !rows.is_empty())
            .map(|(id, rows)| (id.as_str(), rows.len())).collect()
    }

    pub fn mark(&mut self, deck: &str, id: &str) -> bool {
        let ids = self.decks.entry(deck.to_owned()).or_default();
        if ids.iter().any(|existing| existing == id) {
            return false;
        }
        ids.push(id.to_owned());
        true
    }

    pub fn restore(&mut self, deck: &str, visible_ids: &HashSet<&str>) -> usize {
        let Some(ids) = self.decks.get_mut(deck) else {
            return 0;
        };
        let before = ids.len();
        ids.retain(|id| !visible_ids.contains(id.as_str()));
        before - ids.len()
    }

    pub fn order_token(card_ids: &[String]) -> String {
        let mut digest = Sha256::new();
        // Each card ID is fixed-width hex, so the ordered sequence is unambiguous.
        for id in card_ids {
            digest.update(id.as_bytes());
        }
        format!("{:x}", digest.finalize())
    }

    pub fn resolve(&mut self, deck: &str, card_ids: &[String], reviewed_order: &str) -> usize {
        if Self::order_token(card_ids) != reviewed_order {
            return 0;
        }
        let unique = unique_card_ids(card_ids);
        let Some(rows) = self.pending.get_mut(deck) else {
            return 0;
        };
        let stable = self.decks.entry(deck.to_owned()).or_default();
        let mut resolved = 0;
        rows.retain(|&row| {
            let Some(id) = card_ids.get(row) else {
                return true;
            };
            if !unique.contains(id.as_str()) {
                return true;
            }
            if !stable.contains(id) {
                stable.push(id.clone());
            }
            resolved += 1;
            false
        });
        resolved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_keeps_missing_decks_out_of_range_rows_and_duplicate_content_pending() {
        let mut learned = Learned::from_legacy(HashMap::from([
            ("deck".into(), vec![0, 1, 99]),
            ("missing".into(), vec![2]),
        ]));
        let cards = ["unique", "duplicate", "duplicate"].map(String::from);
        assert!(learned.ids("deck").is_empty());
        assert_eq!(
            learned.resolve("deck", &cards, &Learned::order_token(&cards)),
            1
        );
        assert_eq!(learned.ids("deck"), ["unique"]);
        assert_eq!(learned.pending("deck"), [1, 99]);
        assert_eq!(learned.pending("missing"), [2]);
    }

    #[test]
    fn stale_order_and_restore_fail_to_discard_pending_or_absent_markers() {
        let mut learned = Learned::from_legacy(HashMap::from([("deck".into(), vec![0])]));
        let old = ["one", "two"].map(String::from);
        let changed = ["two", "one"].map(String::from);
        assert_eq!(
            learned.resolve("deck", &changed, &Learned::order_token(&old)),
            0
        );
        learned.mark("deck", "absent");
        learned.mark("deck", "two");
        assert_eq!(learned.restore("deck", &HashSet::from(["two"])), 1);
        assert_eq!(learned.ids("deck"), ["absent"]);
        assert_eq!(learned.pending("deck"), [0]);
    }
}

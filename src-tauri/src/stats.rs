use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use chrono::Local;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DailyStat {
    pub shown: u32,
    pub learned: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stats {
    #[serde(flatten)]
    pub days: HashMap<String, DailyStat>,
}

pub struct StatsTracker {
    pub stats: Stats,
}

impl StatsTracker {
    pub fn new(stats: Stats) -> Self {
        Self { stats }
    }

    pub fn today() -> String {
        Local::now().format("%Y-%m-%d").to_string()
    }

    pub fn record_shown(&mut self) {
        let key = Self::today();
        let stat = self.stats.days.entry(key).or_insert(DailyStat::default());
        stat.shown += 1;
    }

    pub fn record_learned(&mut self) {
        let key = Self::today();
        let stat = self.stats.days.entry(key).or_insert(DailyStat::default());
        stat.learned += 1;
    }

    pub fn decrement_learned(&mut self) {
        let key = Self::today();
        if let Some(stat) = self.stats.days.get_mut(&key) {
            if stat.learned > 0 {
                stat.learned -= 1;
            }
        }
    }

    pub fn get_streak(&self) -> u32 {
        let mut streak = 0;
        let mut d = Local::now().naive_local().date();
        loop {
            let key = d.format("%Y-%m-%d").to_string();
            if let Some(stat) = self.stats.days.get(&key) {
                if stat.shown == 0 {
                    break;
                }
                streak += 1;
                d = d - chrono::Duration::days(1);
            } else {
                break;
            }
        }
        streak
    }

    pub fn get_last_7_days(&self) -> Vec<serde_json::Value> {
        let mut result = Vec::new();
        let d = Local::now().naive_local().date();
        for i in (0..=6).rev() {
            let day = d - chrono::Duration::days(i);
            let key = day.format("%Y-%m-%d").to_string();
            let stat = self.stats.days.get(&key).cloned().unwrap_or_default();
            result.push(serde_json::json!({
                "date": key,
                "shown": stat.shown,
                "learned": stat.learned
            }));
        }
        result
    }
}

pub fn load_stats(path: &Path) -> Stats {
    if path.exists() {
        if let Ok(data) = fs::read_to_string(path) {
            if let Ok(stats) = serde_json::from_str::<Stats>(&data) {
                return stats;
            }
        }
    }
    Stats::default()
}

pub fn save_stats(path: &Path, stats: &Stats) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(data) = serde_json::to_string_pretty(stats) {
        let _ = fs::write(path, data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    fn today() -> String {
        Local::now().naive_local().date().format("%Y-%m-%d").to_string()
    }

    fn date_offset(days_ago: i64) -> String {
        (Local::now().naive_local().date() - chrono::Duration::days(days_ago))
            .format("%Y-%m-%d")
            .to_string()
    }

    #[test]
    fn record_shown_increments_today_shown() {
        let mut t = StatsTracker::new(Stats::default());
        t.record_shown();
        t.record_shown();
        assert_eq!(t.stats.days[&today()].shown, 2);
    }

    #[test]
    fn record_learned_increments_today_learned() {
        let mut t = StatsTracker::new(Stats::default());
        t.record_learned();
        assert_eq!(t.stats.days[&today()].learned, 1);
    }

    #[test]
    fn decrement_learned_does_not_go_below_zero() {
        let mut t = StatsTracker::new(Stats::default());
        // No entry — should be a no-op
        t.decrement_learned();
        // Entry already at zero — should stay at zero
        t.stats.days.insert(today(), DailyStat { shown: 1, learned: 0 });
        t.decrement_learned();
        assert_eq!(t.stats.days[&today()].learned, 0);
    }

    #[test]
    fn streak_is_one_for_single_day() {
        let mut stats = Stats::default();
        stats.days.insert(today(), DailyStat { shown: 3, learned: 1 });
        let t = StatsTracker::new(stats);
        assert_eq!(t.get_streak(), 1);
    }

    #[test]
    fn streak_counts_consecutive_days() {
        let mut stats = Stats::default();
        for i in 0..3 {
            stats.days.insert(date_offset(i), DailyStat { shown: 1, learned: 0 });
        }
        let t = StatsTracker::new(stats);
        assert_eq!(t.get_streak(), 3);
    }

    #[test]
    fn get_last_7_days_returns_exactly_7_entries() {
        let t = StatsTracker::new(Stats::default());
        assert_eq!(t.get_last_7_days().len(), 7);
    }
}

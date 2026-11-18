use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Serialize};

use crate::stats::Stats;

pub type Learned = HashMap<String, Vec<usize>>;

#[derive(Debug)]
pub struct LoadError {
    pub path: PathBuf,
    source: io::Error,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.source)
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug)]
pub struct ProfileData {
    pub learned: Learned,
    pub stats: Stats,
}

impl ProfileData {
    pub fn load(dir: &Path) -> Result<Self, LoadError> {
        // Load both before constructing AppState or enabling any profile writers.
        let learned_path = dir.join("learned.json");
        let learned = load_json(&learned_path).map_err(|source| LoadError {
            path: learned_path,
            source,
        })?;
        let stats_path = dir.join("stats.json");
        let stats = crate::stats::load_stats(&stats_path).map_err(|source| LoadError {
            path: stats_path,
            source,
        })?;
        Ok(Self { learned, stats })
    }
}

pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> io::Result<T> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error),
    }
}

pub fn save_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let parent = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{save_stats, DailyStat};

    fn sample_learned() -> Learned {
        HashMap::from([("my-deck".into(), vec![4, 0, 7]), ("other".into(), vec![2])])
    }

    fn sample_stats() -> Stats {
        Stats {
            days: HashMap::from([
                (
                    "2026-11-16".into(),
                    DailyStat {
                        shown: 12,
                        learned: 3,
                    },
                ),
                (
                    "2026-11-17".into(),
                    DailyStat {
                        shown: 0,
                        learned: 0,
                    },
                ),
            ]),
        }
    }

    fn bytes(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                (
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    fs::read(path).unwrap(),
                )
            })
            .collect();
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    }

    #[test]
    fn missing_files_default_without_creating_a_profile_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing/data");
        let profile = ProfileData::load(&path).unwrap();
        assert!(profile.learned.is_empty());
        assert!(profile.stats.days.is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn valid_legacy_profile_keeps_all_decks_order_and_historical_statistics() {
        let dir = tempfile::tempdir().unwrap();
        let learned_path = dir.path().join("learned.json");
        let stats_path = dir.path().join("stats.json");
        save_json(&learned_path, &sample_learned()).unwrap();
        save_stats(&stats_path, &sample_stats()).unwrap();
        let before = bytes(dir.path());
        let restored = ProfileData::load(dir.path()).unwrap();
        assert_eq!(restored.learned, sample_learned());
        assert_eq!(
            serde_json::to_value(restored.stats).unwrap(),
            serde_json::to_value(sample_stats()).unwrap()
        );
        assert_eq!(bytes(dir.path()), before);
    }

    #[test]
    fn malformed_or_wrong_shape_data_aborts_loading_and_changes_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let settings = br#"{"activeDictionary":"my-deck","firstLaunch":false}"#;
        fs::write(dir.path().join("settings.json"), settings).unwrap();
        for file in ["learned.json", "stats.json"] {
            for invalid in [
                b"{".as_slice(),
                b"null",
                b"[]",
                b"not json",
                br#"{"deck":"invalid"}"#,
            ] {
                save_json(&dir.path().join("learned.json"), &sample_learned()).unwrap();
                save_stats(&dir.path().join("stats.json"), &sample_stats()).unwrap();
                fs::write(dir.path().join(file), invalid).unwrap();
                let before = bytes(dir.path());
                let error = ProfileData::load(dir.path()).unwrap_err();
                assert_eq!(error.path, dir.path().join(file));
                assert_eq!(error.source.kind(), io::ErrorKind::InvalidData);
                assert_eq!(bytes(dir.path()), before);
            }
        }
    }

    #[test]
    fn invalid_existing_file_does_not_create_a_missing_sibling() {
        for file in ["learned.json", "stats.json"] {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join(file), b"{").unwrap();
            assert!(ProfileData::load(dir.path()).is_err());
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn unreadable_file_is_an_error_not_an_empty_profile() {
        for file in ["learned.json", "stats.json"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(file);
            fs::create_dir(&path).unwrap();
            assert_eq!(ProfileData::load(dir.path()).unwrap_err().path, path);
            assert!(save_json(&path, &sample_learned()).is_err());
            assert!(path.is_dir());
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn atomic_saves_create_and_replace_both_formats_without_leftover_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data");
        let learned_path = root.join("learned.json");
        let stats_path = root.join("stats.json");
        save_json(&learned_path, &Learned::new()).unwrap();
        save_stats(&stats_path, &Stats::default()).unwrap();
        save_json(&learned_path, &sample_learned()).unwrap();
        save_stats(&stats_path, &sample_stats()).unwrap();
        let restored = ProfileData::load(&root).unwrap();
        assert_eq!(restored.learned, sample_learned());
        assert_eq!(restored.stats.days["2026-11-16"].shown, 12);
        assert_eq!(restored.stats.days["2026-11-16"].learned, 3);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
    }

    #[test]
    fn serialization_failure_does_not_touch_the_destination() {
        struct FailingValue;
        impl Serialize for FailingValue {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("simulated serialization failure"))
            }
        }
        let dir = tempfile::tempdir().unwrap();
        for file in ["learned.json", "stats.json"] {
            let path = dir.path().join(file);
            fs::write(&path, b"original bytes").unwrap();
            let before = bytes(dir.path());
            assert!(save_json(&path, &FailingValue).is_err());
            assert_eq!(bytes(dir.path()), before);
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_read_locks_abort_startup_without_touching_either_file() {
        use std::os::windows::fs::OpenOptionsExt;
        for file in ["learned.json", "stats.json"] {
            let dir = tempfile::tempdir().unwrap();
            save_json(&dir.path().join("learned.json"), &sample_learned()).unwrap();
            save_stats(&dir.path().join("stats.json"), &sample_stats()).unwrap();
            let before = bytes(dir.path());
            let lock = fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(dir.path().join(file))
                .unwrap();
            assert_eq!(
                ProfileData::load(dir.path()).unwrap_err().path,
                dir.path().join(file)
            );
            drop(lock);
            assert_eq!(bytes(dir.path()), before);
        }
    }

    #[cfg(windows)]
    #[test]
    fn blocked_replacement_preserves_original_bytes_for_learned_and_stats() {
        use std::os::windows::fs::OpenOptionsExt;
        use winapi::um::winnt::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        let dir = tempfile::tempdir().unwrap();
        let learned = dir.path().join("learned.json");
        let stats = dir.path().join("stats.json");
        save_json(&learned, &sample_learned()).unwrap();
        save_stats(&stats, &sample_stats()).unwrap();
        for path in [&learned, &stats] {
            let before = bytes(dir.path());
            let lock = fs::OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .open(path)
                .unwrap();
            let result = if path == &learned {
                save_json(path, &Learned::new())
            } else {
                save_stats(path, &Stats::default())
            };
            assert!(result.is_err());
            assert_eq!(bytes(dir.path()), before);
            drop(lock);
        }
        save_json(&learned, &Learned::new()).unwrap();
        save_stats(&stats, &Stats::default()).unwrap();
        let restored = ProfileData::load(dir.path()).unwrap();
        assert!(restored.learned.is_empty());
        assert!(restored.stats.days.is_empty());
    }
}

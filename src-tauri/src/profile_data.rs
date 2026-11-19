use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Serialize};

use crate::learned::{Learned, FILE_NAME};
use crate::stats::Stats;

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
    needs_migration: bool,
}

impl ProfileData {
    pub fn load(dir: &Path) -> Result<Self, LoadError> {
        // Load both before constructing AppState or enabling any profile writers.
        let stable_path = dir.join(FILE_NAME);
        let (learned, needs_migration) = match read_json::<Learned>(&stable_path) {
            Ok(learned) if learned.is_supported() => (learned, false),
            Ok(_) => {
                return Err(LoadError {
                    path: stable_path,
                    source: io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Unsupported learned-marker format",
                    ),
                })
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                let path = dir.join("learned.json");
                let legacy = load_json(&path).map_err(|source| LoadError { path, source })?;
                let learned = Learned::from_legacy(legacy);
                let needs_migration = learned.pending_count() != 0;
                (learned, needs_migration)
            }
            Err(source) => {
                return Err(LoadError {
                    path: stable_path,
                    source,
                })
            }
        };
        let stats_path = dir.join("stats.json");
        let stats = crate::stats::load_stats(&stats_path).map_err(|source| LoadError {
            path: stats_path,
            source,
        })?;
        Ok(Self {
            learned,
            stats,
            needs_migration,
        })
    }

    pub fn open(dir: &Path) -> Result<Self, LoadError> {
        let profile = Self::load(dir)?;
        if profile.needs_migration {
            let path = dir.join(FILE_NAME);
            // The legacy file is neither renamed nor rewritten, including on failure.
            let save = || -> io::Result<()> {
                let temp = write_json_temp(&path, &profile.learned)?;
                temp.persist_noclobber(&path).map_err(|error| error.error)?;
                Ok(())
            };
            save().map_err(|source| LoadError { path, source })?;
        }
        Ok(profile)
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    serde_json::from_slice(&fs::read(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub fn load_json<T: DeserializeOwned + Default>(path: &Path) -> io::Result<T> {
    match read_json(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        result => result,
    }
}

pub fn save_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> io::Result<()> {
    write_json_temp(path, value)?
        .persist(path)
        .map_err(|error| error.error)?;
    Ok(())
}

fn write_json_temp<T: Serialize + ?Sized>(
    path: &Path,
    value: &T,
) -> io::Result<tempfile::NamedTempFile> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let parent = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    Ok(temp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learned::LegacyMarkers;
    use crate::stats::{save_stats, DailyStat};
    use std::collections::HashMap;

    fn sample_learned() -> LegacyMarkers {
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
        assert_eq!(profile.learned.pending_count(), 0);
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
        assert_eq!(restored.learned, Learned::from_legacy(sample_learned()));
        assert_eq!(
            serde_json::to_value(restored.stats).unwrap(),
            serde_json::to_value(sample_stats()).unwrap()
        );
        assert_eq!(bytes(dir.path()), before);
    }

    #[test]
    fn opening_legacy_profile_preserves_original_and_uses_only_v2_on_restart() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("learned.json");
        let stable = dir.path().join(FILE_NAME);
        let original = br#"{ "my-deck": [4, 0, 7], "missing": [999] }"#;
        fs::write(&legacy, original).unwrap();
        let mut data = ProfileData::open(dir.path()).unwrap();
        assert!(stable.exists());
        assert!(data.learned.ids("my-deck").is_empty());
        assert_eq!(data.learned.pending("my-deck"), [4, 0, 7]);
        let ids = ["a".repeat(64)];
        assert_eq!(
            data.learned
                .resolve("my-deck", &ids, &Learned::order_token(&ids)),
            1
        );
        save_json(&stable, &data.learned).unwrap();
        let restarted = ProfileData::open(dir.path()).unwrap();
        assert_eq!(restarted.learned.ids("my-deck"), &ids);
        assert_eq!(restarted.learned.pending("my-deck"), [4, 7]);
        assert_eq!(restarted.learned.pending("missing"), [999]);
        assert_eq!(fs::read(legacy).unwrap(), original);
    }

    #[test]
    fn invalid_stats_prevent_migration_and_invalid_v2_does_not_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        save_json(&dir.path().join("learned.json"), &sample_learned()).unwrap();
        let stats = dir.path().join("stats.json");
        let stable = dir.path().join(FILE_NAME);
        fs::write(&stats, b"{").unwrap();
        assert_eq!(ProfileData::open(dir.path()).unwrap_err().path, stats);
        assert!(!stable.exists());
        save_stats(&stats, &sample_stats()).unwrap();
        for invalid in [br#"{"version":9,"decks":{},"pending":{}}"#.as_slice(), b"{"] {
            fs::write(&stable, invalid).unwrap();
            let before = bytes(dir.path());
            assert_eq!(ProfileData::open(dir.path()).unwrap_err().path, stable);
            assert_eq!(bytes(dir.path()), before);
        }
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
        save_json(&learned_path, &LegacyMarkers::new()).unwrap();
        save_stats(&stats_path, &Stats::default()).unwrap();
        save_json(&learned_path, &sample_learned()).unwrap();
        save_stats(&stats_path, &sample_stats()).unwrap();
        let restored = ProfileData::load(&root).unwrap();
        assert_eq!(restored.learned, Learned::from_legacy(sample_learned()));
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
                save_json(path, &LegacyMarkers::new())
            } else {
                save_stats(path, &Stats::default())
            };
            assert!(result.is_err());
            assert_eq!(bytes(dir.path()), before);
            drop(lock);
        }
        save_json(&learned, &LegacyMarkers::new()).unwrap();
        save_stats(&stats, &Stats::default()).unwrap();
        let restored = ProfileData::load(dir.path()).unwrap();
        assert_eq!(restored.learned.pending_count(), 0);
        assert!(restored.stats.days.is_empty());
    }
}

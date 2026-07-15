//! Durable per-watcher candle progress — the restart-backfill ledger.
//!
//! The watchers evaluate candles as they close; when the process is down
//! (machine shutdown, crash) candles close unobserved, and on restart the
//! candle sources resume at the *current* candle — the interior gap was
//! silently skipped. This store remembers the last candle each instrument
//! actually evaluated so the next startup can replay exactly the candles
//! that closed while the watcher was down (see
//! `MultiInstrumentWatcher::backfill_instrument`).
//!
//! One JSON file per watcher under `<data home>/watch-state/`
//! (`~/.wickd/watch-state/<watcher_id>.json`), mapping instrument →
//! RFC3339 time of the last evaluated candle. Writes go through a temp
//! file + rename so a mid-write crash never corrupts the ledger. Losing
//! the file is safe: no state means no backfill, which is exactly the
//! pre-existing behavior.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use tracing::warn;

/// Persistent map of instrument → last-evaluated candle time for one watcher.
pub struct WatchStateStore {
    /// File this watcher's state lives in.
    path: PathBuf,
    /// In-memory copy of the ledger (instrument → last evaluated candle time).
    last_evaluated: HashMap<String, DateTime<Utc>>,
}

impl WatchStateStore {
    /// Default state directory under the wickd data home.
    pub fn default_dir() -> Result<PathBuf, String> {
        Ok(crate::paths::wickd_data_home()?.join("watch-state"))
    }

    /// Open (or create) the state file for `watcher_id` under `dir`.
    ///
    /// An unreadable/corrupt existing file is treated as empty (with a
    /// warning) rather than an error: the ledger is an optimization, and
    /// refusing to start the watcher over it would be worse than skipping
    /// one backfill.
    pub fn open(dir: &Path, watcher_id: &str) -> Result<Self, String> {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("creating watch-state dir {}: {}", dir.display(), e))?;

        // watcher_ids are CLI/label-derived; keep the filename safe anyway.
        let safe_id: String = watcher_id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let path = dir.join(format!("{}.json", safe_id));

        let last_evaluated = match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<HashMap<String, DateTime<Utc>>>(&raw) {
                Ok(map) => map,
                Err(e) => {
                    warn!(
                        "[WatchState] {} is corrupt ({}); starting with empty state",
                        path.display(),
                        e
                    );
                    HashMap::new()
                }
            },
            Err(_) => HashMap::new(), // first run, or unreadable — same outcome
        };

        Ok(Self { path, last_evaluated })
    }

    /// Last evaluated candle time recorded for `instrument`, if any.
    pub fn last_evaluated(&self, instrument: &str) -> Option<DateTime<Utc>> {
        self.last_evaluated.get(instrument).copied()
    }

    /// Record that `instrument`'s candle at `time` has been evaluated, and
    /// persist the ledger. Persist failures are logged, never fatal — the
    /// watcher must keep running even if the disk is unhappy.
    pub fn record(&mut self, instrument: &str, time: DateTime<Utc>) {
        // Never move the ledger backwards (a replayed/duplicate candle must
        // not shrink the covered range).
        if let Some(prev) = self.last_evaluated.get(instrument) {
            if time <= *prev {
                return;
            }
        }
        self.last_evaluated.insert(instrument.to_string(), time);

        if let Err(e) = self.persist() {
            warn!("[WatchState] failed to persist {}: {}", self.path.display(), e);
        }
    }

    /// Atomic write: serialize to a sibling temp file, then rename over the
    /// real path.
    fn persist(&self) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.last_evaluated)
            .map_err(|e| format!("serializing watch state: {}", e))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("writing {}: {}", tmp.display(), e))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("renaming {} into place: {}", tmp.display(), e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn round_trips_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = WatchStateStore::open(dir.path(), "watch-test-H8").unwrap();
        assert_eq!(store.last_evaluated("EUR_USD"), None);

        store.record("EUR_USD", t("2026-07-14T10:00:00Z"));
        store.record("GBP_USD", t("2026-07-14T18:00:00Z"));

        let reopened = WatchStateStore::open(dir.path(), "watch-test-H8").unwrap();
        assert_eq!(reopened.last_evaluated("EUR_USD"), Some(t("2026-07-14T10:00:00Z")));
        assert_eq!(reopened.last_evaluated("GBP_USD"), Some(t("2026-07-14T18:00:00Z")));
    }

    #[test]
    fn record_never_moves_backwards() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = WatchStateStore::open(dir.path(), "watch-test-H8").unwrap();
        store.record("EUR_USD", t("2026-07-14T10:00:00Z"));
        store.record("EUR_USD", t("2026-07-13T10:00:00Z")); // stale — ignored
        assert_eq!(store.last_evaluated("EUR_USD"), Some(t("2026-07-14T10:00:00Z")));
    }

    #[test]
    fn corrupt_file_starts_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("watch-bad-H4.json"), "not json").unwrap();
        let store = WatchStateStore::open(dir.path(), "watch-bad-H4").unwrap();
        assert_eq!(store.last_evaluated("EUR_USD"), None);
    }

    #[test]
    fn watcher_ids_are_sanitized_into_filenames() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = WatchStateStore::open(dir.path(), "watch/../evil id").unwrap();
        store.record("EUR_USD", t("2026-07-14T10:00:00Z"));
        // Everything unsafe collapsed to '_' — file stays inside dir.
        assert!(dir.path().join("watch____evil_id.json").exists());
    }
}

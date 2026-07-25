//! Durable alert queue — the agent-pollable delivery store (AGT-620).
//!
//! `wickd` fires two kinds of alert on two separate long-running commands:
//! price-level crossings (`wickd alert run`, via `wickd alert run`'s sink)
//! and strategy-signal Buy/Sell alerts (`wickd watch`, via
//! `wickd watch`'s signal-alert sink). Neither of those is durable on its own:
//! a fire is an NDJSON line on stdout that scrolls away. An agent that wants to
//! *react* to alerts needs a store it can poll/tail across invocations.
//!
//! This module is that store: an **append-only NDJSON log** at
//! `~/.wickd/alert-queue.ndjson`. Append-only is the natural shape for a
//! poll/tail feed — new events land at the end, and `wickd queue list
//! [--follow]` reads them back (AC2). Each line is one [`QueuedAlert`].
//!
//! ## D3 — alerts and execution-proposals never share a store (AC1)
//!
//! This file is deliberately **separate** from `~/.wickd/pending.json` (the
//! execution-proposal store owned by [`crate::pending`]). An alert landing in
//! this queue is *not* an execution proposal and never auto-becomes one. The
//! only bridge from here to `pending.json` is the explicit `wickd queue
//! promote <id>` action (AC3, see `wickd queue`) — and only for
//! strategy-signal (Buy/Sell) alerts, which carry an order intent. Price-level
//! alerts are not promotable: a "EUR_USD crossed 1.0900" event says nothing
//! about a side or size, so it has no proposal to promote into.
//!
//! ## Schema (`~/.wickd/alert-queue.ndjson`)
//!
//! One JSON object per line, e.g.:
//!
//! ```jsonc
//! {"id":"<queue-uuid>","ts":"2026-06-30T00:00:00+00:00",
//!  "payload":{"kind":"strategy-signal","instrument":"EUR_USD","signal":"buy",
//!             "proposal":{ /* a full pending::PendingSignal */ }}}
//! {"id":"<queue-uuid>","ts":"2026-06-30T00:00:05Z",
//!  "payload":{"kind":"price-level","instrument":"EUR_USD","level":"1.0900",
//!             "direction":"cross-up","price":"1.0905"}}
//! ```

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use std::str::FromStr;

use anyhow::anyhow;

use crate::pending::PendingSignal;
use crate::shared::PositionDirection;

/// Which way a price-level cross must go to fire an alert. Lives here (the
/// queue wire format) since AGT-652 moved the daemon's client-visible contract
/// into wickd-core; the CLI's `alert` module re-exports it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    /// Fires when price crosses from below the level to at/above it.
    CrossUp,
    /// Fires when price crosses from above the level to at/below it.
    CrossDown,
    /// Fires on a cross in either direction.
    Either,
}

impl FromStr for Direction {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "cross-up" | "crossup" | "up" => Ok(Direction::CrossUp),
            "cross-down" | "crossdown" | "down" => Ok(Direction::CrossDown),
            "either" | "both" => Ok(Direction::Either),
            other => Err(anyhow!(
                "unknown direction '{other}' (expected cross-up, cross-down, or either)"
            )),
        }
    }
}

/// The actionable half of a strategy's per-candle evaluation (Buy/Sell).
/// Serializes `buy`/`sell` on the queue wire; the CLI's `signal_alert` module
/// re-exports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertSignal {
    Buy,
    Sell,
}

impl AlertSignal {
    /// Classify a position direction as its alert signal.
    pub fn from_direction(direction: PositionDirection) -> Self {
        match direction {
            PositionDirection::Long => AlertSignal::Buy,
            PositionDirection::Short => AlertSignal::Sell,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AlertSignal::Buy => "buy",
            AlertSignal::Sell => "sell",
        }
    }
}

/// Queue file name under `~/.wickd/`.
pub const QUEUE_FILE: &str = "alert-queue.ndjson";

/// Rotate the live queue once it exceeds this many bytes (issue #11 / AGT-777).
///
/// Measured growth is ~9KB/day, so 1 MiB is roughly four months of history in
/// the live file and (with [`QUEUE_ARCHIVE_GENERATIONS`]) about a year on disk
/// before the oldest generation falls off — bounded at ~3 MiB total.
///
/// The trigger is **bytes, not entries**, so [`rotate_at`] is a single `stat`
/// on the append path rather than a full parse of the file it is about to
/// append to.
pub const MAX_QUEUE_BYTES: u64 = 1024 * 1024;

/// How many rotated archives are kept beside the live queue
/// (`alert-queue.1.ndjson`, `alert-queue.2.ndjson`). Readers consult them
/// newest-generation-first when the live file is shorter than the requested
/// limit, so rotation never shortens a bounded read.
pub const QUEUE_ARCHIVE_GENERATIONS: u32 = 2;

/// The kind-tagged payload of a queued alert.
///
/// Internally tagged on `kind` so a reader can branch on the alert type without
/// positional guessing, and so [`QueuedAlert::promotable_proposal`] can hand
/// back the embedded proposal for exactly the one promotable variant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum QueuedPayload {
    /// A price-level alert fired (`wickd alert run`). **Not promotable** — a
    /// bare level crossing carries no side/size, so there is no order intent to
    /// bridge into a pending proposal.
    PriceLevel {
        instrument: String,
        /// The level that was crossed (OANDA precision, as a string).
        level: String,
        /// The typed cross direction (serializes `cross-up`/`cross-down`/`either`).
        direction: Direction,
        /// The price that triggered the fire.
        price: String,
    },
    /// A strategy-signal Buy/Sell alert (`wickd watch`). **Promotable**: it
    /// carries the fully-formed [`PendingSignal`] the fire maps to, so `wickd
    /// queue promote <id>` can append that proposal into `pending.json`.
    StrategySignal {
        instrument: String,
        /// The typed Buy/Sell signal (serializes `buy`/`sell`).
        signal: AlertSignal,
        /// The execution proposal this alert promotes into. Built at fire time
        /// via [`crate::pending::pending_from_match`] so a promotion is a pure
        /// move of an already-well-formed record, never a re-derivation.
        /// Boxed: the proposal dwarfs the price-level variant
        /// (clippy::large_enum_variant); serde is transparent to the Box.
        proposal: Box<PendingSignal>,
        /// The watcher's `--account` (issue #8). Distinguishes otherwise
        /// identical rows when several watchers run the same strategy/pair on
        /// different accounts. `Option` + `default` so rows queued before this
        /// field existed still parse; `skip_serializing_if` keeps new rows
        /// readable by pre-field consumers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<String>,
        /// The watcher's candle granularity, e.g. `M5` (issue #8). Same
        /// backward-compatibility contract as `account`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        granularity: Option<String>,
    },
}

/// One entry in the alert queue: a stable id for reference/promotion, the fire
/// timestamp, and the kind-tagged payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueuedAlert {
    /// Stable queue-entry id (`wickd queue promote <id>` names this). A fresh
    /// uuid per entry, distinct from any id inside the payload — a re-fire is a
    /// genuinely new queue event even when it repeats a level or a signal.
    pub id: String,
    /// RFC3339 timestamp of the fire (the source event's own time).
    pub ts: String,
    pub payload: QueuedPayload,
}

impl QueuedAlert {
    fn new_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Build a queued price-level alert (not promotable).
    pub fn price_level(
        ts: String,
        instrument: String,
        level: String,
        direction: Direction,
        price: String,
    ) -> Self {
        Self {
            id: Self::new_id(),
            ts,
            payload: QueuedPayload::PriceLevel { instrument, level, direction, price },
        }
    }

    /// Build a queued strategy-signal alert from the fire's [`PendingSignal`]
    /// proposal (promotable). `signal` is the typed Buy/Sell classification.
    /// `account` / `granularity` identify WHICH watcher fired (issue #8) —
    /// pass them when known; `None` only for legacy/unknown-origin fires.
    pub fn strategy_signal(
        ts: String,
        signal: AlertSignal,
        proposal: PendingSignal,
        account: Option<String>,
        granularity: Option<String>,
    ) -> Self {
        Self {
            id: Self::new_id(),
            ts,
            payload: QueuedPayload::StrategySignal {
                instrument: proposal.instrument.clone(),
                signal,
                proposal: Box::new(proposal),
                account,
                granularity,
            },
        }
    }

    /// The execution proposal this alert promotes into, or `None` if it is not
    /// a promotable (strategy-signal) alert. This is the AC3 gate: only a
    /// strategy-signal alert yields a proposal; a price-level alert never does.
    pub fn promotable_proposal(&self) -> Option<&PendingSignal> {
        match &self.payload {
            QueuedPayload::StrategySignal { proposal, .. } => Some(proposal),
            QueuedPayload::PriceLevel { .. } => None,
        }
    }
}

/// Path to the alert queue (`<data home>/alert-queue.ndjson`;
/// `~/.wickd/alert-queue.ndjson` unless `WICKD_HOME` overrides the data home —
/// tests/smokes only, never live).
pub fn queue_path() -> Result<PathBuf> {
    let home = crate::paths::wickd_data_home().map_err(anyhow::Error::msg)?;
    Ok(home.join(QUEUE_FILE))
}

/// Path of rotated archive generation `generation` (1 = most recent) beside
/// the queue at `path`: `alert-queue.ndjson` → `alert-queue.1.ndjson`.
fn archive_path(path: &Path, generation: u32) -> PathBuf {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or(QUEUE_FILE);
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, "ndjson"));
    path.with_file_name(format!("{stem}.{generation}.{ext}"))
}

/// Bound the queue on disk by **rotating** it: over `max_bytes`, shift the
/// archive generations down and `rename` the live file to
/// `<queue>.1.ndjson`, leaving the next [`append_at`] to create a fresh one.
/// Returns whether a rotation happened. A missing or under-cap queue is a
/// no-op.
///
/// ## Why rotate rather than rewrite (AGT-777 AC2)
///
/// [`crate::feed::prune_at`] bounds `feed.ndjson` by reading it, writing a
/// truncated copy, and renaming that over the original. **That is not safe
/// here.** This queue has two independent writer processes — `signal_alert.rs`
/// (strategy signals) and `sink.rs` (price levels) — so anything appended
/// between the read and the rename is silently destroyed by the rewrite. It
/// would fail invisibly: no error, and only once the file is big enough to
/// prune, i.e. months after the code looks like it works.
///
/// Rotation has no such window. [`append_at`] reopens the path on every call,
/// so an append that races the `rename` holds a descriptor on the *renamed*
/// inode and its entry lands in the archive — which readers still consult (see
/// [`list_tail_at`]) — instead of vanishing. An append that opens after the
/// rename creates the new live file. Either way the entry is durable, with no
/// lock anywhere on the daemon's append path (AC4).
///
/// Two generations rather than one for the same reason. If two writers cross
/// the cap simultaneously, the loser can rotate a freshly-created (nearly
/// empty) live file a moment after the winner rotated the full one. With a
/// single archive that would clobber the just-archived history; with
/// generations it merely shifts it to `.2`, where readers still find it.
pub fn rotate_at(path: impl AsRef<Path>, max_bytes: u64) -> Result<bool> {
    let path = path.as_ref();
    let Ok(meta) = std::fs::metadata(path) else {
        return Ok(false); // no queue yet — nothing to rotate
    };
    if meta.len() <= max_bytes {
        return Ok(false);
    }

    // Shift oldest-first so no generation overwrites one still in use; the
    // last generation falls off the end, which is the retention bound. These
    // are best-effort: a missing generation (or one another process just
    // moved) must not stop the live file from being rotated.
    for generation in (1..QUEUE_ARCHIVE_GENERATIONS).rev() {
        let from = archive_path(path, generation);
        let to = archive_path(path, generation + 1);
        let _ = std::fs::rename(&from, &to);
    }

    let archive = archive_path(path, 1);
    std::fs::rename(path, &archive)
        .with_context(|| format!("rotating {} → {}", path.display(), archive.display()))?;
    Ok(true)
}

/// Append one alert to the append-only log at `path` (creating the parent dir),
/// as a single NDJSON line. Tests pass a temp path so they never touch the real
/// `~/.wickd/alert-queue.ndjson`.
///
/// Retention rides here (AGT-777): the queue is rotated first if it has grown
/// past [`MAX_QUEUE_BYTES`], so the two writer daemons get retention without
/// either of them owning it. A rotation failure is deliberately swallowed —
/// recording the alert matters more than bounding the file, and the caller's
/// error path is about the alert.
pub fn append_at(path: impl AsRef<Path>, entry: &QueuedAlert) -> Result<()> {
    let path = path.as_ref();
    let _ = rotate_at(path, MAX_QUEUE_BYTES);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create alert-queue dir {}", parent.display()))?;
    }
    let mut line = serde_json::to_string(entry).context("could not serialize queued alert")?;
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening alert queue at {}", path.display()))?;
    // One `write_all` of line-plus-newline, NOT `writeln!`. Both writer
    // processes append to this file concurrently, and `writeln!` emits the
    // payload and the newline as separate `write` calls — under O_APPEND each
    // is atomic on its own, so two interleaved appends can land as
    // `<json-a><json-b>\n\n` and corrupt both entries. A single write of the
    // whole line is what makes O_APPEND's atomicity cover a whole entry.
    file.write_all(line.as_bytes())
        .with_context(|| format!("appending to alert queue at {}", path.display()))?;
    Ok(())
}

/// Read every queued alert from `path`, oldest first (file/append order — the
/// natural order for a tail). Returns an empty vec if the queue does not exist.
pub fn list_at(path: impl AsRef<Path>) -> Result<Vec<QueuedAlert>> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(vec![]);
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading alert queue at {}", path.display()))?;
    let mut out = Vec::new();
    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let entry: QueuedAlert = serde_json::from_str(line)
            .with_context(|| format!("alert-queue line {} is not valid JSON", i + 1))?;
        out.push(entry);
    }
    Ok(out)
}

/// Read the most recent `limit` queued alerts, oldest first — from the live
/// queue at `path`, falling back to the rotated archives when the live file
/// holds fewer than `limit` entries.
///
/// Every consumer of this queue only ever wants the tail. [`list_at`] parses
/// every line to return the last hundred; this parses only the lines it
/// returns. That matters because the desktop feed drawer polls this path every
/// 5 seconds while open.
///
/// The archive fallback is what keeps [`rotate_at`] invisible to readers: the
/// tick after a rotation, the live file holds one entry, and a naive tail
/// would return a feed of one. Archives are consulted newest-generation-first
/// and their entries are prepended, so the result is always in queue order.
///
/// Splitting lines still walks the whole file — the win is skipping N JSON
/// parses, which dominate. A malformed line inside the returned window is
/// still a hard error, same as `list_at`: silently dropping entries from a
/// trading audit trail is worse than failing loudly. Lines *outside* the
/// window are never parsed, so an old corrupt entry can no longer break a
/// reader that does not care about it.
pub fn list_tail_at(path: impl AsRef<Path>, limit: usize) -> Result<Vec<QueuedAlert>> {
    let path = path.as_ref();
    let mut out = tail_of_file(path, limit)?;
    let mut generation = 1;
    while out.len() < limit && generation <= QUEUE_ARCHIVE_GENERATIONS {
        // Older entries belong in front of what the live file gave us.
        let mut older = tail_of_file(archive_path(path, generation), limit - out.len())?;
        older.append(&mut out);
        out = older;
        generation += 1;
    }
    Ok(out)
}

/// The last `limit` entries of a single NDJSON file, oldest first. The
/// one-file half of [`list_tail_at`]; a missing file reads as empty.
fn tail_of_file(path: impl AsRef<Path>, limit: usize) -> Result<Vec<QueuedAlert>> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(vec![]);
    }
    if limit == 0 {
        return Ok(vec![]);
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading alert queue at {}", path.display()))?;

    // Collect as (1-based line number, text) so an error still names the real
    // line in the file, not an offset into the returned window.
    let mut lines: Vec<(usize, &str)> = Vec::new();
    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        lines.push((i + 1, line));
    }
    let start = lines.len().saturating_sub(limit);

    let mut out = Vec::with_capacity(lines.len() - start);
    for (lineno, line) in &lines[start..] {
        let entry: QueuedAlert = serde_json::from_str(line)
            .with_context(|| format!("alert-queue line {lineno} is not valid JSON"))?;
        out.push(entry);
    }
    Ok(out)
}

/// Fetch a single queued alert by its queue-entry id, from the live queue at
/// `path` or, failing that, from the rotated archives.
///
/// The archives are searched because [`list_tail_at`] can hand a UI an entry
/// that has already rotated out of the live file; `wickd queue promote <id>`
/// on a row the feed is still showing must not fail as "unknown id".
pub fn get_at(path: impl AsRef<Path>, id: &str) -> Result<Option<QueuedAlert>> {
    let path = path.as_ref();
    if let Some(entry) = list_at(path)?.into_iter().find(|e| e.id == id) {
        return Ok(Some(entry));
    }
    for generation in 1..=QUEUE_ARCHIVE_GENERATIONS {
        let archive = archive_path(path, generation);
        if !archive.exists() {
            continue;
        }
        if let Some(entry) = list_at(&archive)?.into_iter().find(|e| e.id == id) {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

/// The entries of `entries` that follow `last_id`, for pollers that tail the
/// queue across rotations.
///
/// A poller that remembers a *count* breaks the moment [`rotate_at`] runs: the
/// live file shrinks below the remembered length and the poller either goes
/// permanently silent or, once it grows back, skips whatever it stepped over.
/// Remembering the last id instead is rotation-proof — an id that is no longer
/// in the file means the file was rotated, and everything now in it is new.
pub fn entries_after<'a>(entries: &'a [QueuedAlert], last_id: Option<&str>) -> &'a [QueuedAlert] {
    match last_id.and_then(|id| entries.iter().position(|e| e.id == id)) {
        Some(i) => &entries[i + 1..],
        None => entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending::STATUS_PENDING;

    fn temp_queue() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let mut p = std::env::temp_dir();
        p.push(format!("wickd-queue-test-{pid}-{nanos}-{n}.ndjson"));
        p
    }

    fn sample_proposal(id: &str, side: &str, units: i64) -> PendingSignal {
        PendingSignal {
            id: id.to_string(),
            ts: "2026-06-30T00:00:00+00:00".to_string(),
            instrument: "EUR_USD".to_string(),
            side: side.to_string(),
            units,
            suggested_units: None,
            strategy: "ma-crossover".to_string(),
            reason: "fast SMA crossed above slow".to_string(),
            sl: Some("1.0800".to_string()),
            tp: Some("1.0950".to_string()),
            entry_price: Some("1.0850".to_string()),
            status: STATUS_PENDING.to_string(),
        }
    }

    // AC1/AC2: append → list round-trip on a temp path. Order is preserved
    // (oldest first, as appended — tail order), and get_at finds by id.
    #[test]
    fn append_list_round_trip_preserves_order() {
        let path = temp_queue();

        let a = QueuedAlert::strategy_signal(
            "2026-06-30T00:00:00Z".to_string(),
            AlertSignal::Buy,
            sample_proposal("match-1", "long", 1000),
            Some("h004".to_string()),
            Some("M5".to_string()),
        );
        let b = QueuedAlert::price_level(
            "2026-06-30T00:00:05Z".to_string(),
            "EUR_USD".to_string(),
            "1.0900".to_string(),
            Direction::CrossUp,
            "1.0905".to_string(),
        );

        append_at(&path, &a).unwrap();
        append_at(&path, &b).unwrap();

        let listed = list_at(&path).unwrap();
        assert_eq!(listed.len(), 2);
        // Oldest first: a was appended first.
        assert_eq!(listed[0].id, a.id);
        assert_eq!(listed[1].id, b.id);

        // get_at resolves each by its queue-entry id.
        assert_eq!(get_at(&path, &a.id).unwrap().unwrap(), a);
        assert_eq!(get_at(&path, &b.id).unwrap().unwrap(), b);
        assert!(get_at(&path, "nope").unwrap().is_none());

        let _ = std::fs::remove_file(&path);
    }

    // AC3 gate: only strategy-signal alerts expose a promotable proposal.
    // ── list_tail_at (issue #11: bound the read, not just the file) ───────

    fn level_alert(ts: &str, price: &str) -> QueuedAlert {
        QueuedAlert::price_level(
            ts.to_string(),
            "EUR_USD".to_string(),
            "1.0900".to_string(),
            Direction::CrossUp,
            price.to_string(),
        )
    }

    #[test]
    fn tail_returns_the_last_n_oldest_first() {
        let path = temp_queue();
        for i in 0..10 {
            append_at(&path, &level_alert(&format!("2026-06-30T00:00:{i:02}Z"), "1.09")).unwrap();
        }

        let tail = list_tail_at(&path, 3).unwrap();

        assert_eq!(tail.len(), 3);
        assert_eq!(tail[0].ts, "2026-06-30T00:00:07Z");
        assert_eq!(tail[2].ts, "2026-06-30T00:00:09Z");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn tail_returns_everything_when_the_queue_is_shorter_than_the_limit() {
        let path = temp_queue();
        append_at(&path, &level_alert("2026-06-30T00:00:00Z", "1.09")).unwrap();

        assert_eq!(list_tail_at(&path, 100).unwrap().len(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn tail_of_a_missing_queue_is_empty() {
        assert!(list_tail_at(temp_queue(), 10).unwrap().is_empty());
    }

    #[test]
    fn tail_with_a_zero_limit_is_empty() {
        let path = temp_queue();
        append_at(&path, &level_alert("2026-06-30T00:00:00Z", "1.09")).unwrap();

        assert!(list_tail_at(&path, 0).unwrap().is_empty());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn tail_agrees_with_list_at() {
        // The tail must be a suffix of the full read — a faster path that
        // returned different entries would be worse than a slow one.
        let path = temp_queue();
        for i in 0..5 {
            append_at(&path, &level_alert(&format!("2026-06-30T00:00:{i:02}Z"), "1.09")).unwrap();
        }

        let full = list_at(&path).unwrap();
        let tail = list_tail_at(&path, 2).unwrap();

        assert_eq!(tail, full[full.len() - 2..].to_vec());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn tail_ignores_corruption_outside_the_window() {
        // A malformed old line must not break a reader that only wants recent
        // entries — the whole point of not parsing the history every poll.
        let path = temp_queue();
        std::fs::write(&path, "{ not json at all\n").unwrap();
        append_at(&path, &level_alert("2026-06-30T00:00:01Z", "1.09")).unwrap();
        append_at(&path, &level_alert("2026-06-30T00:00:02Z", "1.09")).unwrap();

        let tail = list_tail_at(&path, 2).unwrap();

        assert_eq!(tail.len(), 2);
        // The full read still fails on it — corruption is not being hidden.
        assert!(list_at(&path).is_err());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn tail_still_fails_on_corruption_inside_the_window() {
        // Silently dropping entries from a trading audit trail would be worse
        // than failing loudly.
        let path = temp_queue();
        append_at(&path, &level_alert("2026-06-30T00:00:01Z", "1.09")).unwrap();
        std::fs::write(
            &path,
            format!("{}{{ broken\n", std::fs::read_to_string(&path).unwrap()),
        )
        .unwrap();

        let err = list_tail_at(&path, 5).unwrap_err();

        assert!(format!("{err:#}").contains("line 2"), "should name the line: {err:#}");
        std::fs::remove_file(&path).ok();
    }

    // ── rotation (AGT-777: retention that cannot lose a racing append) ────

    /// Remove a queue and every archive generation it may have left behind.
    fn cleanup(path: &Path) {
        std::fs::remove_file(path).ok();
        for generation in 1..=QUEUE_ARCHIVE_GENERATIONS {
            std::fs::remove_file(archive_path(path, generation)).ok();
        }
    }

    #[test]
    fn rotation_is_a_no_op_below_the_cap() {
        let path = temp_queue();
        append_at(&path, &level_alert("2026-06-30T00:00:00Z", "1.09")).unwrap();

        assert!(!rotate_at(&path, MAX_QUEUE_BYTES).unwrap());
        assert!(!archive_path(&path, 1).exists());
        assert_eq!(list_at(&path).unwrap().len(), 1);
        cleanup(&path);
    }

    #[test]
    fn rotation_is_a_no_op_when_there_is_no_queue() {
        let path = temp_queue();
        assert!(!rotate_at(&path, 0).unwrap());
        cleanup(&path);
    }

    #[test]
    fn rotation_archives_the_live_file_and_appends_start_a_fresh_one() {
        let path = temp_queue();
        append_at(&path, &level_alert("2026-06-30T00:00:00Z", "1.09")).unwrap();

        assert!(rotate_at(&path, 0).unwrap());

        assert!(!path.exists(), "the live queue is renamed away, not copied");
        assert_eq!(list_at(archive_path(&path, 1)).unwrap().len(), 1);

        append_at(&path, &level_alert("2026-06-30T00:00:01Z", "1.10")).unwrap();
        assert_eq!(list_at(&path).unwrap().len(), 1, "a fresh live file");
        cleanup(&path);
    }

    #[test]
    fn rotation_shifts_generations_and_drops_the_oldest() {
        let path = temp_queue();

        // Rotate once per entry, so each generation holds a known entry.
        for i in 0..(QUEUE_ARCHIVE_GENERATIONS + 1) {
            append_at(&path, &level_alert(&format!("2026-06-30T00:00:{i:02}Z"), "1.09")).unwrap();
            assert!(rotate_at(&path, 0).unwrap());
        }

        // Newest archive first: .1 is the most recently rotated.
        assert_eq!(list_at(archive_path(&path, 1)).unwrap()[0].ts, "2026-06-30T00:00:02Z");
        assert_eq!(list_at(archive_path(&path, 2)).unwrap()[0].ts, "2026-06-30T00:00:01Z");
        assert!(
            !archive_path(&path, 3).exists(),
            "retention bound: the oldest generation falls off"
        );
        cleanup(&path);
    }

    // AC3: a bounded read is not shortened by a rotation — the archives are
    // consulted, in queue order, until the limit is satisfied.
    #[test]
    fn tail_reads_through_the_archives_when_the_live_file_is_short() {
        let path = temp_queue();
        for i in 0..3 {
            append_at(&path, &level_alert(&format!("2026-06-30T00:00:{i:02}Z"), "1.09")).unwrap();
            rotate_at(&path, 0).unwrap();
        }
        append_at(&path, &level_alert("2026-06-30T00:00:03Z", "1.09")).unwrap();

        let tail = list_tail_at(&path, 10).unwrap();

        // One entry per file: .2, .1, live — oldest first, no duplicates.
        let stamps: Vec<_> = tail.iter().map(|e| e.ts.as_str()).collect();
        assert_eq!(
            stamps,
            ["2026-06-30T00:00:01Z", "2026-06-30T00:00:02Z", "2026-06-30T00:00:03Z"]
        );
        cleanup(&path);
    }

    #[test]
    fn tail_stops_at_the_limit_without_touching_older_generations() {
        let path = temp_queue();
        for i in 0..3 {
            append_at(&path, &level_alert(&format!("2026-06-30T00:00:{i:02}Z"), "1.09")).unwrap();
            rotate_at(&path, 0).unwrap();
        }
        append_at(&path, &level_alert("2026-06-30T00:00:03Z", "1.09")).unwrap();

        let tail = list_tail_at(&path, 2).unwrap();

        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].ts, "2026-06-30T00:00:02Z");
        assert_eq!(tail[1].ts, "2026-06-30T00:00:03Z");
        cleanup(&path);
    }

    // A promotable row the feed can still show must still be promotable after
    // the rotation that moved it out of the live file.
    #[test]
    fn get_finds_an_entry_that_has_rotated_into_an_archive() {
        let path = temp_queue();
        let archived = QueuedAlert::strategy_signal(
            "2026-06-30T00:00:00Z".to_string(),
            AlertSignal::Buy,
            sample_proposal("match-rotated", "long", 1000),
            None,
            None,
        );
        append_at(&path, &archived).unwrap();
        rotate_at(&path, 0).unwrap();
        append_at(&path, &level_alert("2026-06-30T00:00:01Z", "1.09")).unwrap();

        assert_eq!(get_at(&path, &archived.id).unwrap().unwrap(), archived);
        assert!(get_at(&path, "nope").unwrap().is_none());
        cleanup(&path);
    }

    // AC5: the retention step runs while both writers are appending, and no
    // entry is lost. This is the test a feed-style tmp+rename prune fails:
    // whatever is appended between its read and its rename is destroyed.
    #[test]
    fn concurrent_appends_survive_rotation() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let path = temp_queue();
        const WRITERS: usize = 2; // signal_alert.rs and sink.rs
        const PER_WRITER: usize = 300;
        /// ~1/6 of the storm's bytes, so rotations interleave with appends.
        const ROTATE_AFTER_BYTES: u64 = 20 * 1024;

        let stop = Arc::new(AtomicBool::new(false));
        let writers: Vec<_> = (0..WRITERS)
            .map(|w| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let mut ids = Vec::with_capacity(PER_WRITER);
                    for i in 0..PER_WRITER {
                        let entry = level_alert(&format!("w{w}-{i}"), "1.09");
                        // The live append path: non-fatal, no lock, and it is
                        // the caller that reopens the queue each time.
                        append_at(&path, &entry).unwrap();
                        ids.push(entry.id);
                    }
                    ids
                })
            })
            .collect();

        // Rotate underneath them, but no more times than there are archive
        // generations — beyond that, dropping the oldest is the intended
        // retention behaviour rather than a loss.
        let rotator = {
            let path = path.clone();
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                for _ in 0..QUEUE_ARCHIVE_GENERATIONS {
                    while !stop.load(Ordering::Relaxed) {
                        // A cap the storm crosses partway through, so each
                        // rotation lands mid-append rather than on an idle
                        // file.
                        if rotate_at(&path, ROTATE_AFTER_BYTES).unwrap_or(false) {
                            break;
                        }
                        std::thread::yield_now();
                    }
                }
            })
        };

        let mut appended: Vec<String> = Vec::new();
        for w in writers {
            appended.extend(w.join().expect("writer thread"));
        }
        stop.store(true, Ordering::Relaxed);
        rotator.join().expect("rotator thread");

        // Every appended id must still be readable somewhere: the live file or
        // the archive the rename moved its writer's descriptor into.
        let mut found: Vec<String> = list_at(&path).unwrap().into_iter().map(|e| e.id).collect();
        for generation in 1..=QUEUE_ARCHIVE_GENERATIONS {
            let archive = archive_path(&path, generation);
            if archive.exists() {
                found.extend(list_at(&archive).unwrap().into_iter().map(|e| e.id));
            }
        }

        assert_eq!(appended.len(), WRITERS * PER_WRITER);
        let found: std::collections::HashSet<_> = found.into_iter().collect();
        let missing: Vec<_> = appended.iter().filter(|id| !found.contains(*id)).collect();
        assert!(missing.is_empty(), "{} appended entries were lost", missing.len());
        cleanup(&path);
    }

    // ── entries_after: rotation-proof polling ─────────────────────────────

    #[test]
    fn entries_after_returns_only_what_follows_the_last_seen_id() {
        let a = level_alert("2026-06-30T00:00:00Z", "1.09");
        let b = level_alert("2026-06-30T00:00:01Z", "1.09");
        let c = level_alert("2026-06-30T00:00:02Z", "1.09");
        let entries = vec![a.clone(), b.clone(), c.clone()];

        assert_eq!(entries_after(&entries, Some(&a.id)), &entries[1..]);
        assert_eq!(entries_after(&entries, Some(&c.id)), &[] as &[QueuedAlert]);
    }

    #[test]
    fn entries_after_treats_a_rotated_away_id_as_an_all_new_file() {
        // The poller's last id is in the archive now; everything in the fresh
        // live file is unseen. A count-based poller would go silent here.
        let entries = vec![level_alert("2026-06-30T00:00:03Z", "1.09")];

        assert_eq!(entries_after(&entries, Some("gone-with-the-rotation")), &entries[..]);
        assert_eq!(entries_after(&entries, None), &entries[..]);
    }

    #[test]
    fn only_strategy_signal_is_promotable() {
        let strat = QueuedAlert::strategy_signal(
            "2026-06-30T00:00:00Z".to_string(),
            AlertSignal::Sell,
            sample_proposal("match-2", "short", -1000),
            None,
            None,
        );
        let proposal = strat.promotable_proposal().expect("strategy-signal is promotable");
        assert_eq!(proposal.id, "match-2");
        assert_eq!(proposal.side, "short");
        // The queue mirrors the proposal's instrument at the payload top level.
        assert_eq!(proposal.instrument, "EUR_USD");

        let price = QueuedAlert::price_level(
            "2026-06-30T00:00:05Z".to_string(),
            "EUR_USD".to_string(),
            "1.0900".to_string(),
            Direction::CrossUp,
            "1.0905".to_string(),
        );
        assert!(
            price.promotable_proposal().is_none(),
            "a price-level alert carries no order intent — not promotable"
        );
    }

    #[test]
    fn missing_queue_lists_empty() {
        let path = temp_queue();
        assert!(list_at(&path).unwrap().is_empty());
        assert!(get_at(&path, "anything").unwrap().is_none());
        let _ = std::fs::remove_file(&path);
    }

    // Issue #8 backward compatibility: rows queued BEFORE the account /
    // granularity fields existed must still parse (as None), and a full
    // round-trip preserves the new fields.
    #[test]
    fn strategy_signal_fields_are_backward_compatible() {
        // A verbatim pre-#8 line: no account, no granularity.
        let legacy = r#"{"id":"q-1","ts":"2026-06-30T00:00:00Z","payload":{"kind":"strategy-signal","instrument":"EUR_USD","signal":"buy","proposal":{"id":"match-1","ts":"2026-06-30T00:00:00+00:00","instrument":"EUR_USD","side":"long","units":1000,"strategy":"ma-crossover","reason":"fast SMA crossed above slow","sl":"1.0800","tp":"1.0950","entry_price":"1.0850","status":"pending"}}}"#;
        let entry: QueuedAlert = serde_json::from_str(legacy).expect("legacy row parses");
        match &entry.payload {
            QueuedPayload::StrategySignal { account, granularity, .. } => {
                assert_eq!(account, &None);
                assert_eq!(granularity, &None);
            }
            other => panic!("expected strategy-signal, got {other:?}"),
        }

        // None fields are omitted on the wire (old readers see the old shape).
        let json = serde_json::to_string(&entry).unwrap();
        assert!(!json.contains("\"account\""));
        assert!(!json.contains("\"granularity\""));

        // Round-trip with the fields set preserves them.
        let path = temp_queue();
        let tagged = QueuedAlert::strategy_signal(
            "2026-06-30T00:00:00Z".to_string(),
            AlertSignal::Buy,
            sample_proposal("match-3", "long", 1000),
            Some("tf-m5".to_string()),
            Some("M5".to_string()),
        );
        append_at(&path, &tagged).unwrap();
        let listed = list_at(&path).unwrap();
        assert_eq!(listed.len(), 1);
        match &listed[0].payload {
            QueuedPayload::StrategySignal { account, granularity, .. } => {
                assert_eq!(account.as_deref(), Some("tf-m5"));
                assert_eq!(granularity.as_deref(), Some("M5"));
            }
            other => panic!("expected strategy-signal, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    // Each entry gets its own id even when the underlying event repeats, so a
    // re-fire is addressable as a distinct queue event.
    #[test]
    fn entries_get_distinct_ids() {
        let a = QueuedAlert::price_level(
            "t".to_string(),
            "EUR_USD".to_string(),
            "1.0900".to_string(),
            Direction::CrossUp,
            "1.0905".to_string(),
        );
        let b = QueuedAlert::price_level(
            "t".to_string(),
            "EUR_USD".to_string(),
            "1.0900".to_string(),
            Direction::CrossUp,
            "1.0905".to_string(),
        );
        assert_ne!(a.id, b.id);
    }
}

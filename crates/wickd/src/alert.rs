//! Price-level alerts — define, evaluate, and fire (AGT-617).
//!
//! The cheapest, most obviously useful kind of alert: "tell me once when
//! EUR_USD crosses 1.0900." Deliberately independent of the strategy engine
//! (`wickd_core::strategy`/`RulesEngine`) — no indicators, no candles,
//! just a price level and a direction evaluated against a stream of ticks.
//!
//! `wickd alert add|list|remove` manage alerts persisted to a JSON file at
//! `~/.wickd/alerts.json`, mirroring the home-dir + JSON-store pattern of
//! [`crate::pending`] and [`crate::vault_store`]. `wickd alert run` (see
//! [`crate::commands::alert`]) is the long-running daemon that subscribes to a
//! live OANDA price stream and drives [`evaluate`] for every stored alert.
//!
//! ## Schema (`~/.wickd/alerts.json`)
//!
//! ```json
//! { "version": 1, "alerts": [ { "id": "...", "instrument": "EUR_USD",
//!   "price": "1.0900", "direction": "cross-up", "source": "mid", "rearm": "5",
//!   "status": "armed" } ] }
//! ```
//!
//! `direction` is one of `cross-up` | `cross-down` | `either`. `source` is one
//! of `bid` | `ask` | `mid` (default `mid`). `rearm` is the hysteresis re-arm
//! band in pips (default `5`). `status` is `armed` | `fired`.
//!
//! Two extra fields (`last_price`, `fired_above`) ride along on disk beyond
//! the AC1 field list above — they are the evaluator's own bookkeeping (the
//! previous tick's price, and which side of the level the alert fired on),
//! persisted so `wickd alert run` can resume mid-sequence across a restart
//! instead of re-arming blind. They are omitted from JSON while `None` so a
//! freshly-added alert's file entry matches the schema exactly.
//!
//! ## The evaluate/fire mechanism (AC2)
//!
//! [`evaluate`] is a **pure, synchronous** function: given `&mut Alert` and one
//! [`PriceTick`], it decides whether the alert fires (returns `Some(Fired)`) or
//! re-arms, and updates `alert.status` in place. No network, no sleeping — the
//! synthetic-tick unit tests below drive it directly. The live-feed wiring
//! (`wickd alert run`, via [`crate::sink::AlertSink`]) is a thin adapter that
//! feeds real OANDA ticks through the same function and persists the result.
//!
//! An armed alert fires once it observes a *transition* across the level in
//! the configured direction — i.e. the previous tick was on one side and the
//! current tick is on/past the other. A single out-of-the-gate observation
//! (no prior tick) never fires on its own; it only seeds the baseline. Once
//! fired, the alert stops evaluating for a fresh cross until a
//! [`RearmPolicy`] says it may re-arm (AC3).
//!
//! ## The re-arm policy (AC3)
//!
//! [`RearmPolicy`] is a small trait so hysteresis is not the only re-arm
//! strategy wired into `evaluate` forever — a future cooldown-per-N-minutes or
//! pure one-shot policy just implements the trait; `evaluate`'s signature and
//! the on-disk schema do not need to change. [`HysteresisPips`] is the only
//! implementation built here: a fired alert re-arms once price pulls back
//! beyond `rearm` pips from the level, on the side opposite the one it fired
//! toward.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use wickd_core::oanda::streaming::PriceUpdate;
use wickd_core::shared::get_pip_value;

/// Whether an alert is waiting for a cross ([`Armed`](Status::Armed)) or has
/// fired and is waiting for the re-arm policy to release it
/// ([`Fired`](Status::Fired)). `Unknown` deserializes any future status value
/// the store might hold and evaluates as armed (see [`evaluate_with_policy`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    #[default]
    Armed,
    Fired,
    #[serde(other)]
    Unknown,
}

/// Default hysteresis re-arm band, in pips, for a newly added alert (AC1).
pub fn default_rearm_pips() -> Decimal {
    Decimal::new(5, 0)
}

/// Which way a cross must go to fire the alert. Moved to wickd-core with the
/// queue wire format (AGT-652); re-exported here so `crate::alert::Direction`
/// keeps resolving.
pub use wickd_core::alert_queue::Direction;

/// Which quoted price the alert watches.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Bid,
    Ask,
    Mid,
}

impl Default for Source {
    fn default() -> Self {
        Source::Mid
    }
}

impl FromStr for Source {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "bid" => Ok(Source::Bid),
            "ask" => Ok(Source::Ask),
            "mid" => Ok(Source::Mid),
            other => Err(anyhow!("unknown source '{other}' (expected bid, ask, or mid)")),
        }
    }
}

/// One price observation fed to [`evaluate`]. Deliberately decoupled from the
/// live-feed wire type ([`PriceUpdate`]) so the evaluator can be driven by a
/// synthetic sequence in tests with no network and no OANDA types involved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriceTick {
    pub bid: Decimal,
    pub ask: Decimal,
}

impl PriceTick {
    // Only the synthetic-tick unit tests construct a `PriceTick` directly —
    // production ticks arrive via `TryFrom<&PriceUpdate>` — but it is the
    // natural constructor for callers driving `evaluate` off a feed other
    // than OANDA's, so it stays part of the module's public surface.
    #[allow(dead_code)]
    pub fn new(bid: Decimal, ask: Decimal) -> Self {
        Self { bid, ask }
    }

    /// Midpoint of bid/ask.
    pub fn mid(&self) -> Decimal {
        (self.bid + self.ask) / Decimal::from(2)
    }

    /// The price this tick offers for a given [`Source`].
    pub fn price_for(&self, source: Source) -> Decimal {
        match source {
            Source::Bid => self.bid,
            Source::Ask => self.ask,
            Source::Mid => self.mid(),
        }
    }
}

impl TryFrom<&PriceUpdate> for PriceTick {
    type Error = anyhow::Error;

    fn try_from(update: &PriceUpdate) -> Result<Self> {
        let bid = Decimal::from_str(&update.bid)
            .with_context(|| format!("invalid bid price '{}'", update.bid))?;
        let ask = Decimal::from_str(&update.ask)
            .with_context(|| format!("invalid ask price '{}'", update.ask))?;
        Ok(PriceTick { bid, ask })
    }
}

/// A pluggable re-arm strategy (AC3). `evaluate`/`evaluate_with_policy` call
/// this only while an alert is `fired`, to decide whether it may return to
/// `armed`. Implementations read whatever bookkeeping they need off `alert`
/// (e.g. [`Alert::fired_above`]) — adding a new policy (cooldown-per-N-minutes,
/// pure one-shot) means adding a new implementation, not touching `evaluate`.
pub trait RearmPolicy: std::fmt::Debug {
    /// `price` is the current tick's price for the alert's configured
    /// [`Source`]. Return true to flip the alert back to `armed`.
    fn should_rearm(&self, alert: &Alert, price: Decimal) -> bool;
}

/// Default re-arm policy (AC1/AC3): a fired alert re-arms once price pulls
/// back beyond `pips` pips from the trigger level, on the side opposite the
/// one it fired toward. E.g. a `cross-up` alert at 1.0900 with 5-pip
/// hysteresis fires the moment price reaches 1.0900, then only re-arms once
/// price falls back to <= 1.0895.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HysteresisPips(pub Decimal);

impl RearmPolicy for HysteresisPips {
    fn should_rearm(&self, alert: &Alert, price: Decimal) -> bool {
        let band = get_pip_value(&alert.instrument) * self.0;
        match alert.fired_above {
            // Fired because price reached/crossed above the level: re-arm once
            // it pulls back at or below (level - band).
            Some(true) => price <= alert.price - band,
            // Fired because price reached/crossed below the level: re-arm once
            // it pulls back at or above (level + band).
            Some(false) => price >= alert.price + band,
            // A fired alert always sets fired_above; None means the store was
            // hand-edited or is from a future schema version. Stay fired
            // rather than guess.
            None => false,
        }
    }
}

/// One price-level alert (AC1 schema, plus evaluator bookkeeping — see module
/// docs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Alert {
    pub id: String,
    pub instrument: String,
    pub price: Decimal,
    pub direction: Direction,
    #[serde(default)]
    pub source: Source,
    #[serde(default = "default_rearm_pips")]
    pub rearm: Decimal,
    #[serde(default)]
    pub status: Status,
    /// Previous tick's price for this alert's source, used to detect a fresh
    /// crossing. `None` until the first tick has been observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_price: Option<Decimal>,
    /// Set when the alert fires: true if it fired because price reached/rose
    /// above the level, false if it fired because price reached/fell below
    /// it. Consumed by [`HysteresisPips`] (and any future re-arm policy that
    /// needs to know which side to watch for the pullback).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fired_above: Option<bool>,
}

impl Alert {
    /// Build a new, armed alert with a fresh id. Pure — no I/O.
    pub fn new(
        instrument: String,
        price: Decimal,
        direction: Direction,
        source: Source,
        rearm: Decimal,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            instrument,
            price,
            direction,
            source,
            rearm,
            status: Status::Armed,
            last_price: None,
            fired_above: None,
        }
    }
}

/// What [`evaluate`] returns the instant an alert fires. Deliberately minimal
/// — instrument/time context lives on the caller's tick/event, not here.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fired {
    pub alert_id: String,
    pub level: Decimal,
    pub direction: Direction,
    /// The exact price (per the alert's source) that triggered the fire.
    pub price: Decimal,
}

/// Evaluate one alert against one price tick using the default re-arm policy
/// ([`HysteresisPips`] seeded from `alert.rearm`). Mutates `alert.status` (and
/// its bookkeeping fields) in place; returns `Some(Fired)` exactly on the tick
/// that causes a fire (AC2).
pub fn evaluate(alert: &mut Alert, tick: PriceTick) -> Option<Fired> {
    let policy = HysteresisPips(alert.rearm);
    evaluate_with_policy(alert, tick, &policy)
}

/// Same as [`evaluate`] but with an explicit [`RearmPolicy`] — the seam a
/// future cooldown/one-shot policy (or a test) plugs into instead of the
/// hysteresis default.
pub fn evaluate_with_policy(alert: &mut Alert, tick: PriceTick, policy: &dyn RearmPolicy) -> Option<Fired> {
    let price = tick.price_for(alert.source);
    let level = alert.price;

    match alert.status {
        Status::Fired => {
            if policy.should_rearm(alert, price) {
                alert.status = Status::Armed;
                alert.fired_above = None;
            }
            alert.last_price = Some(price);
            return None;
        }
        // Armed behaves normally; Unknown (a future status read from disk)
        // degrades gracefully to armed rather than silently going inert.
        Status::Armed | Status::Unknown => {}
    }

    let prev = alert.last_price;
    alert.last_price = Some(price);

    let crossed = match (prev, alert.direction) {
        (None, _) => false, // first observation only seeds the baseline
        (Some(p), Direction::CrossUp) => p < level && price >= level,
        (Some(p), Direction::CrossDown) => p > level && price <= level,
        (Some(p), Direction::Either) => (p < level && price >= level) || (p > level && price <= level),
    };

    if !crossed {
        return None;
    }

    alert.status = Status::Fired;
    alert.fired_above = Some(price >= level);
    Some(Fired {
        alert_id: alert.id.clone(),
        level,
        direction: alert.direction,
        price,
    })
}

/// On-disk store. `version` allows future format migrations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AlertStore {
    pub version: u32,
    #[serde(default)]
    pub alerts: Vec<Alert>,
}

/// Path to the alert store (`~/.wickd/alerts.json`).
pub fn alerts_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not resolve home directory")?;
    Ok(home.join(".wickd").join("alerts.json"))
}

/// Load the store at `path`, or an empty one if it does not exist yet. Tests
/// pass a temp path so they never touch the real `~/.wickd/alerts.json`.
pub fn load_at(path: impl AsRef<Path>) -> Result<AlertStore> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(AlertStore { version: 1, alerts: vec![] });
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading alert store at {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| "alert store is corrupt or not valid JSON")
}

/// Write the store to `path` (creating the parent dir), pretty-printed.
pub fn save_at(path: impl AsRef<Path>, store: &AlertStore) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create alert dir {}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(store).context("could not serialize alert store")?;
    std::fs::write(path, body).with_context(|| format!("writing alert store at {}", path.display()))?;
    Ok(())
}

/// Append one alert to the store at `path` (load -> push -> save).
pub fn add_at(path: impl AsRef<Path>, alert: &Alert) -> Result<()> {
    let path = path.as_ref();
    let mut store = load_at(path)?;
    store.version = 1;
    store.alerts.push(alert.clone());
    save_at(path, &store)
}

/// List all alerts (armed + fired) from `path`, newest first.
pub fn list_at(path: impl AsRef<Path>) -> Result<Vec<Alert>> {
    let store = load_at(path)?;
    let mut alerts = store.alerts;
    alerts.reverse();
    Ok(alerts)
}

/// Remove an alert by id from `path`. Returns true if it was found and removed.
pub fn remove_at(path: impl AsRef<Path>, id: &str) -> Result<bool> {
    let path = path.as_ref();
    let mut store = load_at(path)?;
    let before = store.alerts.len();
    store.alerts.retain(|a| a.id != id);
    let removed = store.alerts.len() != before;
    if removed {
        save_at(path, &store)?;
    }
    Ok(removed)
}

/// Distinct instruments referenced by any alert (armed or fired) in the store
/// at `path`, in first-seen order. `wickd alert run` subscribes to exactly
/// this set — a fired alert still needs ticks to detect its re-arm pullback,
/// so this intentionally is not filtered to `armed` only.
pub fn instruments_at(path: impl AsRef<Path>) -> Result<Vec<String>> {
    let store = load_at(path)?;
    let mut seen = Vec::new();
    for alert in store.alerts {
        if !seen.contains(&alert.instrument) {
            seen.push(alert.instrument);
        }
    }
    Ok(seen)
}

// --- Default-path convenience wrappers (production callers) ---

/// Append one alert to the default store.
pub fn add(alert: &Alert) -> Result<()> {
    add_at(alerts_path()?, alert)
}

/// List all alerts from the default store, newest first.
pub fn list() -> Result<Vec<Alert>> {
    list_at(alerts_path()?)
}

/// Remove an alert by id from the default store.
pub fn remove(id: &str) -> Result<bool> {
    remove_at(alerts_path()?, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn armed(direction: Direction) -> Alert {
        Alert::new("EUR_USD".to_string(), dec!(1.0900), direction, Source::Mid, dec!(5))
    }

    fn tick(price: Decimal) -> PriceTick {
        // bid == ask == price keeps mid()/bid/ask all equal so tests can
        // reason about a single number regardless of `source`.
        PriceTick::new(price, price)
    }

    // AC2: a cross-up alert does not fire on the first observation (no prior
    // tick to compare against) — it only seeds the baseline.
    #[test]
    fn first_tick_never_fires() {
        let mut a = armed(Direction::CrossUp);
        assert!(evaluate(&mut a, tick(dec!(1.0850))).is_none());
        assert_eq!(a.status, Status::Armed);
        assert_eq!(a.last_price, Some(dec!(1.0850)));
    }

    // AC2: a cross-up alert fires exactly once when price crosses from below
    // the level to at/above it, and NOT again on subsequent ticks that stay
    // above the level (status flips to fired, so the crossing check stops
    // running).
    #[test]
    fn cross_up_fires_exactly_once() {
        let mut a = armed(Direction::CrossUp);
        assert!(evaluate(&mut a, tick(dec!(1.0850))).is_none()); // seed below
        let fired = evaluate(&mut a, tick(dec!(1.0905))).expect("crosses above 1.0900");
        assert_eq!(fired.direction, Direction::CrossUp);
        assert_eq!(fired.level, dec!(1.0900));
        assert_eq!(fired.price, dec!(1.0905));
        assert_eq!(a.status, Status::Fired);

        // Still above the level on the next several ticks: no re-fire.
        assert!(evaluate(&mut a, tick(dec!(1.0910))).is_none());
        assert!(evaluate(&mut a, tick(dec!(1.0920))).is_none());
        assert_eq!(a.status, Status::Fired);
    }

    // AC2: a cross-down alert fires on the opposite transition.
    #[test]
    fn cross_down_fires_on_downward_cross() {
        let mut a = armed(Direction::CrossDown);
        assert!(evaluate(&mut a, tick(dec!(1.0950))).is_none()); // seed above
        let fired = evaluate(&mut a, tick(dec!(1.0895))).expect("crosses below 1.0900");
        assert_eq!(fired.direction, Direction::CrossDown);
        assert_eq!(a.status, Status::Fired);
    }

    // A cross-up alert never fires on a downward move, and vice versa.
    #[test]
    fn wrong_direction_never_fires() {
        let mut up = armed(Direction::CrossUp);
        evaluate(&mut up, tick(dec!(1.0950)));
        assert!(evaluate(&mut up, tick(dec!(1.0850))).is_none()); // crossed DOWN, not up
        assert_eq!(up.status, Status::Armed);

        let mut down = armed(Direction::CrossDown);
        evaluate(&mut down, tick(dec!(1.0850)));
        assert!(evaluate(&mut down, tick(dec!(1.0950))).is_none()); // crossed UP, not down
        assert_eq!(down.status, Status::Armed);
    }

    // AC2: `either` fires on a crossing from either side — exercise both.
    #[test]
    fn either_fires_on_crossing_from_either_side() {
        let mut from_below = armed(Direction::Either);
        evaluate(&mut from_below, tick(dec!(1.0850)));
        let fired = evaluate(&mut from_below, tick(dec!(1.0905))).expect("either fires crossing up");
        assert_eq!(fired.direction, Direction::Either);
        assert_eq!(from_below.status, Status::Fired);

        let mut from_above = armed(Direction::Either);
        evaluate(&mut from_above, tick(dec!(1.0950)));
        let fired = evaluate(&mut from_above, tick(dec!(1.0895))).expect("either fires crossing down");
        assert_eq!(fired.direction, Direction::Either);
        assert_eq!(from_above.status, Status::Fired);
    }

    // AC3: after firing, staying within the hysteresis band does not re-arm —
    // so a small wobble right at the level can't refire the alert.
    #[test]
    fn stays_fired_within_hysteresis_band() {
        let mut a = armed(Direction::CrossUp); // 5-pip rearm, EUR_USD pip = 0.0001
        evaluate(&mut a, tick(dec!(1.0850)));
        evaluate(&mut a, tick(dec!(1.0905))).unwrap(); // fires
        assert_eq!(a.status, Status::Fired);

        // Pulls back to 1.0898 — only 0.2 pip below the level, well inside the
        // 5-pip (0.0005) band. Must not re-arm.
        assert!(evaluate(&mut a, tick(dec!(1.0898))).is_none());
        assert_eq!(a.status, Status::Fired);
    }

    // AC3/AC2: re-arm only happens after the price pulls back beyond the
    // hysteresis band, and once re-armed, a fresh cross fires again (proves
    // the alert did not just silently stay fired forever, and did not refire
    // early).
    #[test]
    fn rearm_after_hysteresis_pullback_then_fires_again() {
        let mut a = armed(Direction::CrossUp); // level 1.0900, 5-pip band = 0.0005
        evaluate(&mut a, tick(dec!(1.0850)));
        evaluate(&mut a, tick(dec!(1.0905))).unwrap(); // fires
        assert_eq!(a.status, Status::Fired);

        // Pulls back to exactly the band edge (1.0900 - 0.0005 = 1.0895): re-arms.
        assert!(evaluate(&mut a, tick(dec!(1.0895))).is_none());
        assert_eq!(a.status, Status::Armed);

        // A fresh cross above the level fires again — exactly once.
        let fired = evaluate(&mut a, tick(dec!(1.0910))).expect("re-armed alert fires on a fresh cross");
        assert_eq!(fired.price, dec!(1.0910));
        assert_eq!(a.status, Status::Fired);
        assert!(evaluate(&mut a, tick(dec!(1.0912))).is_none(), "does not fire a third time without another rearm");
    }

    // AC3: a cross-down fire's hysteresis band is on the opposite (upper)
    // side — re-arms only once price rises back above level + band.
    #[test]
    fn cross_down_rearms_on_upside_pullback() {
        let mut a = armed(Direction::CrossDown); // level 1.0900
        evaluate(&mut a, tick(dec!(1.0950)));
        evaluate(&mut a, tick(dec!(1.0895))).unwrap(); // fires
        assert_eq!(a.status, Status::Fired);

        // Still within band on the upside: no re-arm.
        assert!(evaluate(&mut a, tick(dec!(1.0902))).is_none());
        assert_eq!(a.status, Status::Fired);

        // Past 1.0900 + 0.0005 = 1.0905: re-arms.
        assert!(evaluate(&mut a, tick(dec!(1.0906))).is_none());
        assert_eq!(a.status, Status::Armed);
    }

    // AC1: source selection actually changes which price is evaluated — a mid
    // alert can cross while bid hasn't (wide spread straddling the level).
    #[test]
    fn source_selection_changes_evaluated_price() {
        let mut mid_alert = Alert::new(
            "EUR_USD".to_string(),
            dec!(1.0900),
            Direction::CrossUp,
            Source::Mid,
            dec!(5),
        );
        let mut bid_alert = Alert::new(
            "EUR_USD".to_string(),
            dec!(1.0900),
            Direction::CrossUp,
            Source::Bid,
            dec!(5),
        );

        let seed = PriceTick::new(dec!(1.0890), dec!(1.0895)); // mid 1.08925
        evaluate(&mut mid_alert, seed);
        evaluate(&mut bid_alert, seed);

        // bid 1.0899 / ask 1.0905 -> mid 1.0902 (crosses); bid itself (1.0899)
        // has not reached the level yet.
        let wide = PriceTick::new(dec!(1.0899), dec!(1.0905));
        assert!(evaluate(&mut mid_alert, wide).is_some(), "mid crossed the level");
        assert!(evaluate(&mut bid_alert, wide).is_none(), "bid has not reached the level");
    }

    // Default rearm (AC1) is 5 pips.
    #[test]
    fn new_alert_defaults_to_five_pip_rearm_and_mid_source_and_armed_status() {
        let a = Alert::new("EUR_USD".to_string(), dec!(1.0900), Direction::CrossUp, Source::Mid, default_rearm_pips());
        assert_eq!(a.rearm, dec!(5));
        assert_eq!(a.source, Source::Mid);
        assert_eq!(a.status, Status::Armed);
    }

    #[test]
    fn direction_and_source_parse_from_str() {
        assert_eq!(Direction::from_str("cross-up").unwrap(), Direction::CrossUp);
        assert_eq!(Direction::from_str("CROSS-DOWN").unwrap(), Direction::CrossDown);
        assert_eq!(Direction::from_str("either").unwrap(), Direction::Either);
        assert!(Direction::from_str("sideways").is_err());

        assert_eq!(Source::from_str("bid").unwrap(), Source::Bid);
        assert_eq!(Source::from_str("ASK").unwrap(), Source::Ask);
        assert_eq!(Source::from_str("mid").unwrap(), Source::Mid);
        assert!(Source::from_str("last").is_err());
    }

    #[test]
    fn direction_and_source_serialize_per_ac1() {
        assert_eq!(serde_json::to_string(&Direction::CrossUp).unwrap(), "\"cross-up\"");
        assert_eq!(serde_json::to_string(&Direction::CrossDown).unwrap(), "\"cross-down\"");
        assert_eq!(serde_json::to_string(&Direction::Either).unwrap(), "\"either\"");
        assert_eq!(serde_json::to_string(&Source::Bid).unwrap(), "\"bid\"");
        assert_eq!(serde_json::to_string(&Source::Ask).unwrap(), "\"ask\"");
        assert_eq!(serde_json::to_string(&Source::Mid).unwrap(), "\"mid\"");
    }

    fn temp_store() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let mut p = std::env::temp_dir();
        p.push(format!("wickd-alert-test-{pid}-{nanos}-{n}.json"));
        p
    }

    // Round-trip: add -> list (newest first) -> remove -> gone from the list.
    #[test]
    fn store_round_trip_add_list_remove() {
        let path = temp_store();

        let a = armed(Direction::CrossUp);
        let mut b = armed(Direction::CrossDown);
        b.id = "second-id".to_string();

        add_at(&path, &a).unwrap();
        add_at(&path, &b).unwrap();

        let listed = list_at(&path).unwrap();
        assert_eq!(listed.len(), 2);
        // Newest first: b was added last.
        assert_eq!(listed[0].id, "second-id");
        assert_eq!(listed[1].id, a.id);

        assert_eq!(listed[0].direction, Direction::CrossDown);

        assert!(remove_at(&path, "second-id").unwrap());
        let listed = list_at(&path).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, a.id);

        // Removing again is a no-op.
        assert!(!remove_at(&path, "second-id").unwrap());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_store_lists_empty() {
        let path = temp_store();
        assert!(list_at(&path).unwrap().is_empty());
        assert!(instruments_at(&path).unwrap().is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn instruments_at_deduplicates_across_alerts() {
        let path = temp_store();
        let mut a = armed(Direction::CrossUp);
        a.instrument = "EUR_USD".to_string();
        let mut b = armed(Direction::CrossDown);
        b.id = "b".to_string();
        b.instrument = "EUR_USD".to_string();
        let mut c = armed(Direction::Either);
        c.id = "c".to_string();
        c.instrument = "GBP_USD".to_string();

        add_at(&path, &a).unwrap();
        add_at(&path, &b).unwrap();
        add_at(&path, &c).unwrap();

        assert_eq!(instruments_at(&path).unwrap(), vec!["EUR_USD".to_string(), "GBP_USD".to_string()]);
        let _ = std::fs::remove_file(&path);
    }

    // JPY pairs use a 0.01 pip (not 0.0001) — the rearm band must scale with it.
    #[test]
    fn hysteresis_band_scales_with_instrument_pip_value() {
        let mut a = Alert::new("USD_JPY".to_string(), dec!(150.00), Direction::CrossUp, Source::Mid, dec!(5));
        evaluate(&mut a, tick(dec!(149.90)));
        evaluate(&mut a, tick(dec!(150.05))).unwrap(); // fires
        assert_eq!(a.status, Status::Fired);

        // 5 pips on JPY = 0.05: 149.96 is still inside the band.
        assert!(evaluate(&mut a, tick(dec!(149.96))).is_none());
        assert_eq!(a.status, Status::Fired);

        // 149.94 is past 150.00 - 0.05: re-arms.
        assert!(evaluate(&mut a, tick(dec!(149.94))).is_none());
        assert_eq!(a.status, Status::Armed);
    }

    #[test]
    fn price_update_conversion_parses_bid_ask() {
        let update = PriceUpdate {
            instrument: "EUR_USD".to_string(),
            bid: "1.08500".to_string(),
            ask: "1.08520".to_string(),
            spread: "0.00020".to_string(),
            time: "2024-01-15T10:30:00.000000000Z".to_string(),
            tradeable: true,
        };
        let tick = PriceTick::try_from(&update).unwrap();
        assert_eq!(tick.bid, dec!(1.08500));
        assert_eq!(tick.ask, dec!(1.08520));
    }

    #[test]
    fn price_update_conversion_rejects_unparseable_price() {
        let update = PriceUpdate {
            instrument: "EUR_USD".to_string(),
            bid: "".to_string(),
            ask: "1.08520".to_string(),
            spread: "0".to_string(),
            time: "2024-01-15T10:30:00.000000000Z".to_string(),
            tradeable: false,
        };
        assert!(PriceTick::try_from(&update).is_err());
    }
}

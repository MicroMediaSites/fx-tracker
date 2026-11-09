//! On-disk state for `wickd tui`, under the wickd data home (`~/.wickd`):
//!
//! - `tui.json` — `{"watchlist": ["SPY", …, "EUR_USD", …], "fx_seeded": true}`
//!   (the operator's symbols in order: stocks, then forex pairs)
//! - `cache/alpaca-assets.json` — the stock list for search, refreshed daily
//! - `cache/oanda-instruments.json` — the forex pairs for search, same TTL
//!
//! Single writer (the TUI process), so a tmp-file + rename is enough to keep a
//! crash mid-write from truncating the file.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use wickd_core::equities::Asset;

/// First-run watchlist: the broad-market ETFs plus a few liquid large caps
/// (the universe the research recommends starting with).
pub const DEFAULT_WATCHLIST: [&str; 7] = ["SPY", "QQQ", "IWM", "AAPL", "MSFT", "NVDA", "AMZN"];

/// How long the cached symbol list is trusted.
const ASSET_CACHE_TTL_HOURS: i64 = 24;

#[derive(Debug, Default, Serialize, Deserialize)]
struct TuiFile {
    #[serde(default)]
    watchlist: Vec<String>,
    /// The default pairs were added once; removing them all keeps them gone.
    #[serde(default)]
    fx_seeded: bool,
}

#[derive(Serialize, Deserialize)]
struct AssetCache {
    fetched_at: DateTime<Utc>,
    assets: Vec<Asset>,
}

fn home() -> Result<PathBuf> {
    wickd_core::paths::wickd_data_home().map_err(|e| anyhow!(e))
}

fn tui_path(home: &Path) -> PathBuf {
    home.join("tui.json")
}

fn assets_path(home: &Path) -> PathBuf {
    home.join("cache").join("alpaca-assets.json")
}

fn fx_assets_path(home: &Path) -> PathBuf {
    home.join("cache").join("oanda-instruments.json")
}

/// Stocks first, then forex pairs, each in their saved order (the watchlist
/// shows them as two sections).
pub fn sectioned(symbols: Vec<String>) -> Vec<String> {
    let (fx, stocks): (Vec<String>, Vec<String>) =
        symbols.into_iter().partition(|s| super::fx::is_fx(s));
    stocks.into_iter().chain(fx).collect()
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

/// Normalise a user-entered symbol (upper-case, trimmed). `BRK.B` style
/// share classes keep their dot.
pub fn normalize_symbol(s: &str) -> String {
    s.trim().to_ascii_uppercase()
}

pub fn load_watchlist_in(home: &Path) -> Result<Vec<String>> {
    let path = tui_path(home);
    let file: TuiFile = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("{} is not valid JSON", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => TuiFile {
            watchlist: DEFAULT_WATCHLIST.iter().map(|s| s.to_string()).collect(),
            fx_seeded: false,
        },
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut seen = std::collections::HashSet::new();
    let mut list: Vec<String> = file
        .watchlist
        .iter()
        .map(|s| normalize_symbol(s))
        .filter(|s| !s.is_empty() && seen.insert(s.clone()))
        .collect();
    // Existing watchlists get the default pairs once (the next save records
    // `fx_seeded`, so removing them later sticks).
    if !file.fx_seeded && !list.iter().any(|s| super::fx::is_fx(s)) {
        list.extend(super::fx::DEFAULT_PAIRS.iter().map(|s| s.to_string()));
    }
    Ok(sectioned(list))
}

pub fn save_watchlist_in(home: &Path, symbols: &[String]) -> Result<()> {
    let file = TuiFile {
        watchlist: symbols.to_vec(),
        fx_seeded: true,
    };
    write_atomic(&tui_path(home), &serde_json::to_vec_pretty(&file)?)
}

pub fn load_watchlist() -> Result<Vec<String>> {
    load_watchlist_in(&home()?)
}

pub fn save_watchlist(symbols: &[String]) -> Result<()> {
    save_watchlist_in(&home()?, symbols)
}

/// The cached asset list if it's younger than the TTL.
pub fn cached_assets_in(home: &Path, now: DateTime<Utc>) -> Option<Vec<Asset>> {
    let bytes = std::fs::read(assets_path(home)).ok()?;
    let cache: AssetCache = serde_json::from_slice(&bytes).ok()?;
    (now - cache.fetched_at < Duration::hours(ASSET_CACHE_TTL_HOURS)).then_some(cache.assets)
}

pub fn save_assets_in(home: &Path, assets: &[Asset], now: DateTime<Utc>) -> Result<()> {
    let cache = AssetCache {
        fetched_at: now,
        assets: assets.to_vec(),
    };
    write_atomic(&assets_path(home), &serde_json::to_vec(&cache)?)
}

/// The cached OANDA instrument list if it's younger than the TTL.
pub fn cached_fx_assets(now: DateTime<Utc>) -> Option<Vec<Asset>> {
    let bytes = std::fs::read(fx_assets_path(&home().ok()?)).ok()?;
    let cache: AssetCache = serde_json::from_slice(&bytes).ok()?;
    (now - cache.fetched_at < Duration::hours(ASSET_CACHE_TTL_HOURS)).then_some(cache.assets)
}

pub fn save_fx_assets(assets: &[Asset], now: DateTime<Utc>) -> Result<()> {
    let cache = AssetCache {
        fetched_at: now,
        assets: assets.to_vec(),
    };
    write_atomic(&fx_assets_path(&home()?), &serde_json::to_vec(&cache)?)
}

pub fn cached_assets(now: DateTime<Utc>) -> Option<Vec<Asset>> {
    cached_assets_in(&home().ok()?, now)
}

pub fn save_assets(assets: &[Asset], now: DateTime<Utc>) -> Result<()> {
    save_assets_in(&home()?, assets, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let wl = load_watchlist_in(dir.path()).unwrap();
        assert_eq!(wl[0], "SPY");
        // Stocks, then the default pairs.
        assert_eq!(wl.len(), DEFAULT_WATCHLIST.len() + super::super::fx::DEFAULT_PAIRS.len());
        assert_eq!(wl[DEFAULT_WATCHLIST.len()], "EUR_USD");

        save_watchlist_in(dir.path(), &["tsla".into(), "TSLA".into(), " amd ".into()]).unwrap();
        // Normalised + de-duplicated on load.
        assert_eq!(load_watchlist_in(dir.path()).unwrap(), vec!["TSLA", "AMD"]);
    }

    #[test]
    fn empty_saved_list_stays_empty() {
        let dir = tempfile::tempdir().unwrap();
        save_watchlist_in(dir.path(), &[]).unwrap();
        assert!(load_watchlist_in(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn existing_lists_get_the_default_pairs_once_and_sections_hold() {
        let dir = tempfile::tempdir().unwrap();
        // A pre-forex file: no pairs, no fx_seeded flag.
        std::fs::write(tui_path(dir.path()), r#"{"watchlist":["TSLA","AMD"]}"#).unwrap();
        let wl = load_watchlist_in(dir.path()).unwrap();
        assert_eq!(&wl[..3], ["TSLA", "AMD", "EUR_USD"]);
        // Once saved, removing every pair sticks.
        save_watchlist_in(dir.path(), &["TSLA".into()]).unwrap();
        assert_eq!(load_watchlist_in(dir.path()).unwrap(), vec!["TSLA"]);
        // Mixed order on disk loads as stocks, then pairs.
        save_watchlist_in(dir.path(), &["GBP_USD".into(), "TSLA".into(), "EUR_USD".into()])
            .unwrap();
        assert_eq!(
            load_watchlist_in(dir.path()).unwrap(),
            vec!["TSLA", "GBP_USD", "EUR_USD"]
        );
    }

    #[test]
    fn asset_cache_respects_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let now: DateTime<Utc> = "2026-11-08T12:00:00Z".parse().unwrap();
        let assets = vec![Asset {
            symbol: "SPY".into(),
            name: "SPDR".into(),
            exchange: "ARCA".into(),
            tradable: true,
        }];
        save_assets_in(dir.path(), &assets, now).unwrap();
        assert_eq!(
            cached_assets_in(dir.path(), now + Duration::hours(23)).unwrap(),
            assets
        );
        assert!(cached_assets_in(dir.path(), now + Duration::hours(25)).is_none());
    }
}

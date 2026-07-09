//! Tauri commands over the unified `.rhai` strategy store
//! (`~/.wickd/strategies/`, AGT-651).
//!
//! Read-only on purpose: the desktop app is a strategy **viewer/runner**
//! (list, inspect source, run backtests). Authoring goes through the wickd
//! CLI (`wickd strategy add/update/remove` — see `docs/strategy-store.md`),
//! so there are no write commands here and the app can never diverge from
//! the CLI's view of the store.

use serde::Serialize;
use wickd_core::strategy_store::{StoredStrategy, StrategyStore};

/// A stored strategy plus its source, for the read-only source viewer.
#[derive(Debug, Serialize)]
pub struct StoredStrategyWithSource {
    #[serde(flatten)]
    pub entry: StoredStrategy,
    pub source: String,
}

/// List every `.rhai` strategy in the store with parsed metadata. A missing
/// store directory is an empty list, never an error.
#[tauri::command]
pub fn store_list_strategies() -> Result<Vec<StoredStrategy>, String> {
    StrategyStore::open_default()?.list()
}

/// Read one stored strategy (metadata + full source) by name.
#[tauri::command]
pub fn store_read_strategy(name: String) -> Result<StoredStrategyWithSource, String> {
    let store = StrategyStore::open_default()?;
    let (entry, source) = store
        .read(&name)?
        .ok_or_else(|| format!("no stored strategy '{name}'"))?;
    Ok(StoredStrategyWithSource { entry, source })
}

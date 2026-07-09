//! Versioned migrations for the wickd local app store (`~/.wickd/app.db`).
//!
//! ╔═══════════════════════════════════════════════════════════════════════════╗
//! ║              LOCAL SQLITE MIGRATIONS LIVE HERE (AGT-642)                    ║
//! ║                                                                            ║
//! ║  This is the migration home for the desktop app's local-first store.      ║
//! ║  The old rule "all migrations in queries-service/src/migrate.ts" applies  ║
//! ║  only to the retired cloud (Zero/Postgres) path, which is frozen and      ║
//! ║  being removed. New local datasets (trades, notes, settings, ... —        ║
//! ║  AGT-645/646/647) add a new entry to `MIGRATIONS` below.                  ║
//! ╚═══════════════════════════════════════════════════════════════════════════╝
//!
//! ## How it works
//!
//! Each entry in [`MIGRATIONS`] is one schema version, applied in order inside
//! a transaction. `PRAGMA user_version` tracks the last applied version, so
//! running against an existing store applies only the tail. **Never edit or
//! reorder an existing entry** — append a new one.
//!
//! ## Conventions (match the wickd CLI stores)
//!
//! - Booleans are `INTEGER` 0/1.
//! - Timestamps are epoch **milliseconds** (`INTEGER`), matching the Zero
//!   schema's `created_at`/`updated_at` so the AGT-645 data migration is a
//!   straight copy.
//! - Rich sub-objects are JSON `TEXT`, same encoding the Zero `strategy`
//!   table used (see `shared/schema.ts`).
//! - No `user_id`: the local store is single-user by design (personal-first,
//!   see wickd-architecture.md).

use rusqlite::Connection;

/// Ordered schema migrations. `MIGRATIONS[n]` takes the store from
/// `user_version == n` to `user_version == n + 1`.
pub const MIGRATIONS: &[&str] = &[
    // v1 — strategies: the walking-skeleton dataset (AGT-642).
    // Column set mirrors the Zero `strategy` table minus `user_id`.
    "CREATE TABLE IF NOT EXISTS strategy (
        id                     TEXT PRIMARY KEY,
        name                   TEXT NOT NULL,
        description            TEXT NOT NULL DEFAULT '',
        schema_version         INTEGER,
        parameters             TEXT,
        variables              TEXT,
        indicators             TEXT NOT NULL DEFAULT '[]',
        entry_rules            TEXT NOT NULL DEFAULT '[]',
        entry_logic            TEXT,
        exit_rules             TEXT NOT NULL DEFAULT '[]',
        risk_settings          TEXT NOT NULL DEFAULT '{}',
        planning_conversation  TEXT,
        auto_note_indicators   TEXT,
        pivot_config           TEXT,
        strategy_type          TEXT,
        script_content         TEXT,
        version                INTEGER NOT NULL DEFAULT 1,
        is_active              INTEGER NOT NULL DEFAULT 1,
        is_promoted            INTEGER NOT NULL DEFAULT 0,
        is_locked              INTEGER NOT NULL DEFAULT 0,
        is_archived            INTEGER NOT NULL DEFAULT 0,
        created_at             INTEGER NOT NULL,
        updated_at             INTEGER NOT NULL
    );",
    // v2 — charting domain: S/R zones, notes, per-instrument chart config
    // (AGT-646). One self-contained block; column sets mirror the Zero
    // `sr_zone`/`note` tables minus `user_id`. Prices are TEXT (Decimal-safe,
    // never REAL). `chart_config` replaces the per-instrument localStorage
    // persistence of chart indicator configs.
    "CREATE TABLE IF NOT EXISTS sr_zone (
        id           TEXT PRIMARY KEY,
        instrument   TEXT NOT NULL,
        upper_price  TEXT NOT NULL,
        lower_price  TEXT NOT NULL,
        label        TEXT,
        color        TEXT,
        created_at   INTEGER NOT NULL,
        updated_at   INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_sr_zone_instrument ON sr_zone(instrument);
    CREATE TABLE IF NOT EXISTS note (
        id           TEXT PRIMARY KEY,
        trade_id     TEXT,
        strategy_id  TEXT,
        title        TEXT NOT NULL DEFAULT '',
        content      TEXT NOT NULL,
        created_at   INTEGER NOT NULL,
        updated_at   INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_note_trade ON note(trade_id);
    CREATE INDEX IF NOT EXISTS idx_note_strategy ON note(strategy_id);
    CREATE TABLE IF NOT EXISTS chart_config (
        instrument   TEXT PRIMARY KEY,
        indicators   TEXT NOT NULL DEFAULT '[]',
        updated_at   INTEGER NOT NULL
    );",
    // v3 — trades + AI trade scores: the trades/analysis domain (AGT-647).
    // Column sets mirror the Zero `trade` and `trade_score` tables minus
    // `user_id` (single-user store). Trade ids are raw OANDA trade ids (the
    // cloud path's `userID:oandaId` composite keys are gone with `user_id`).
    // Prices/units/P&L stay TEXT (Decimal-safe) — never REAL.
    "CREATE TABLE IF NOT EXISTS trade (
        id           TEXT PRIMARY KEY,
        account_id   TEXT,
        instrument   TEXT NOT NULL,
        units        TEXT NOT NULL,
        open_price   TEXT NOT NULL,
        close_price  TEXT,
        open_time    INTEGER NOT NULL,
        close_time   INTEGER,
        realized_pl  TEXT,
        state        TEXT NOT NULL,
        synced_at    INTEGER NOT NULL,
        created_at   INTEGER NOT NULL,
        updated_at   INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS trade_score (
        id                      TEXT PRIMARY KEY,
        trade_id                TEXT NOT NULL UNIQUE,
        score_entry             INTEGER NOT NULL,
        score_exit              INTEGER NOT NULL,
        score_risk_management   INTEGER NOT NULL,
        score_overall           INTEGER NOT NULL,
        summary                 TEXT NOT NULL DEFAULT '',
        entry_assessment        TEXT NOT NULL DEFAULT '',
        exit_assessment         TEXT NOT NULL DEFAULT '',
        indicator_analysis      TEXT NOT NULL DEFAULT '[]',
        conflicting_indicators  TEXT NOT NULL DEFAULT '[]',
        learning_points         TEXT NOT NULL DEFAULT '[]',
        created_at              INTEGER NOT NULL
    );",
    // v4 — strategies + backtests domain (AGT-645), one self-contained block.
    //
    // Mirrors the Zero `backtest`, `backtest_job` and `promotion_audit` tables
    // minus `user_id` (single-user store). `results`/`params`/`result` carry
    // JSON TEXT exactly as the Zero schema did; for `backtest.results` this is
    // the full run payload (metrics + trades + equity curve) so the backtest
    // UI renders runs/equity/trades entirely from local data.
    "CREATE TABLE IF NOT EXISTS backtest (
        id          TEXT PRIMARY KEY,
        strategy_id TEXT NOT NULL,
        instrument  TEXT NOT NULL,
        start_date  INTEGER NOT NULL,
        end_date    INTEGER NOT NULL,
        results     TEXT NOT NULL,
        created_at  INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_backtest_strategy ON backtest(strategy_id);
    CREATE TABLE IF NOT EXISTS backtest_job (
        id              TEXT PRIMARY KEY,
        strategy_id     TEXT NOT NULL,
        job_type        TEXT NOT NULL,
        status          TEXT NOT NULL,
        params          TEXT NOT NULL,
        progress        INTEGER NOT NULL DEFAULT 0,
        progress_detail TEXT,
        result          TEXT,
        error_message   TEXT,
        created_at      INTEGER NOT NULL,
        updated_at      INTEGER NOT NULL,
        completed_at    INTEGER
    );
    CREATE INDEX IF NOT EXISTS idx_backtest_job_strategy ON backtest_job(strategy_id);
    CREATE TABLE IF NOT EXISTS promotion_audit (
        id            TEXT PRIMARY KEY,
        strategy_id   TEXT NOT NULL,
        strategy_name TEXT NOT NULL,
        action        TEXT NOT NULL,
        created_at    INTEGER NOT NULL
    );",
    // v5 — provenance tagging for the CandleSight archive import (AGT-648).
    //
    // Every dataset table gains a `source` column: `''` (empty) marks native
    // wickd data; rows restored from the CandleSight archive carry
    // `'candlesight'` so imported history stays clearly distinguished and
    // filterable (app badge/filter + import CLI `--status`/`--list`) without
    // polluting new data. `chart_config` is excluded: the archive has no
    // chart-config dataset (it was localStorage, imported once by AGT-646).
    "ALTER TABLE strategy        ADD COLUMN source TEXT NOT NULL DEFAULT '';
     ALTER TABLE sr_zone         ADD COLUMN source TEXT NOT NULL DEFAULT '';
     ALTER TABLE note            ADD COLUMN source TEXT NOT NULL DEFAULT '';
     ALTER TABLE trade           ADD COLUMN source TEXT NOT NULL DEFAULT '';
     ALTER TABLE trade_score     ADD COLUMN source TEXT NOT NULL DEFAULT '';
     ALTER TABLE backtest        ADD COLUMN source TEXT NOT NULL DEFAULT '';
     ALTER TABLE backtest_job    ADD COLUMN source TEXT NOT NULL DEFAULT '';
     ALTER TABLE promotion_audit ADD COLUMN source TEXT NOT NULL DEFAULT '';",
    // v6 — Zero removal sweep (AGT-650), one self-contained block.
    // (Claimed the next free version at rebase time; trivially renumberable
    // if a concurrent ticket claims it first — position in MIGRATIONS is the
    // version, nothing below hard-codes the number.)
    //
    // The last Zero-backed domains move local so the Zero client/schema can be
    // deleted outright instead of deleting the features:
    //   - labels: `label` + the `trade_label`/`strategy_label` junctions
    //     (TradingTicketApp, LabelPicker, LabelSelector).
    //   - `strategy_trade`: OANDA-trade↔strategy attribution rows written on
    //     execution (useTradeExecution) and read by the strategy stats UI.
    //   - `strategy_watcher`: persisted watcher configs for auto-start
    //     (StrategyWatcherApp).
    //   - `credential`: the device's encrypted OANDA credential blobs, moved
    //     from the cloud `user_credentials` table (Zero) to the local store.
    //     Blobs are ciphertext (encrypt/decrypt stays in the Rust vault);
    //     account ids are plaintext, same as the cloud schema.
    // Column sets mirror the Zero tables minus `user_id` (single-user store).
    // Prices stay TEXT (Decimal-safe) — never REAL.
    "CREATE TABLE IF NOT EXISTS label (
        id          TEXT PRIMARY KEY,
        name        TEXT NOT NULL,
        color       TEXT,
        created_at  INTEGER NOT NULL,
        source      TEXT NOT NULL DEFAULT ''
    );
    CREATE TABLE IF NOT EXISTS trade_label (
        id          TEXT PRIMARY KEY,
        trade_id    TEXT NOT NULL,
        label_id    TEXT NOT NULL,
        created_at  INTEGER NOT NULL,
        source      TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX IF NOT EXISTS idx_trade_label_trade ON trade_label(trade_id);
    CREATE TABLE IF NOT EXISTS strategy_label (
        id          TEXT PRIMARY KEY,
        strategy_id TEXT NOT NULL,
        label_id    TEXT NOT NULL,
        created_at  INTEGER NOT NULL,
        source      TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX IF NOT EXISTS idx_strategy_label_strategy ON strategy_label(strategy_id);
    CREATE TABLE IF NOT EXISTS strategy_trade (
        id                  TEXT PRIMARY KEY,
        strategy_id         TEXT NOT NULL,
        strategy_config_id  TEXT,
        trade_id            TEXT NOT NULL,
        instrument          TEXT NOT NULL,
        timeframe           TEXT NOT NULL,
        direction           TEXT NOT NULL,
        entry_price         TEXT NOT NULL,
        match_time          INTEGER NOT NULL,
        executed_at         INTEGER NOT NULL,
        rules_triggered     TEXT,
        created_at          INTEGER NOT NULL,
        source              TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX IF NOT EXISTS idx_strategy_trade_strategy ON strategy_trade(strategy_id);
    CREATE TABLE IF NOT EXISTS strategy_watcher (
        id             TEXT PRIMARY KEY,
        strategy_id    TEXT NOT NULL,
        strategy_name  TEXT,
        instrument     TEXT NOT NULL,
        timeframe      TEXT NOT NULL,
        mode           TEXT NOT NULL,
        signal_filter  TEXT NOT NULL,
        is_active      INTEGER NOT NULL DEFAULT 0,
        created_at     INTEGER NOT NULL,
        updated_at     INTEGER NOT NULL,
        source         TEXT NOT NULL DEFAULT ''
    );
    CREATE TABLE IF NOT EXISTS credential (
        id                   TEXT PRIMARY KEY,
        device_id            TEXT NOT NULL,
        practice_blob        TEXT,
        practice_account_id  TEXT,
        live_blob            TEXT,
        live_account_id      TEXT,
        created_at           INTEGER NOT NULL,
        updated_at           INTEGER NOT NULL
    );",
];

/// Apply any unapplied migrations. Idempotent; safe to call on every open.
pub fn apply(conn: &mut Connection) -> rusqlite::Result<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let target = MIGRATIONS.len() as i64;
    if current >= target {
        return Ok(());
    }

    for (idx, sql) in MIGRATIONS.iter().enumerate() {
        let version = idx as i64 + 1;
        if version <= current {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
    }
    Ok(())
}

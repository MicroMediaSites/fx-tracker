//! CandleSight archive → local store import (AGT-648).
//!
//! Restores Matt's CandleSight-era data (see `docs/candlesight-archive.md`)
//! into the wickd local store, tagged with `source = 'candlesight'` so
//! imported history is clearly distinguished from — and never pollutes — new
//! wickd data.
//!
//! ## Pipeline
//!
//! 1. The CLI (`src-tauri/src/bin/import_candlesight.rs`) shells out to a
//!    PostgreSQL **16+** `pg_restore --data-only --file=-` over the archived
//!    custom-format dump, producing textual `COPY ... FROM stdin;` blocks.
//! 2. [`parse_copy_text`] parses those blocks (tab-separated fields, `\N`
//!    nulls, standard COPY text escapes) into rows keyed by **column name**,
//!    so Postgres column order never matters.
//! 3. [`LocalStore::import_candlesight_archive`] maps the rows onto the local
//!    datasets: it keeps only the selected user's rows (the store is
//!    single-user), drops `user_id`, normalizes the cloud path's
//!    `userID:oandaId` composite trade ids to raw OANDA ids (the AGT-647
//!    convention — applied to `trade.id`, `trade_score.trade_id` and
//!    `note.trade_id`), converts Postgres booleans, and writes everything in
//!    one transaction with **`INSERT OR IGNORE`**.
//!
//! `INSERT OR IGNORE` gives the two properties the import must have:
//! **idempotency** (re-running inserts nothing new) and **no clobbering**
//! (a row that already exists locally — e.g. a trade wickd has since synced
//! from OANDA — always wins over the archive copy).

use std::collections::HashMap;

use rusqlite::params;

use super::{err_str, LocalStore};

/// The provenance tag written on every imported row.
pub const CANDLESIGHT_SOURCE: &str = "candlesight";

/// The local-store dataset tables the archive can populate, in import order.
/// (`chart_config` is deliberately absent: the archive has no such dataset —
/// chart configs lived in localStorage and were imported once by AGT-646.)
pub const DATASET_TABLES: &[&str] = &[
    "strategy",
    "sr_zone",
    "note",
    "trade",
    "trade_score",
    "backtest",
    "backtest_job",
    "promotion_audit",
];

/// One parsed `COPY` block: the column list from the header plus the data
/// rows (`None` = SQL NULL).
#[derive(Debug, Default)]
pub struct CopyBlock {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}

/// Parsed dump data, keyed by (schema-stripped, unquoted) table name.
pub type ArchiveData = HashMap<String, CopyBlock>;

/// Parse `pg_restore --data-only` textual output into per-table rows.
/// Unknown tables are parsed and kept too; the importer simply ignores them.
pub fn parse_copy_text(text: &str) -> Result<ArchiveData, String> {
    let mut data = ArchiveData::new();
    let mut current: Option<(String, CopyBlock)> = None;

    for line in text.lines() {
        if let Some((table, block)) = current.as_mut() {
            if line == "\\." {
                let (table, block) = (std::mem::take(table), std::mem::take(block));
                data.insert(table, block);
                current = None;
                continue;
            }
            let ncols = block.columns.len();
            let fields: Vec<Option<String>> = line.split('\t').map(unescape_copy_field).collect();
            if fields.len() != ncols {
                return Err(format!(
                    "COPY row for `{table}` has {} fields, expected {ncols}: {line:?}",
                    fields.len()
                ));
            }
            block.rows.push(fields);
            continue;
        }

        if let Some(rest) = line.strip_prefix("COPY ") {
            let Some((name_part, cols_part)) = rest.split_once(" (") else {
                return Err(format!("unparseable COPY header: {line:?}"));
            };
            let Some(cols) = cols_part.strip_suffix(") FROM stdin;") else {
                return Err(format!("unparseable COPY header: {line:?}"));
            };
            let table = name_part
                .rsplit('.')
                .next()
                .unwrap_or(name_part)
                .trim_matches('"')
                .to_string();
            let columns = cols
                .split(',')
                .map(|c| c.trim().trim_matches('"').to_string())
                .collect();
            current = Some((table, CopyBlock { columns, rows: Vec::new() }));
        }
    }

    if let Some((table, _)) = current {
        return Err(format!("COPY block for `{table}` never terminated with \\."));
    }
    Ok(data)
}

/// Decode one COPY text-format field. `\N` is SQL NULL; otherwise standard
/// COPY escapes (`\\`, `\t`, `\n`, `\r`, `\b`, `\f`, `\v`, octal `\NNN`,
/// hex `\xNN`) are resolved.
fn unescape_copy_field(field: &str) -> Option<String> {
    if field == "\\N" {
        return None;
    }
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('b') => out.push('\u{0008}'),
            Some('f') => out.push('\u{000C}'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('v') => out.push('\u{000B}'),
            Some('x') => {
                let mut hex = String::new();
                while hex.len() < 2 && chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                    hex.push(chars.next().unwrap());
                }
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) if !hex.is_empty() => out.push(ch),
                    _ => {
                        out.push('x');
                        out.push_str(&hex);
                    }
                }
            }
            Some(d @ '0'..='7') => {
                let mut oct = String::from(d);
                while oct.len() < 3 && chars.peek().is_some_and(|c| ('0'..='7').contains(c)) {
                    oct.push(chars.next().unwrap());
                }
                if let Some(ch) = u32::from_str_radix(&oct, 8).ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    Some(out)
}

/// Per-table outcome of one import run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableReport {
    pub table: &'static str,
    /// Rows of this table present in the dump (all users).
    pub in_dump: usize,
    /// Rows belonging to the selected user.
    pub matched_user: usize,
    /// Rows newly written this run.
    pub inserted: usize,
    /// Matched rows skipped because an identical-keyed row already exists
    /// locally (previous import run, or native wickd data — which wins).
    pub skipped_existing: usize,
    /// `backtest_job` rows that were in-flight (running/pending) at archive
    /// time and were imported as `cancelled`.
    pub adjusted_inflight: usize,
}

/// Outcome of one import run, per dataset table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub tables: Vec<TableReport>,
}

impl ImportReport {
    pub fn total_inserted(&self) -> usize {
        self.tables.iter().map(|t| t.inserted).sum()
    }
}

/// `id`, stripped of a leading `"{user_id}:"` — the cloud path's composite
/// trade-id form. Raw ids pass through unchanged (AGT-647 convention).
fn strip_user_prefix(id: &str, user_id: &str) -> String {
    match id.strip_prefix(user_id).and_then(|rest| rest.strip_prefix(':')) {
        Some(raw) if !raw.is_empty() => raw.to_string(),
        _ => id.to_string(),
    }
}

/// Postgres COPY boolean (`t`/`f`) → SQLite 0/1.
fn pg_bool(v: Option<&str>) -> i64 {
    i64::from(v == Some("t"))
}

fn pg_i64(v: Option<&str>) -> Result<i64, String> {
    let s = v.ok_or("unexpected NULL integer")?;
    s.parse::<i64>().map_err(|e| format!("bad integer {s:?}: {e}"))
}

fn opt_i64(v: Option<&str>) -> Result<Option<i64>, String> {
    v.map(|s| s.parse::<i64>().map_err(|e| format!("bad integer {s:?}: {e}")))
        .transpose()
}

/// Column-name-addressed view over one COPY row.
struct RowView<'a> {
    index: &'a HashMap<&'a str, usize>,
    row: &'a [Option<String>],
    table: &'static str,
}

impl<'a> RowView<'a> {
    /// The field, or `None` when the column is absent or SQL NULL.
    fn opt(&self, col: &str) -> Option<&'a str> {
        self.index
            .get(col)
            .and_then(|&i| self.row.get(i))
            .and_then(|v| v.as_deref())
    }

    /// The field as text; errors on NULL/absent (for NOT NULL columns).
    fn req(&self, col: &str) -> Result<&'a str, String> {
        self.opt(col)
            .ok_or_else(|| format!("{}: unexpected NULL in column `{col}`", self.table))
    }

    /// The field as text, with a default for NULL/absent.
    fn text_or(&self, col: &str, default: &str) -> String {
        self.opt(col).unwrap_or(default).to_string()
    }
}

impl LocalStore {
    /// Import the parsed archive rows belonging to `user_id` into this store,
    /// tagged `source = 'candlesight'`, in one transaction. With `dry_run` the
    /// transaction is rolled back after computing the full report.
    pub fn import_candlesight_archive(
        &mut self,
        data: &ArchiveData,
        user_id: &str,
        dry_run: bool,
    ) -> Result<ImportReport, String> {
        let tx = self.conn.transaction().map_err(err_str)?;
        let mut tables = Vec::with_capacity(DATASET_TABLES.len());

        for &table in DATASET_TABLES {
            let empty = CopyBlock::default();
            let block = data.get(table).unwrap_or(&empty);
            let index: HashMap<&str, usize> = block
                .columns
                .iter()
                .enumerate()
                .map(|(i, c)| (c.as_str(), i))
                .collect();

            let mut report = TableReport {
                table,
                in_dump: block.rows.len(),
                matched_user: 0,
                inserted: 0,
                skipped_existing: 0,
                adjusted_inflight: 0,
            };

            for row in &block.rows {
                let view = RowView { index: &index, row, table };
                if view.opt("user_id") != Some(user_id) {
                    continue;
                }
                report.matched_user += 1;
                let n = insert_row(&tx, table, &view, user_id, &mut report)?;
                if n == 0 {
                    report.skipped_existing += 1;
                } else {
                    report.inserted += 1;
                }
            }
            tables.push(report);
        }

        if dry_run {
            tx.rollback().map_err(err_str)?;
        } else {
            tx.commit().map_err(err_str)?;
        }
        Ok(ImportReport { tables })
    }

    /// Row count per dataset table carrying the given `source` tag
    /// (import CLI `--status`).
    pub fn count_by_source(&self, source: &str) -> Result<Vec<(&'static str, i64)>, String> {
        DATASET_TABLES
            .iter()
            .map(|&table| {
                // `table` is a compile-time constant — not user input.
                let n: i64 = self
                    .conn
                    .query_row(
                        &format!("SELECT COUNT(*) FROM {table} WHERE source = ?1"),
                        params![source],
                        |r| r.get(0),
                    )
                    .map_err(err_str)?;
                Ok((table, n))
            })
            .collect()
    }
}

/// `INSERT OR IGNORE` one archive row into `table`. Returns rows written
/// (0 = an equally-keyed local row already exists and wins).
fn insert_row(
    tx: &rusqlite::Transaction<'_>,
    table: &'static str,
    v: &RowView<'_>,
    user_id: &str,
    report: &mut TableReport,
) -> Result<usize, String> {
    let src = CANDLESIGHT_SOURCE;
    let n = match table {
        "strategy" => tx.execute(
            "INSERT OR IGNORE INTO strategy (
                id, name, description, schema_version, parameters, variables,
                indicators, entry_rules, entry_logic, exit_rules, risk_settings,
                planning_conversation, auto_note_indicators, pivot_config,
                strategy_type, script_content, version,
                is_active, is_promoted, is_locked, is_archived,
                created_at, updated_at, source
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)",
            params![
                v.req("id")?,
                v.req("name")?,
                v.text_or("description", ""),
                opt_i64(v.opt("schema_version"))?,
                v.opt("parameters"),
                v.opt("variables"),
                v.text_or("indicators", "[]"),
                v.text_or("entry_rules", "[]"),
                v.opt("entry_logic"),
                v.text_or("exit_rules", "[]"),
                v.text_or("risk_settings", "{}"),
                v.opt("planning_conversation"),
                v.opt("auto_note_indicators"),
                v.opt("pivot_config"),
                v.opt("strategy_type"),
                v.opt("script_content"),
                opt_i64(v.opt("version"))?.unwrap_or(1),
                pg_bool(v.opt("is_active")),
                pg_bool(v.opt("is_promoted")),
                pg_bool(v.opt("is_locked")),
                pg_bool(v.opt("is_archived")),
                pg_i64(v.opt("created_at"))?,
                pg_i64(v.opt("updated_at"))?,
                src,
            ],
        ),
        "sr_zone" => tx.execute(
            "INSERT OR IGNORE INTO sr_zone (
                id, instrument, upper_price, lower_price, label, color,
                created_at, updated_at, source
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                v.req("id")?,
                v.req("instrument")?,
                v.req("upper_price")?,
                v.req("lower_price")?,
                v.opt("label"),
                v.opt("color"),
                pg_i64(v.opt("created_at"))?,
                pg_i64(v.opt("updated_at"))?,
                src,
            ],
        ),
        "note" => tx.execute(
            "INSERT OR IGNORE INTO note (
                id, trade_id, strategy_id, title, content, created_at, updated_at, source
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                v.req("id")?,
                v.opt("trade_id").map(|id| strip_user_prefix(id, user_id)),
                v.opt("strategy_id"),
                v.text_or("title", ""),
                v.text_or("content", ""),
                pg_i64(v.opt("created_at"))?,
                pg_i64(v.opt("updated_at"))?,
                src,
            ],
        ),
        "trade" => tx.execute(
            "INSERT OR IGNORE INTO trade (
                id, account_id, instrument, units, open_price, close_price,
                open_time, close_time, realized_pl, state,
                synced_at, created_at, updated_at, source
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                strip_user_prefix(v.req("id")?, user_id),
                v.opt("account_id"),
                v.req("instrument")?,
                v.req("units")?,
                v.req("open_price")?,
                v.opt("close_price"),
                pg_i64(v.opt("open_time"))?,
                opt_i64(v.opt("close_time"))?,
                v.opt("realized_pl"),
                v.req("state")?,
                pg_i64(v.opt("synced_at"))?,
                pg_i64(v.opt("created_at"))?,
                pg_i64(v.opt("updated_at"))?,
                src,
            ],
        ),
        "trade_score" => tx.execute(
            "INSERT OR IGNORE INTO trade_score (
                id, trade_id, score_entry, score_exit, score_risk_management,
                score_overall, summary, entry_assessment, exit_assessment,
                indicator_analysis, conflicting_indicators, learning_points,
                created_at, source
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                v.req("id")?,
                strip_user_prefix(v.req("trade_id")?, user_id),
                pg_i64(v.opt("score_entry"))?,
                pg_i64(v.opt("score_exit"))?,
                pg_i64(v.opt("score_risk_management"))?,
                pg_i64(v.opt("score_overall"))?,
                v.text_or("summary", ""),
                v.text_or("entry_assessment", ""),
                v.text_or("exit_assessment", ""),
                v.text_or("indicator_analysis", "[]"),
                v.text_or("conflicting_indicators", "[]"),
                v.text_or("learning_points", "[]"),
                pg_i64(v.opt("created_at"))?,
                src,
            ],
        ),
        "backtest" => tx.execute(
            "INSERT OR IGNORE INTO backtest (
                id, strategy_id, instrument, start_date, end_date, results, created_at, source
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                v.req("id")?,
                v.req("strategy_id")?,
                v.req("instrument")?,
                pg_i64(v.opt("start_date"))?,
                pg_i64(v.opt("end_date"))?,
                v.req("results")?,
                pg_i64(v.opt("created_at"))?,
                src,
            ],
        ),
        "backtest_job" => {
            // A job that was in-flight when prod was archived can never
            // complete — import it as cancelled so the UI shows no zombie.
            let status = v.req("status")?;
            let inflight = matches!(status, "running" | "pending");
            let (status, error_message) = if inflight {
                report.adjusted_inflight += 1;
                (
                    "cancelled",
                    Some(
                        "Imported from the CandleSight archive; the job was still \
                         in-flight when production was archived."
                            .to_string(),
                    ),
                )
            } else {
                (status, v.opt("error_message").map(str::to_string))
            };
            tx.execute(
                "INSERT OR IGNORE INTO backtest_job (
                    id, strategy_id, job_type, status, params, progress,
                    progress_detail, result, error_message,
                    created_at, updated_at, completed_at, source
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![
                    v.req("id")?,
                    v.req("strategy_id")?,
                    v.req("job_type")?,
                    status,
                    v.text_or("params", "{}"),
                    opt_i64(v.opt("progress"))?.unwrap_or(0),
                    v.opt("progress_detail"),
                    v.opt("result"),
                    error_message,
                    pg_i64(v.opt("created_at"))?,
                    pg_i64(v.opt("updated_at"))?,
                    opt_i64(v.opt("completed_at"))?,
                    src,
                ],
            )
        }
        "promotion_audit" => tx.execute(
            "INSERT OR IGNORE INTO promotion_audit (
                id, strategy_id, strategy_name, action, created_at, source
             ) VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                v.req("id")?,
                v.req("strategy_id")?,
                v.text_or("strategy_name", ""),
                v.req("action")?,
                pg_i64(v.opt("created_at"))?,
                src,
            ],
        ),
        other => return Err(format!("unknown dataset table `{other}`")),
    };
    n.map_err(|e| format!("{table}: insert failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const USER: &str = "user_primary";

    fn temp_store() -> (LocalStore, PathBuf) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "wickd-import-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let store = LocalStore::open_at(&dir.join("app.db")).expect("open temp store");
        (store, dir)
    }

    /// A small but complete pg_restore-style fixture: every dataset table,
    /// two users, composite trade ids, an in-flight job, NULLs and escapes.
    fn fixture() -> String {
        [
            "--",
            "-- Data for Name: strategy; Type: TABLE DATA",
            "--",
            "",
            "COPY public.strategy (id, user_id, name, description, indicators, entry_rules, entry_logic, exit_rules, risk_settings, version, is_active, created_at, updated_at, planning_conversation, is_promoted, auto_note_indicators, is_locked, is_archived, pivot_config, parameters, schema_version, variables, strategy_type, script_content) FROM stdin;",
            "strat-1\tuser_primary\tIchimoku breakout\tLine1\\nLine2\t[]\t[]\t\\N\t[]\t{}\t2\tt\t1000\t2000\t\\N\tt\t\\N\tf\tf\t\\N\t\\N\t2\t\\N\trules\t\\N",
            "strat-2\tuser_primary\tArchived one\t\t[]\t[]\t\\N\t[]\t{}\t1\tf\t1000\t3000\t\\N\tf\t\\N\tf\tt\t\\N\t\\N\t\\N\t\\N\t\\N\t\\N",
            "strat-other\tuser_other\tNot mine\t\t[]\t[]\t\\N\t[]\t{}\t1\tt\t1000\t1000\t\\N\tf\t\\N\tf\tf\t\\N\t\\N\t\\N\t\\N\t\\N\t\\N",
            "\\.",
            "",
            "COPY public.sr_zone (id, user_id, instrument, upper_price, lower_price, label, color, created_at, updated_at) FROM stdin;",
            "z-1\tuser_primary\tEUR_GBP\t0.86767\t0.86757\tDaily R3\trgba(239, 68, 68, 0.20)\t1000\t1000",
            "\\.",
            "",
            "COPY public.note (id, user_id, trade_id, title, content, created_at, updated_at, strategy_id) FROM stdin;",
            "n-1\tuser_primary\tuser_primary:14\t\tTook the retest\t1000\t1000\t\\N",
            "\\.",
            "",
            "COPY public.trade (id, user_id, instrument, units, open_price, close_price, open_time, close_time, realized_pl, state, synced_at, created_at, updated_at, account_id) FROM stdin;",
            "user_primary:14\tuser_primary\tEUR_USD\t10000\t1.08500\t1.08750\t1000\t2000\t25.00\tCLOSED\t3000\t3000\t3000\tacct-1",
            "171\tuser_primary\tGBP_USD\t-5000\t1.26000\t\\N\t4000\t\\N\t\\N\tOPEN\t5000\t5000\t5000\tacct-1",
            "user_other:14\tuser_other\tEUR_USD\t1\t1.0\t\\N\t1\t\\N\t\\N\tOPEN\t1\t1\t1\t\\N",
            "\\.",
            "",
            "COPY public.trade_score (id, user_id, trade_id, score_entry, score_exit, score_risk_management, score_overall, summary, entry_assessment, exit_assessment, indicator_analysis, conflicting_indicators, learning_points, created_at) FROM stdin;",
            "ts-1\tuser_primary\tuser_primary:14\t7\t6\t8\t7\tSolid\tGood\tEarly\t[]\t[]\t[]\t1000",
            "\\.",
            "",
            "COPY public.backtest (id, user_id, strategy_id, instrument, start_date, end_date, results, created_at) FROM stdin;",
            "\\.",
            "",
            "COPY public.backtest_job (id, user_id, strategy_id, job_type, status, params, progress, progress_detail, result, error_message, created_at, updated_at, completed_at) FROM stdin;",
            "j-1\tuser_primary\tstrat-1\twalk_forward\tcompleted\t{}\t100\t\\N\t{\"ok\":true}\t\\N\t1000\t2000\t2000",
            "j-2\tuser_primary\tstrat-1\tsimple_backtest\trunning\t{}\t40\t\\N\t\\N\t\\N\t1000\t2000\t\\N",
            "\\.",
            "",
            "COPY public.promotion_audit (id, user_id, strategy_id, strategy_name, action, created_at) FROM stdin;",
            "p-1\tuser_primary\tstrat-1\tIchimoku breakout\tpromote\t1000",
            "\\.",
            "",
        ]
        .join("\n")
    }

    #[test]
    fn parses_copy_blocks_nulls_and_escapes() {
        let data = parse_copy_text(&fixture()).unwrap();
        assert_eq!(data["strategy"].rows.len(), 3);
        assert_eq!(data["backtest"].rows.len(), 0, "empty table parses as zero rows");
        // \N is NULL, \n unescapes to a newline.
        let strat1 = &data["strategy"].rows[0];
        assert_eq!(strat1[3].as_deref(), Some("Line1\nLine2"));
        assert_eq!(strat1[6], None, "entry_logic is NULL");
    }

    #[test]
    fn unescapes_octal_hex_and_backslash() {
        assert_eq!(unescape_copy_field("a\\\\b").as_deref(), Some("a\\b"));
        assert_eq!(unescape_copy_field("\\x41\\101").as_deref(), Some("AA"));
        assert_eq!(unescape_copy_field("tab\\there").as_deref(), Some("tab\there"));
        assert_eq!(unescape_copy_field("\\N"), None);
    }

    #[test]
    fn unterminated_copy_block_is_an_error() {
        let text = "COPY public.strategy (id, user_id) FROM stdin;\ns-1\tu-1\n";
        assert!(parse_copy_text(text).is_err());
    }

    #[test]
    fn strip_user_prefix_only_strips_matching_composites() {
        assert_eq!(strip_user_prefix("user_primary:208", "user_primary"), "208");
        assert_eq!(strip_user_prefix("171", "user_primary"), "171");
        assert_eq!(strip_user_prefix("user_other:208", "user_primary"), "user_other:208");
    }

    #[test]
    fn imports_only_selected_user_normalizes_ids_and_tags_source() {
        let (mut store, dir) = temp_store();
        let data = parse_copy_text(&fixture()).unwrap();
        let report = store.import_candlesight_archive(&data, USER, false).unwrap();

        let by_table: HashMap<_, _> = report.tables.iter().map(|t| (t.table, t)).collect();
        assert_eq!(by_table["strategy"].in_dump, 3);
        assert_eq!(by_table["strategy"].matched_user, 2);
        assert_eq!(by_table["strategy"].inserted, 2);
        assert_eq!(by_table["trade"].inserted, 2, "other user's trade excluded");
        assert_eq!(by_table["backtest"].inserted, 0);
        assert_eq!(by_table["backtest_job"].adjusted_inflight, 1);

        // Strategies land tagged and fully shaped.
        let strategies = store.list_strategies().unwrap();
        assert_eq!(strategies.len(), 2);
        assert!(strategies.iter().all(|s| s.source == CANDLESIGHT_SOURCE));
        let s1 = store.get_strategy("strat-1").unwrap().unwrap();
        assert_eq!(s1.description, "Line1\nLine2");
        assert!(s1.is_promoted && s1.is_active && !s1.is_archived);
        assert_eq!(s1.schema_version, Some(2));

        // Composite ids are normalized everywhere they appear.
        let trades = store.list_trades().unwrap();
        let mut ids: Vec<_> = trades.iter().map(|t| t.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["14", "171"]);
        assert_eq!(
            store.get_trade_score_by_trade("14").unwrap().unwrap().trade_id,
            "14"
        );
        assert_eq!(
            store.list_notes(Some("14"), None).unwrap().len(),
            1,
            "note.trade_id normalized to the raw OANDA id"
        );

        // The in-flight job was imported as cancelled, with an explanation.
        let j2 = store.get_backtest_job("j-2").unwrap().unwrap();
        assert_eq!(j2.status, "cancelled");
        assert!(j2.error_message.unwrap().contains("archive"));
        // The completed job kept its status.
        assert_eq!(store.get_backtest_job("j-1").unwrap().unwrap().status, "completed");

        // --status counts match what was written.
        let counts: HashMap<_, _> = store
            .count_by_source(CANDLESIGHT_SOURCE)
            .unwrap()
            .into_iter()
            .collect();
        assert_eq!(counts["strategy"], 2);
        assert_eq!(counts["trade"], 2);
        assert_eq!(counts["promotion_audit"], 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn reimport_is_idempotent_and_never_overwrites_local_rows() {
        let (mut store, dir) = temp_store();

        // A native wickd strategy that shares an id with an archive row: the
        // local row must win and stay untagged.
        let native = crate::local_store::LocalStrategy {
            id: "strat-1".to_string(),
            name: "Native wins".to_string(),
            description: String::new(),
            schema_version: Some(2),
            parameters: None,
            variables: None,
            indicators: "[]".to_string(),
            entry_rules: "[]".to_string(),
            entry_logic: None,
            exit_rules: "[]".to_string(),
            risk_settings: "{}".to_string(),
            planning_conversation: None,
            auto_note_indicators: None,
            pivot_config: None,
            strategy_type: Some("rules".to_string()),
            script_content: None,
            version: 1,
            is_active: true,
            is_promoted: false,
            is_locked: false,
            is_archived: false,
            created_at: 9_000,
            updated_at: 9_000,
            source: String::new(),
        };
        store.save_strategy(&native).unwrap();

        let data = parse_copy_text(&fixture()).unwrap();
        let first = store.import_candlesight_archive(&data, USER, false).unwrap();
        let strategy = first.tables.iter().find(|t| t.table == "strategy").unwrap();
        assert_eq!(strategy.inserted, 1);
        assert_eq!(strategy.skipped_existing, 1);

        let kept = store.get_strategy("strat-1").unwrap().unwrap();
        assert_eq!(kept.name, "Native wins");
        assert_eq!(kept.source, "");

        // Second run: nothing new anywhere.
        let second = store.import_candlesight_archive(&data, USER, false).unwrap();
        assert_eq!(second.total_inserted(), 0);
        assert!(second
            .tables
            .iter()
            .all(|t| t.skipped_existing == t.matched_user));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn dry_run_reports_but_writes_nothing() {
        let (mut store, dir) = temp_store();
        let data = parse_copy_text(&fixture()).unwrap();
        let report = store.import_candlesight_archive(&data, USER, true).unwrap();
        assert!(report.total_inserted() > 0, "dry run still reports the plan");
        assert!(store.list_strategies().unwrap().is_empty());
        assert!(store.list_trades().unwrap().is_empty());
        std::fs::remove_dir_all(dir).ok();
    }
}

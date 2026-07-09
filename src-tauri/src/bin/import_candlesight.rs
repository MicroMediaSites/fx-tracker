//! `import_candlesight` — restore the CandleSight archive into the wickd
//! local store (`~/.wickd/app.db`), tagged `source = 'candlesight'` (AGT-648).
//!
//! Run by hand, not by the app:
//!
//! ```sh
//! cd src-tauri
//! cargo run --bin import_candlesight -- --dry-run   # preview
//! cargo run --bin import_candlesight                # import
//! cargo run --bin import_candlesight -- --status    # imported row counts
//! cargo run --bin import_candlesight -- --list      # imported strategies
//! ```
//!
//! Requires a PostgreSQL **16+** `pg_restore` (the dump is pg_dump v17 custom
//! format; pg 15's pg_restore fails with `unsupported version (1.16)`). The
//! tool auto-detects the Homebrew postgresql@17/@16 binaries and falls back
//! to `pg_restore` on PATH; override with `--pg-restore`.
//!
//! See `docs/candlesight-archive.md` ("Importing into the wickd local store")
//! for the full procedure and semantics.

use std::path::PathBuf;
use std::process::Command;

use clap::Parser;

use candlesight_lib::local_store::import::{
    parse_copy_text, CANDLESIGHT_SOURCE, DATASET_TABLES,
};
use candlesight_lib::local_store::LocalStore;

/// Candidate pg_restore locations, tried in order when --pg-restore is unset.
const PG_RESTORE_CANDIDATES: &[&str] = &[
    "/opt/homebrew/opt/postgresql@17/bin/pg_restore",
    "/opt/homebrew/opt/postgresql@16/bin/pg_restore",
    "pg_restore",
];

#[derive(Parser, Debug)]
#[command(
    name = "import_candlesight",
    about = "Import the CandleSight archive dump into the wickd local store, tagged 'candlesight'."
)]
struct Args {
    /// Path to the archived pg_dump custom-format file.
    #[arg(long, default_value_os_t = default_dump_path())]
    dump: PathBuf,

    /// Local store to import into (defaults to ~/.wickd/app.db).
    #[arg(long)]
    db: Option<PathBuf>,

    /// CandleSight user (Clerk id) whose rows are imported (the store is
    /// single-user). Required — no built-in default, so no personal id is
    /// baked into the binary.
    #[arg(long)]
    user: String,

    /// Explicit pg_restore binary (must be PostgreSQL 16+).
    #[arg(long)]
    pg_restore: Option<PathBuf>,

    /// Parse, map and report — but write nothing.
    #[arg(long)]
    dry_run: bool,

    /// Show per-dataset counts of already-imported rows, then exit.
    #[arg(long)]
    status: bool,

    /// List imported strategies, then exit.
    #[arg(long)]
    list: bool,
}

fn default_dump_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join("Documents/candlesight-archive/candlesight-prod-2026-07-06.dump")
}

fn open_store(db: &Option<PathBuf>) -> Result<LocalStore, String> {
    match db {
        Some(path) => LocalStore::open_at(path),
        None => LocalStore::open_default(),
    }
}

/// Extract textual COPY data for the dataset tables from the custom-format
/// dump via `pg_restore --data-only --file=-`.
fn extract_copy_text(args: &Args) -> Result<String, String> {
    let candidates: Vec<PathBuf> = match &args.pg_restore {
        Some(p) => vec![p.clone()],
        None => PG_RESTORE_CANDIDATES.iter().map(PathBuf::from).collect(),
    };

    let mut last_err = String::new();
    for bin in &candidates {
        let mut cmd = Command::new(bin);
        cmd.arg("--data-only").arg("--file=-");
        for table in DATASET_TABLES {
            cmd.arg("-t").arg(table);
        }
        cmd.arg(&args.dump);

        match cmd.output() {
            Ok(out) if out.status.success() => {
                return String::from_utf8(out.stdout)
                    .map_err(|e| format!("pg_restore output was not UTF-8: {e}"));
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                last_err = format!("{} failed: {}", bin.display(), stderr.trim());
                if stderr.contains("unsupported version") {
                    last_err.push_str(
                        "\nhint: this pg_restore is too old for a pg_dump v17 archive — \
                         use PostgreSQL 16+ (e.g. `brew install postgresql@17`).",
                    );
                }
            }
            Err(e) => last_err = format!("could not run {}: {e}", bin.display()),
        }
    }
    Err(format!(
        "no usable pg_restore found (tried {}).\n{last_err}",
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn print_status(store: &LocalStore) -> Result<(), String> {
    println!("Rows tagged source='{CANDLESIGHT_SOURCE}' in the local store:");
    println!("{:<18} {:>8}", "dataset", "rows");
    let mut total = 0;
    for (table, n) in store.count_by_source(CANDLESIGHT_SOURCE)? {
        println!("{table:<18} {n:>8}");
        total += n;
    }
    println!("{:<18} {total:>8}", "total");
    Ok(())
}

fn print_list(store: &LocalStore) -> Result<(), String> {
    let imported: Vec<_> = store
        .list_strategies()?
        .into_iter()
        .filter(|s| s.source == CANDLESIGHT_SOURCE)
        .collect();
    println!("Imported CandleSight strategies: {}", imported.len());
    for s in imported {
        let mut flags = Vec::new();
        if s.is_archived {
            flags.push("archived");
        }
        if !s.is_active {
            flags.push("inactive");
        }
        if s.is_promoted {
            flags.push("promoted");
        }
        let flags = if flags.is_empty() {
            String::new()
        } else {
            format!("  [{}]", flags.join(", "))
        };
        println!("  {}  {}{flags}", s.id, s.name);
    }
    Ok(())
}

fn run(args: Args) -> Result<(), String> {
    if args.status || args.list {
        let store = open_store(&args.db)?;
        if args.status {
            print_status(&store)?;
        }
        if args.list {
            print_list(&store)?;
        }
        return Ok(());
    }

    if !args.dump.is_file() {
        return Err(format!("archive dump not found at {}", args.dump.display()));
    }

    println!("Reading archive dump {} …", args.dump.display());
    let copy_text = extract_copy_text(&args)?;
    let data = parse_copy_text(&copy_text)?;

    let mut store = open_store(&args.db)?;
    let report = store.import_candlesight_archive(&data, &args.user, args.dry_run)?;

    let mode = if args.dry_run { " (dry run — nothing written)" } else { "" };
    println!(
        "\nImport of user {} tagged '{CANDLESIGHT_SOURCE}'{mode}:",
        args.user
    );
    println!(
        "{:<18} {:>8} {:>8} {:>9} {:>9}",
        "dataset", "in-dump", "matched", "inserted", "skipped"
    );
    for t in &report.tables {
        let note = if t.adjusted_inflight > 0 {
            format!("  ({} in-flight job(s) imported as cancelled)", t.adjusted_inflight)
        } else {
            String::new()
        };
        println!(
            "{:<18} {:>8} {:>8} {:>9} {:>9}{note}",
            t.table, t.in_dump, t.matched_user, t.inserted, t.skipped_existing
        );
    }
    println!(
        "\n{} row(s) {}. Re-running is safe: existing rows are never overwritten.",
        report.total_inserted(),
        if args.dry_run { "would be inserted" } else { "inserted" }
    );
    Ok(())
}

fn main() {
    let args = Args::parse();
    if let Err(e) = run(args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

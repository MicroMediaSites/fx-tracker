//! `wickd audit` — read the append-only execution audit log.
//!
//!   wickd audit                 # most recent 50 decisions, newest first
//!   wickd audit --limit 200
//!
//! Every execution decision (paper or live, place or close) is recorded to the
//! local SQLite store at `~/.wickd/audit.db` (see [`crate::audit`]). This verb
//! is the read path: it emits the recent rows as a single JSON object, like the
//! rest of the CLI. The log is append-only by construction — there is no
//! `wickd audit` flag (or any other code path) that mutates or deletes rows.

use clap::Args;

use crate::audit;
use crate::output::{exit, Out};

#[derive(Args, Debug)]
pub struct AuditArgs {
    /// Maximum number of recent rows to return (newest first).
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
}

pub async fn run(args: AuditArgs, out: Out) -> ! {
    let conn = match audit::open() {
        Ok(c) => c,
        Err(e) => out.fail(exit::GENERIC, "audit_open_failed", format!("{e:#}")),
    };
    match audit::query(&conn, args.limit) {
        Ok(rows) => {
            out.ok(&serde_json::json!({ "count": rows.len(), "entries": rows }));
            std::process::exit(exit::OK);
        }
        Err(e) => out.fail(exit::GENERIC, "audit_query_failed", format!("{e:#}")),
    }
}

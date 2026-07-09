//! `wickd pending` — list signals awaiting approval (AGT-599, Stage 1).
//!
//!   wickd pending
//!
//! Surfaces the pending-signal proposals that `wickd watch --semi-auto` recorded
//! to `~/.wickd/pending.json`, newest first, so an agent or human can decide
//! what to approve. Read-only: listing never executes or mutates anything.
//!
//! Approve one with `wickd approve <id>` (paper by default, `--live` to arm).

use anyhow::Result;
use clap::Args;

use crate::output::{exit, Out};
use crate::pending;

#[derive(Args, Debug)]
pub struct PendingArgs {
    /// Cap the number of (newest-first) pending signals returned.
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
}

pub async fn run(args: PendingArgs, out: Out) -> ! {
    match list(args) {
        Ok(v) => {
            out.ok(&v);
            std::process::exit(exit::OK);
        }
        Err(e) => out.fail(exit::GENERIC, "pending_failed", format!("{e:#}")),
    }
}

fn list(args: PendingArgs) -> Result<serde_json::Value> {
    let mut signals = pending::list()?;
    signals.truncate(args.limit);
    Ok(serde_json::json!({
        "count": signals.len(),
        "pending": signals,
    }))
}

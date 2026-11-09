//! `wickd list` — named symbol lists, the agent-facing way to build a day's
//! plan (wickd-lists AGT-2391; four plan fields per symbol, AGT-2404). See
//! [`crate::lists`] for the store.
//!
//!   wickd list ls
//!   wickd list show mon-11-9
//!   wickd list add mon-11-9 NET --why "earnings +39%" --constraints "stop below Fri low" \
//!       --signal "holds Fri close after 8:00 on volume"
//!   wickd list set mon-11-9 NET --call "continues"
//!   wickd list rm mon-11-9 NET
//!   wickd list clear mon-11-9
//!   wickd list delete mon-11-9
//!
//! One JSON object per call (see `crate::output`). Symbols are upper-cased
//! (`eur/usd` → `EUR_USD`) and checked against the cached Alpaca and OANDA
//! symbol lists (`wickd tui` fills those caches); with no cache yet they're
//! accepted and reported as `unverified`. `add`/`set`/`rm` answer with what
//! changed, not the whole list (`show` is for that).

use anyhow::Result;
use chrono::Utc;
use clap::{Args, Subcommand};
use serde_json::json;

use crate::lists::{self, Plan};
use crate::output::{exit, Out};

#[derive(Args, Debug)]
pub struct ListArgs {
    #[command(subcommand)]
    cmd: ListCmd,
}

/// The four plan fields (all optional; an empty value clears a field).
#[derive(Args, Debug, Default)]
struct PlanArgs {
    /// Why it's on the list. (`--note` is the old name.)
    #[arg(long, alias = "note")]
    why: Option<String>,
    /// Your call: what you expect it to do.
    #[arg(long)]
    call: Option<String>,
    /// Constraints: stop, size, "watch only"…
    #[arg(long)]
    constraints: Option<String>,
    /// The signal: act when …
    #[arg(long)]
    signal: Option<String>,
}

impl From<PlanArgs> for Plan {
    fn from(a: PlanArgs) -> Self {
        Plan {
            why: a.why,
            call: a.call,
            constraints: a.constraints,
            signal: a.signal,
        }
    }
}

#[derive(Subcommand, Debug)]
enum ListCmd {
    /// Every list with its symbol count.
    Ls,
    /// One list's symbols with all their fields.
    Show { name: String },
    /// Add symbols (stocks or OANDA pairs) to a list, creating it if needed.
    /// Re-adding a symbol keeps its place; given fields replace its fields.
    Add {
        name: String,
        #[arg(required = true)]
        symbols: Vec<String>,
        #[command(flatten)]
        plan: PlanArgs,
    },
    /// Set fields on a symbol already on the list (e.g. your call before
    /// the open). Fields not given are kept; an empty value clears one.
    Set {
        name: String,
        symbol: String,
        #[command(flatten)]
        plan: PlanArgs,
    },
    /// Remove symbols from a list.
    Rm {
        name: String,
        #[arg(required = true)]
        symbols: Vec<String>,
    },
    /// Empty a list (keeps the name).
    Clear { name: String },
    /// Delete a list entirely.
    Delete { name: String },
}

pub async fn run(args: ListArgs, out: Out) -> ! {
    match exec(args.cmd) {
        Ok(v) => {
            out.ok(&v);
            std::process::exit(exit::OK);
        }
        Err(e) => {
            let code = if e.downcast_ref::<lists::ValidationError>().is_some() {
                exit::VALIDATION
            } else {
                exit::GENERIC
            };
            out.fail(code, "list_failed", format!("{e:#}"));
        }
    }
}

fn exec(cmd: ListCmd) -> Result<serde_json::Value> {
    let path = lists::lists_path()?;
    match cmd {
        ListCmd::Ls => {
            let f = lists::load_at(&path)?;
            let all: Vec<_> = f
                .lists
                .iter()
                .map(|(name, entries)| json!({ "name": name, "count": entries.len() }))
                .collect();
            Ok(json!({ "lists": all }))
        }
        ListCmd::Show { name } => {
            let name = lists::validate_name(&name)?;
            let f = lists::load_at(&path)?;
            let Some(entries) = f.lists.get(&name) else {
                return Err(lists::no_list(&name));
            };
            Ok(json!({ "name": name, "count": entries.len(), "entries": entries }))
        }
        ListCmd::Add {
            name,
            symbols,
            plan,
        } => {
            let name = lists::validate_name(&name)?;
            let symbols = normalized(&symbols);
            let known = crate::tui::store::known_symbols();
            lists::check_known(&symbols, known.as_ref())?;
            let plan = Plan::from(plan);
            let (r, count, changed) = lists::update_at(&path, |f| {
                let r = lists::add(f, &name, &symbols, &plan, Utc::now());
                let entries = &f.lists[&name];
                let changed: Vec<lists::Entry> = entries
                    .iter()
                    .filter(|e| r.added.contains(&e.symbol) || r.updated.contains(&e.symbol))
                    .cloned()
                    .collect();
                Ok((r, entries.len(), changed))
            })?;
            let mut v = json!({
                "name": name,
                "count": count,
                "added": r.added,
                "updated": r.updated,
                "entries": changed,
            });
            if known.is_none() {
                v["unverified"] = json!(symbols);
            }
            Ok(v)
        }
        ListCmd::Set { name, symbol, plan } => {
            let name = lists::validate_name(&name)?;
            let symbol = lists::normalize_symbol(&symbol);
            let plan = Plan::from(plan);
            let entry = lists::update_at(&path, |f| lists::set(f, &name, &symbol, &plan))?;
            Ok(json!({ "name": name, "entry": entry }))
        }
        ListCmd::Rm { name, symbols } => {
            let name = lists::validate_name(&name)?;
            let symbols = normalized(&symbols);
            let (removed, count) = lists::update_at(&path, |f| {
                let removed = lists::remove(f, &name, &symbols)?;
                Ok((removed, f.lists[&name].len()))
            })?;
            Ok(json!({ "name": name, "count": count, "removed": removed }))
        }
        ListCmd::Clear { name } => {
            let name = lists::validate_name(&name)?;
            let n = lists::update_at(&path, |f| {
                let entries = f.lists.get_mut(&name).ok_or_else(|| lists::no_list(&name))?;
                let n = entries.len();
                entries.clear();
                Ok(n)
            })?;
            Ok(json!({ "name": name, "cleared": n }))
        }
        ListCmd::Delete { name } => {
            let name = lists::validate_name(&name)?;
            lists::update_at(&path, |f| {
                f.lists
                    .remove(&name)
                    .map(|_| ())
                    .ok_or_else(|| lists::no_list(&name))
            })?;
            Ok(json!({ "name": name, "deleted": true }))
        }
    }
}

fn normalized(symbols: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    symbols
        .iter()
        .flat_map(|s| s.split(','))
        .map(lists::normalize_symbol)
        .filter(|s| !s.is_empty() && seen.insert(s.clone()))
        .collect()
}

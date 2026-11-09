//! `wickd list` — named symbol lists, the agent-facing way to build a "watch
//! tomorrow" list (wickd-lists AGT-2391). See [`crate::lists`] for the store.
//!
//!   wickd list ls
//!   wickd list show tomorrow
//!   wickd list add tomorrow NVDA AMD --note "earnings beat; watch 185"
//!   wickd list rm tomorrow AMD
//!   wickd list clear tomorrow
//!   wickd list delete tomorrow
//!
//! One JSON object per call (see `crate::output`). Symbols are upper-cased
//! (`eur/usd` → `EUR_USD`) and checked against the cached Alpaca and OANDA
//! symbol lists (`wickd tui` fills those caches); with no cache yet they're
//! accepted and reported as `unverified`.

use anyhow::Result;
use chrono::Utc;
use clap::{Args, Subcommand};
use serde_json::json;

use crate::lists;
use crate::output::{exit, Out};

#[derive(Args, Debug)]
pub struct ListArgs {
    #[command(subcommand)]
    cmd: ListCmd,
}

#[derive(Subcommand, Debug)]
enum ListCmd {
    /// Every list with its symbol count.
    Ls,
    /// One list's symbols and notes.
    Show { name: String },
    /// Add symbols (stocks or OANDA pairs) to a list, creating it if needed.
    /// Re-adding a symbol keeps its place; --note replaces its note.
    Add {
        name: String,
        #[arg(required = true)]
        symbols: Vec<String>,
        /// Why these are on the list (applies to every symbol given).
        #[arg(long)]
        note: Option<String>,
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
            note,
        } => {
            let name = lists::validate_name(&name)?;
            let symbols = normalized(&symbols);
            let known = crate::tui::store::known_symbols();
            lists::check_known(&symbols, known.as_ref())?;
            let added = lists::update_at(&path, |f| {
                Ok(lists::add(f, &name, &symbols, note.as_deref(), Utc::now()))
            })?;
            let mut v = show(&path, &name)?;
            v["added"] = json!(added);
            if known.is_none() {
                v["unverified"] = json!(symbols);
            }
            Ok(v)
        }
        ListCmd::Rm { name, symbols } => {
            let name = lists::validate_name(&name)?;
            let symbols = normalized(&symbols);
            let removed = lists::update_at(&path, |f| lists::remove(f, &name, &symbols))?;
            let mut v = show(&path, &name)?;
            v["removed"] = json!(removed);
            Ok(v)
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

fn show(path: &std::path::Path, name: &str) -> Result<serde_json::Value> {
    let f = lists::load_at(path)?;
    let entries = f.lists.get(name).cloned().unwrap_or_default();
    Ok(json!({ "name": name, "count": entries.len(), "entries": entries }))
}

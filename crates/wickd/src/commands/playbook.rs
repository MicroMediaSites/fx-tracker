//! `wickd playbook` — plain-language strategy docs the agent looks up on
//! demand (wickd-learn D-20, AGT-2546). See [`crate::playbooks`] for the
//! store.
//!
//!   wickd playbook add ~/strategies/rahagod.md --name rahagod
//!   wickd playbook ls
//!   wickd playbook show rahagod
//!   wickd playbook search tenkan close
//!   wickd playbook rm rahagod
//!
//! One JSON object per call (see `crate::output`). Unknown names and invalid
//! names exit with the validation code; an empty store lists as empty.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::json;

use crate::lists::ValidationError;
use crate::output::{exit, Out};
use crate::playbooks;

#[derive(Args, Debug)]
pub struct PlaybookArgs {
    #[command(subcommand)]
    cmd: PlaybookCmd,
}

#[derive(Subcommand, Debug)]
enum PlaybookCmd {
    /// Copy a markdown file into the playbooks store. The name defaults to
    /// the file's stem; re-adding a name replaces it.
    Add {
        file: PathBuf,
        /// The playbook's name (a-z, 0-9, - or _, max 32).
        #[arg(long)]
        name: Option<String>,
    },
    /// Every playbook with its title and size.
    Ls,
    /// One playbook's full text.
    Show { name: String },
    /// Remove a playbook.
    Rm { name: String },
    /// Playbooks whose sections contain every word (case-insensitive), each
    /// with the matching sections (nearest heading + paragraph).
    Search {
        #[arg(required = true)]
        words: Vec<String>,
    },
}

pub async fn run(args: PlaybookArgs, out: Out) -> ! {
    match exec(args.cmd) {
        Ok(v) => {
            out.ok(&v);
            std::process::exit(exit::OK);
        }
        Err(e) => {
            let code = if e.downcast_ref::<ValidationError>().is_some() {
                exit::VALIDATION
            } else {
                exit::GENERIC
            };
            out.fail(code, "playbook_failed", format!("{e:#}"));
        }
    }
}

fn exec(cmd: PlaybookCmd) -> Result<serde_json::Value> {
    let dir = playbooks::playbooks_dir()?;
    match cmd {
        PlaybookCmd::Add { file, name } => {
            let name = match name {
                Some(n) => playbooks::validate_name(&n)?,
                None => playbooks::name_from_path(&file)?,
            };
            let text = playbooks::read_source(&file)?;
            let replaced = playbooks::add_at(&dir, &name, &text)?;
            Ok(json!({
                "name": name,
                "title": playbooks::title(&name, &text),
                "size": text.len(),
                "replaced": replaced,
            }))
        }
        PlaybookCmd::Ls => Ok(json!({ "playbooks": playbooks::list_at(&dir)? })),
        PlaybookCmd::Show { name } => {
            let name = playbooks::validate_name(&name)?;
            let text = playbooks::show_at(&dir, &name)?;
            Ok(json!({
                "name": name,
                "title": playbooks::title(&name, &text),
                "size": text.len(),
                "text": text,
            }))
        }
        PlaybookCmd::Rm { name } => {
            let name = playbooks::validate_name(&name)?;
            playbooks::remove_at(&dir, &name)?;
            Ok(json!({ "name": name, "removed": true }))
        }
        PlaybookCmd::Search { words } => {
            let words: Vec<String> = words
                .iter()
                .flat_map(|w| w.split_whitespace())
                .map(str::to_string)
                .collect();
            if words.is_empty() {
                return Err(ValidationError("search needs at least one word".into()).into());
            }
            let hits = playbooks::search_at(&dir, &words)?;
            Ok(json!({ "words": words, "playbooks": hits }))
        }
    }
}

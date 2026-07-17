//! Observing the watcher runtime from outside (lifted from the app's daemon
//! commands so the CLI's feed producer can share it).
//!
//! Daemon liveness has no status socket, so it is observed from the process
//! table: any process whose binary basename starts with `wickd` running the
//! `watch` verb counts (including pinned binaries like `wickd-h004`).

use serde::{Deserialize, Serialize};

/// One running `wickd ... watch ...` process.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WatchProcess {
    pub pid: u32,
    /// Full command line, e.g.
    /// `/Users/x/.wickd/bin/wickd-h004 watch revert_adx EUR_USD --auto`.
    pub command: String,
    /// The strategy argument (first arg after `watch`), when parseable.
    pub strategy: Option<String>,
    /// The instruments argument (second arg after `watch`), when parseable.
    pub instruments: Vec<String>,
}

/// Parse `ps` output lines (`PID COMMAND...`) into watch processes.
///
/// A line counts when the executable's basename starts with `wickd` and its
/// first argument is the `watch` verb — matching both the repo binary and
/// pinned copies (`wickd-h004`), and never e.g. `grep wickd watch`.
pub fn parse_watch_processes(ps_output: &str) -> Vec<WatchProcess> {
    let mut out = Vec::new();
    for line in ps_output.lines() {
        let line = line.trim();
        let Some((pid_str, command)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = pid_str.trim().parse::<u32>() else {
            continue;
        };
        let command = command.trim();
        let mut parts = command.split_whitespace();
        let Some(binary) = parts.next() else { continue };
        let basename = binary.rsplit('/').next().unwrap_or(binary);
        if !basename.starts_with("wickd") {
            continue;
        }
        if parts.next() != Some("watch") {
            continue;
        }
        // `wickd watch <strategy> <instruments,csv> [flags...]`
        let strategy = parts.next().map(str::to_string);
        let instruments = parts
            .next()
            .filter(|a| !a.starts_with('-'))
            .map(|csv| csv.split(',').map(str::to_string).collect())
            .unwrap_or_default();
        out.push(WatchProcess {
            pid,
            command: command.to_string(),
            strategy,
            instruments,
        });
    }
    out
}

/// Snapshot the running watch processes from the live process table. Errors
/// (no `ps`, non-zero exit) degrade to "none observed" — callers treat this
/// as a best-effort signal, never a hard failure.
pub fn running_watchers() -> Vec<WatchProcess> {
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output();
    match output {
        Ok(o) if o.status.success() => parse_watch_processes(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repo_and_pinned_watch_processes() {
        let ps = "\
  101 /usr/local/bin/wickd watch revert_adx EUR_USD,GBP_USD --granularity H1 --auto
  102 /Users/m/.wickd/bin/wickd-h004 watch revert_adx EUR_USD,GBP_USD,USD_CHF,EUR_GBP --granularity H1 --env practice --account h004 --units 2000 --auto
  103 grep wickd watch
  104 /usr/local/bin/wickd stream EUR_USD
  105 nvim wickd-watch-notes.md
";
        let procs = parse_watch_processes(ps);
        assert_eq!(procs.len(), 2, "{procs:?}");
        assert_eq!(procs[0].pid, 101);
        assert_eq!(procs[0].strategy.as_deref(), Some("revert_adx"));
        assert_eq!(procs[0].instruments, vec!["EUR_USD", "GBP_USD"]);
        assert_eq!(procs[1].pid, 102);
        assert_eq!(
            procs[1].instruments,
            vec!["EUR_USD", "GBP_USD", "USD_CHF", "EUR_GBP"]
        );
    }

    #[test]
    fn watch_verb_must_be_the_first_argument() {
        let ps = "  55 /usr/local/bin/wickd queue list --follow watch";
        assert!(parse_watch_processes(ps).is_empty());
    }
}

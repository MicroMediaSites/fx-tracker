//! Output contract for the CLI.
//!
//! Agent-first: every command prints exactly one JSON object to stdout on
//! success and exits 0. Errors print `{"error":{"code","message"}}` to stdout
//! and exit with a stable, category-specific code so a wrapping agent can
//! branch on `$?` without parsing text. `--pretty` switches to human-readable
//! stderr lines (stdout still carries the JSON unless noted).
//!
//! Streaming commands (`listen`) emit NDJSON — one JSON object per line — via
//! the sink, not through here.

use serde::Serialize;

/// Stable process exit codes. Keep these in sync with the CLI README so agents
/// can rely on them.
pub mod exit {
    pub const OK: i32 = 0;
    /// Generic / unexpected failure.
    pub const GENERIC: i32 = 1;
    /// Auth: missing credentials or a keychain read failure.
    pub const AUTH: i32 = 2;
    /// OANDA API failure (network, rejected request, bad account).
    pub const OANDA: i32 = 3;
    /// Validation: bad arguments or invalid strategy file.
    pub const VALIDATION: i32 = 4;
}

#[derive(Clone, Copy)]
pub struct Out {
    pub pretty: bool,
}

impl Out {
    pub fn new(pretty: bool) -> Self {
        Self { pretty }
    }

    /// Print a success payload (one JSON object) and return Ok.
    pub fn ok<T: Serialize>(&self, payload: &T) {
        if self.pretty {
            // Human mode still emits valid JSON, just indented.
            println!("{}", serde_json::to_string_pretty(payload).unwrap_or_else(|_| "{}".into()));
        } else {
            println!("{}", serde_json::to_string(payload).unwrap_or_else(|_| "{}".into()));
        }
    }

    /// Print an error envelope and exit with the given code. Never returns.
    pub fn fail(&self, code: i32, kind: &str, message: impl AsRef<str>) -> ! {
        let message = message.as_ref();
        if self.pretty {
            eprintln!("error [{kind}]: {message}");
        } else {
            let env = serde_json::json!({ "error": { "code": kind, "message": message } });
            println!("{}", serde_json::to_string(&env).unwrap_or_else(|_| "{}".into()));
        }
        std::process::exit(code);
    }
}


//! Tiny interactive-prompt helpers. All prompts are written to **stderr** so
//! that stdout carries only the command's JSON result (agents parse stdout).

use std::io::Write;

use anyhow::Result;

/// Prompt for a line of visible input (e.g. an account id).
pub fn line(label: &str) -> Result<String> {
    use std::io::BufRead;
    eprint!("{label}");
    std::io::stderr().flush()?;
    let mut s = String::new();
    std::io::stdin().lock().read_line(&mut s)?;
    Ok(s.trim().to_string())
}

/// Prompt for a secret (no echo). Reads from the TTY.
pub fn secret(label: &str) -> Result<String> {
    eprint!("{label}");
    std::io::stderr().flush()?;
    Ok(rpassword::read_password()?)
}

/// True if we appear to be attached to an interactive terminal on stdin.
pub fn is_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

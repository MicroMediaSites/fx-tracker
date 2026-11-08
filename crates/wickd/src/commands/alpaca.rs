//! `wickd alpaca` — Alpaca market-data credentials for the stock ticker.
//!
//!   wickd alpaca login     # prompts for key id + secret, validates, stores in keychain
//!   wickd alpaca status    # where keys resolve from (no secrets shown)
//!   wickd alpaca logout    # remove the keychain item
//!
//! Free keys come from an Alpaca paper account (alpaca.markets → Paper
//! Trading → API Keys). Nothing here trades: the keys are used read-only for
//! quotes, history, the symbol list and the market clock.
//!
//! Like `wickd login`, the secret is never accepted as a flag — it's read with
//! no echo on a TTY. Agents can skip the keychain entirely by exporting
//! `APCA_API_KEY_ID` / `APCA_API_SECRET_KEY`.

use clap::{Args, Subcommand};

use wickd_core::equities::alpaca::{AlpacaClient, AlpacaCredentials, AlpacaError, QuoteFeed};

use crate::alpaca_creds;
use crate::output::{exit, Out};
use crate::prompt;

#[derive(Args, Debug)]
pub struct AlpacaArgs {
    #[command(subcommand)]
    pub command: AlpacaCommand,
}

#[derive(Subcommand, Debug)]
pub enum AlpacaCommand {
    /// Store Alpaca API keys in the OS keychain (validated first).
    Login,
    /// Show where Alpaca keys resolve from. No secrets shown.
    Status,
    /// Remove the stored Alpaca keys from the keychain.
    Logout,
}

pub async fn run(args: AlpacaArgs, out: Out) -> ! {
    match args.command {
        AlpacaCommand::Login => login(out).await,
        AlpacaCommand::Status => status(out),
        AlpacaCommand::Logout => logout(out),
    }
}

async fn login(out: Out) -> ! {
    let key_id = match prompt::line("Alpaca API key id: ") {
        Ok(k) => k,
        Err(e) => out.fail(exit::GENERIC, "prompt_failed", format!("{e:#}")),
    };
    let secret = match prompt::secret("Alpaca API secret: ") {
        Ok(s) => s.trim().to_string(),
        Err(e) => out.fail(exit::GENERIC, "prompt_failed", format!("{e:#}")),
    };
    if key_id.is_empty() || secret.is_empty() {
        out.fail(
            exit::VALIDATION,
            "missing_credentials",
            "key id and secret are required",
        );
    }
    let creds = AlpacaCredentials { key_id, secret };

    // Validate with a real (read-only) call before persisting.
    let client = AlpacaClient::new(creds.clone(), QuoteFeed::Iex);
    let clock = match client.clock().await {
        Ok(c) => c,
        Err(AlpacaError::Auth(m)) | Err(AlpacaError::Forbidden(m)) => out.fail(
            exit::AUTH,
            "credentials_rejected",
            format!(
                "Alpaca rejected these keys ({m}). Use keys from your Alpaca *paper* account \
                 (Paper Trading → API Keys)."
            ),
        ),
        Err(e) => out.fail(exit::GENERIC, "validation_failed", e.to_string()),
    };

    if let Err(e) = alpaca_creds::store(&creds) {
        out.fail(exit::GENERIC, "store_failed", format!("{e:#}"));
    }
    out.ok(&serde_json::json!({
        "ok": true,
        "stored": "keychain",
        "key_id": alpaca_creds::mask(&creds.key_id),
        "market_open": clock.is_open,
    }));
    std::process::exit(exit::OK)
}

fn status(out: Out) -> ! {
    match alpaca_creds::resolve() {
        Ok(Some((c, src))) => out.ok(&serde_json::json!({
            "configured": true,
            "source": src.as_str(),
            "key_id": alpaca_creds::mask(&c.key_id),
        })),
        Ok(None) => out.ok(&serde_json::json!({
            "configured": false,
            "hint": "run `wickd alpaca login`",
        })),
        Err(e) => out.fail(exit::GENERIC, "status_failed", format!("{e:#}")),
    }
    std::process::exit(exit::OK)
}

fn logout(out: Out) -> ! {
    match alpaca_creds::delete() {
        Ok(removed) => out.ok(&serde_json::json!({ "ok": true, "removed": removed })),
        Err(e) => out.fail(exit::GENERIC, "logout_failed", format!("{e:#}")),
    }
    std::process::exit(exit::OK)
}

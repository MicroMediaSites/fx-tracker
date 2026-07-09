//! `wickd instruments` — list tradeable OANDA instruments as JSON.

use anyhow::{Context, Result};
use clap::Args;

use wickd_core::oanda::endpoints;

use crate::commands::client;
use crate::vault_store;
use crate::output::{exit, Out};

#[derive(Args, Debug)]
pub struct InstrumentsArgs {
    /// OANDA environment whose stored credentials are used.
    #[arg(long, default_value = "practice")]
    pub env: String,
}

pub async fn run(args: InstrumentsArgs, out: Out) -> ! {
    let result: Result<serde_json::Value> = async {
        let (_env, client) = client::resolve(&args.env, vault_store::DEFAULT_ACCOUNT)?;
        let instruments = endpoints::get_instruments(&client)
            .await
            .context("OANDA instruments fetch failed")?;
        Ok(serde_json::json!({
            "count": instruments.len(),
            "instruments": instruments,
        }))
    }
    .await;

    match result {
        Ok(v) => {
            out.ok(&v);
            std::process::exit(exit::OK);
        }
        Err(e) => {
            let msg = format!("{e:#}");
            let code = if msg.contains("keychain") || msg.contains("credentials") {
                exit::AUTH
            } else {
                exit::OANDA
            };
            out.fail(code, "instruments_failed", msg);
        }
    }
}

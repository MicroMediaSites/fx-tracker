//! wickd MCP Server
//!
//! Local, single-user MCP server over **stdio**, reading the wickd local
//! store (`~/.wickd/app.db`) — the same SQLite database the desktop app
//! owns (AGT-642/645/646/647). Landed by AGT-649, which retires the
//! Railway/Postgres HTTP deployment: no OAuth, no sessions, no rate
//! limiting, no feature gates — stdio is inherently local and the store is
//! single-user by design.
//!
//! Launch it from any MCP client config (Claude Desktop, `claude mcp add`):
//! `wickd-mcp` with optional env `WICKD_DB_PATH` (defaults to
//! `~/.wickd/app.db`) and `ANTHROPIC_API_KEY` (enables convert_strategy).
//!
//! Logging goes to **stderr** only — stdout carries the MCP transport.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use rusqlite::{params, Connection};
use rust_decimal::Decimal;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod sanitize;
mod shared;
mod store;
mod types;

use shared::StrategyDefinition;
use types::{*, validate_id, validate_price_range};

/// Placeholder owner id used only when validating strategy JSON against the
/// shared `StrategyDefinition` type (which still carries a `user_id` field
/// for wire compatibility). The local store itself has no user column.
const LOCAL_USER: &str = "local";

/// SECURITY: Create sanitized param error that logs details server-side but returns generic message (AUDIT-013)
fn param_parse_error(tool: &str, error: serde_json::Error) -> McpError {
    tracing::error!(tool = %tool, error = %error, "param parse error");
    McpError::invalid_params("Invalid parameters. Check required fields and types.", None)
}

/// SECURITY: Create sanitized internal error that logs details server-side but returns generic message (AUDIT-013)
fn db_error(operation: &str, error: impl std::fmt::Display) -> McpError {
    tracing::error!(operation = %operation, error = %error, "database error");
    McpError::internal_error("Operation failed. Please try again.", None)
}

// ============================================================================
// MCP Server Implementation
// ============================================================================

/// Main MCP server handler
#[derive(Clone)]
pub struct WickdMcp {
    /// The local store connection (`~/.wickd/app.db`). rusqlite is sync and
    /// calls are local + fast; a mutex around one connection is plenty for a
    /// single-user stdio server. Never held across an await.
    db: Arc<Mutex<Connection>>,
    tool_router: ToolRouter<Self>,
    /// Anthropic API key for direct AI calls (strategy conversion)
    anthropic_api_key: Option<String>,
    /// Out-of-band human confirmations pending for destructive ops:
    /// `resource_key` (e.g. `"delete_sr_zone:<id>"`) -> the nonce this server
    /// issued for it. A destructive call with no valid confirmation issues a
    /// nonce here; the op only proceeds once a HUMAN has created the matching
    /// ack file (see [`WickdMcp::check_out_of_band_confirmation`]).
    pending_confirmations: Arc<Mutex<HashMap<String, String>>>,
    /// Directory in which the HUMAN drops `<nonce>.ok` ack files to authorize a
    /// destructive op out-of-band. The MCP surface exposes no file-write tool,
    /// so the model cannot create these itself.
    confirm_dir: PathBuf,
}

impl WickdMcp {
    pub fn new(
        db: Arc<Mutex<Connection>>,
        anthropic_api_key: Option<String>,
        confirm_dir: PathBuf,
    ) -> Self {
        Self {
            db,
            tool_router: Self::tool_router(),
            anthropic_api_key,
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            confirm_dir,
        }
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A poisoned mutex means another tool call panicked mid-query; the
        // connection itself is still usable for independent statements.
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Enforce out-of-band HUMAN confirmation for a destructive operation.
    ///
    /// `resource_key` uniquely identifies the target (e.g.
    /// `"delete_sr_zone:<id>"`); `provided_token` is the nonce the caller echoed
    /// back (if any); `human_desc` is a human-readable description of what will
    /// be destroyed.
    ///
    /// The gate opens ([`ConfirmGate::Confirmed`]) only when BOTH hold:
    ///   1. `provided_token` equals a nonce THIS server issued for
    ///      `resource_key`, and
    ///   2. an ack file `<confirm_dir>/<nonce>.ok` exists on disk.
    ///
    /// The ack file can only be created by a human out-of-band — no MCP tool
    /// writes files — so a prompt-injected instruction that merely sets a flag
    /// or echoes the nonce it can read from the tool result cannot self-satisfy
    /// the gate. Confirmation is single-use: the nonce and ack file are consumed
    /// on success. Any peer destructive tool must route through this same gate.
    fn check_out_of_band_confirmation(
        &self,
        resource_key: &str,
        provided_token: Option<&str>,
        human_desc: &str,
    ) -> ConfirmGate {
        // Fast path: caller echoed a token — accept only if it matches the
        // nonce we issued for THIS resource AND the human's ack file is present.
        if let Some(tok) = provided_token.map(str::trim).filter(|t| !t.is_empty()) {
            let issued = self
                .pending_confirmations
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(resource_key)
                .cloned();
            if issued.as_deref() == Some(tok) {
                // `tok` is byte-equal to a UUID we generated, so it is safe to
                // use as a path component (no traversal).
                let ack_path = self.confirm_dir.join(format!("{tok}.ok"));
                if ack_path.exists() {
                    // Single-use: consume the pending nonce and the ack file.
                    self.pending_confirmations
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(resource_key);
                    let _ = std::fs::remove_file(&ack_path);
                    tracing::info!(resource = %resource_key, "out-of-band confirmation verified");
                    return ConfirmGate::Confirmed;
                }
            }
        }

        // Otherwise (re)issue a nonce and instruct the human. There is one map
        // entry per `resource_key`; the nonce value is refreshed on every pass
        // through this branch, invalidating any prior nonce for the same
        // resource (so an ack file for a stale nonce cannot open the gate).
        let nonce = uuid::Uuid::new_v4().to_string();
        self.pending_confirmations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(resource_key.to_string(), nonce.clone());
        // Best-effort: ensure the drop directory exists so `touch` works.
        let _ = std::fs::create_dir_all(&self.confirm_dir);
        let ack_path = self.confirm_dir.join(format!("{nonce}.ok"));
        // Human-visible channel (stderr) — NOT part of the model transport.
        tracing::warn!(
            resource = %resource_key,
            ack = %ack_path.display(),
            "destructive op requires out-of-band human confirmation"
        );
        ConfirmGate::NeedsConfirmation(format!(
            "⚠️ HUMAN CONFIRMATION REQUIRED\n\n\
             This will permanently delete {human_desc}.\n\n\
             This destructive action CANNOT be confirmed by the assistant alone.\n\
             A human must approve it out-of-band by creating an empty ack file:\n\n\
             \x20\x20{ack}\n\n\
             For example, run in a terminal:\n\
             \x20\x20touch \"{ack}\"\n\n\
             Then call this tool again with confirm_token: \"{nonce}\".",
            human_desc = human_desc,
            ack = ack_path.display(),
            nonce = nonce
        ))
    }
}

/// Outcome of an out-of-band confirmation check for a destructive op.
enum ConfirmGate {
    /// Human confirmation verified out-of-band; proceed with the destructive op.
    Confirmed,
    /// Not yet confirmed; return this message to the caller unchanged.
    NeedsConfirmation(String),
}

// ============================================================================
// Tool Implementations
// ============================================================================

#[tool_router]
impl WickdMcp {
    /// Get server information
    #[tool(description = "Get information about the wickd MCP server")]
    async fn get_server_info(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![Content::text(
            "wickd MCP Server v0.2.0 - local strategy management and trading tools \
             reading the wickd local store (~/.wickd/app.db).\n\n\
             Available tools:\n\
             - list_strategies: List all your strategies\n\
             - get_strategy: Get a specific strategy by ID\n\
             - create_strategy: Create a new strategy (validated against Rust types)\n\
             - update_strategy: Update an existing strategy\n\
             - get_strategy_help: Get strategy authoring documentation\n\
             - get_trades, get_open_trades, get_account_summary: Trade queries\n\
             - get_notes, create_note, update_note: Note management\n\
             - get_sr_zones, create_sr_zone, delete_sr_zone: S/R zone management\n\
             - get_backtests: Backtest results\n\
             - convert_strategy: Convert Pine Script/MQL/English to wickd JSON"
        )]))
    }

    /// List all strategies in the local store
    #[tool(description = "List all strategies (excludes archived)")]
    async fn list_strategies(&self) -> Result<CallToolResult, McpError> {
        let strategies: Vec<StrategyRow> = {
            let conn = self.conn();
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, description, is_active
                     FROM strategy
                     WHERE is_archived = 0
                     ORDER BY updated_at DESC",
                )
                .map_err(|e| db_error("list_strategies", e))?;
            let rows = stmt
                .query_map([], StrategyRow::from_row)
                .map_err(|e| db_error("list_strategies", e))?;
            rows.collect::<Result<_, _>>()
                .map_err(|e| db_error("list_strategies", e))?
        };

        // SECURITY: Sanitize user-controlled fields before returning to LLM (AUDIT-012)
        let response = serde_json::json!({
            "count": strategies.len(),
            "strategies": strategies.iter().map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "name": sanitize::sanitize_for_tool_result(&s.name),
                    "description": sanitize::sanitize_for_tool_result(&s.description),
                    "is_active": s.is_active,
                })
            }).collect::<Vec<_>>()
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap()
        )]))
    }

    /// Get a specific strategy by ID
    #[tool(description = "Get a specific strategy by ID with full details")]
    async fn get_strategy(&self, params: Parameters<GetStrategyParams>) -> Result<CallToolResult, McpError> {
        // SECURITY: Validate ID format early (AUDIT-014)
        if let Some(err) = validate_id(&params.0.id, "strategy_id") {
            return Err(McpError::invalid_params(err, None));
        }

        let strategy: Option<FullStrategyRow> = {
            let conn = self.conn();
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, description, schema_version, parameters, variables,
                            indicators, entry_rules, exit_rules, risk_settings,
                            is_active, version
                     FROM strategy
                     WHERE id = ?1 AND is_archived = 0",
                )
                .map_err(|e| db_error("get_strategy", e))?;
            stmt.query_row(params![params.0.id], FullStrategyRow::from_row)
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
                .map_err(|e| db_error("get_strategy", e))?
        };

        match strategy {
            Some(s) => {
                // SECURITY: Sanitize user-controlled fields (AUDIT-012)
                let response = serde_json::json!({
                    "id": s.id,
                    "name": sanitize::sanitize_for_tool_result(&s.name),
                    "description": sanitize::sanitize_for_tool_result(&s.description),
                    "schema_version": s.schema_version.unwrap_or(2),
                    "parameters": s.parameters.as_deref().and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok()).unwrap_or_default(),
                    "variables": s.variables.as_deref().and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok()).unwrap_or_default(),
                    "indicators": s.indicators.as_deref().and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok()).unwrap_or_default(),
                    "entry_rules": s.entry_rules.as_deref().and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok()).unwrap_or_default(),
                    "exit_rules": s.exit_rules.as_deref().and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok()).unwrap_or_default(),
                    "risk_settings": s.risk_settings.as_deref().and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok()).unwrap_or_default(),
                    "is_active": s.is_active,
                    "version": s.version,
                });
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&response).unwrap()
                )]))
            }
            None => Err(McpError::invalid_params("Strategy not found", None))
        }
    }

    /// Create a new strategy with validation
    #[tool(description = "Create a new trading strategy. The strategy is validated against the same Rust types used by the backtest engine, ensuring it will parse correctly.")]
    async fn create_strategy(&self, params: Parameters<CreateStrategyParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();

        // Extract and default optional fields, parsing stringified JSON
        let params_val = parse_json_value(p.parameters.unwrap_or(serde_json::json!([])));
        let vars_val = parse_json_value(p.variables.unwrap_or(serde_json::json!([])));
        let indicators_val = parse_json_value(p.indicators);
        let entry_rules_val = parse_json_value(p.entry_rules);
        let exit_rules_val = parse_json_value(p.exit_rules);
        let risk_settings_val = parse_json_value(p.risk_settings);

        // Build the complete strategy JSON
        let strategy_json = serde_json::json!({
            "id": id,
            "user_id": LOCAL_USER,
            "name": p.name,
            "description": p.description.as_deref().unwrap_or(""),
            "schema_version": 2,
            "parameters": params_val,
            "variables": vars_val,
            "indicators": indicators_val,
            "entry_rules": entry_rules_val,
            "entry_logic": { "mode": "all" },
            "exit_rules": exit_rules_val,
            "risk_settings": risk_settings_val,
            "version": 1,
            "is_active": true,
        });

        // CRITICAL: Validate by attempting to parse with the shared Rust types
        let strategy_str = serde_json::to_string(&strategy_json)
            .map_err(|e| {
                tracing::error!(error = %e, "Strategy serialization failed");
                McpError::invalid_params("Failed to process strategy. Check your input format.", None)
            })?;

        let _parsed: StrategyDefinition = serde_json::from_str(&strategy_str)
            .map_err(|e| {
                // SECURITY: Log full error server-side (AUDIT-013)
                tracing::error!(error = %e, "Strategy validation failed");
                McpError::invalid_params(
                    "Invalid strategy format. The strategy doesn't match expected types.\n\n\
                     Common issues:\n\
                     - PriceSource: Use {\"source\": \"price\", \"value\": \"close\"} NOT {\"type\": \"price\"}\n\
                     - IndicatorSource: Use {\"indicator\": \"...\", \"output\": \"...\"} with NO type field\n\
                     - StopLossSource: Use {\"type\": \"indicator\", ...} (this one DOES need type)\n\n\
                     Use get_strategy_help for full documentation.".to_string(),
                    None
                )
            })?;

        // Strategy validated! Now save to the local store
        let params_json = serde_json::to_string(&params_val).unwrap();
        let vars_json = serde_json::to_string(&vars_val).unwrap();
        let indicators_json = serde_json::to_string(&indicators_val).unwrap();
        let entry_json = serde_json::to_string(&entry_rules_val).unwrap();
        let exit_json = serde_json::to_string(&exit_rules_val).unwrap();
        let risk_json = serde_json::to_string(&risk_settings_val).unwrap();

        self.conn()
            .execute(
                "INSERT INTO strategy (
                    id, name, description, schema_version,
                    parameters, variables, indicators, entry_rules, entry_logic, exit_rules, risk_settings,
                    is_active, is_promoted, is_archived, is_locked, version, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
                params![
                    id,
                    p.name,
                    p.description.as_deref().unwrap_or(""),
                    2i64,
                    params_json,
                    vars_json,
                    indicators_json,
                    entry_json,
                    r#"{"mode":"all"}"#,
                    exit_json,
                    risk_json,
                    1i64,
                    0i64,
                    0i64,
                    0i64,
                    1i64,
                    now,
                    now
                ],
            )
            .map_err(|e| db_error("save_strategy", e))?;

        Ok(CallToolResult::success(vec![Content::text(
            format!("Successfully created strategy \"{}\" with ID: {}\n\nThe strategy has been validated and will parse correctly in the backtest engine.", p.name, id)
        )]))
    }

    /// Update an existing strategy
    #[tool(description = "Update an existing strategy. Only the fields you provide will be updated. The updated strategy is validated before saving.")]
    async fn update_strategy(&self, params: Parameters<UpdateStrategyParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;

        // SECURITY: Validate ID format early (AUDIT-014)
        if let Some(err) = validate_id(&p.id, "strategy_id") {
            return Err(McpError::invalid_params(err, None));
        }

        // First, fetch the existing strategy
        let existing: FullStrategyRow = {
            let conn = self.conn();
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, description, schema_version, parameters, variables,
                            indicators, entry_rules, exit_rules, risk_settings,
                            is_active, version
                     FROM strategy
                     WHERE id = ?1 AND is_archived = 0",
                )
                .map_err(|e| db_error("update_strategy", e))?;
            stmt.query_row(params![p.id], FullStrategyRow::from_row)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Err(McpError::invalid_params(
                        format!("Strategy not found: {}", p.id),
                        None,
                    )),
                    other => Err(db_error("update_strategy", other)),
                })?
        };

        // Merge updates with existing values, parsing stringified JSON.
        // Existing values may be NULL for older strategies — default to empty JSON.
        let name = p.name.unwrap_or(existing.name);
        let description = p.description.unwrap_or(existing.description);
        let parameters = p.parameters
            .map(|v| serde_json::to_string(&parse_json_value(v)).unwrap_or_else(|_| "[]".to_string()))
            .or(existing.parameters)
            .unwrap_or_else(|| "[]".to_string());
        let variables = p.variables
            .map(|v| serde_json::to_string(&parse_json_value(v)).unwrap_or_else(|_| "[]".to_string()))
            .or(existing.variables)
            .unwrap_or_else(|| "[]".to_string());
        let indicators = p.indicators
            .map(|v| serde_json::to_string(&parse_json_value(v)).unwrap_or_else(|_| "[]".to_string()))
            .or(existing.indicators)
            .unwrap_or_else(|| "[]".to_string());
        let entry_rules = p.entry_rules
            .map(|v| serde_json::to_string(&parse_json_value(v)).unwrap_or_else(|_| "[]".to_string()))
            .or(existing.entry_rules)
            .unwrap_or_else(|| "[]".to_string());
        let exit_rules = p.exit_rules
            .map(|v| serde_json::to_string(&parse_json_value(v)).unwrap_or_else(|_| "[]".to_string()))
            .or(existing.exit_rules)
            .unwrap_or_else(|| "[]".to_string());
        let risk_settings = p.risk_settings
            .map(|v| serde_json::to_string(&parse_json_value(v)).unwrap_or_else(|_| "{}".to_string()))
            .or(existing.risk_settings)
            .unwrap_or_else(|| "{}".to_string());

        // Build complete strategy for validation
        let strategy_json = serde_json::json!({
            "id": p.id,
            "user_id": LOCAL_USER,
            "name": name,
            "description": description,
            "schema_version": 2,
            "parameters": serde_json::from_str::<serde_json::Value>(&parameters).unwrap_or_default(),
            "variables": serde_json::from_str::<serde_json::Value>(&variables).unwrap_or_default(),
            "indicators": serde_json::from_str::<serde_json::Value>(&indicators).unwrap_or_default(),
            "entry_rules": serde_json::from_str::<serde_json::Value>(&entry_rules).unwrap_or_default(),
            "entry_logic": { "mode": "all" },
            "exit_rules": serde_json::from_str::<serde_json::Value>(&exit_rules).unwrap_or_default(),
            "risk_settings": serde_json::from_str::<serde_json::Value>(&risk_settings).unwrap_or_default(),
            "version": existing.version + 1,
            "is_active": true,
        });

        // Validate the merged strategy
        let strategy_str = serde_json::to_string(&strategy_json).unwrap();
        let _parsed: StrategyDefinition = serde_json::from_str(&strategy_str)
            .map_err(|e| {
                // SECURITY: Log full error server-side (AUDIT-013)
                tracing::error!(error = %e, "Strategy update validation failed");
                McpError::invalid_params(
                    "Invalid strategy after update. Use get_strategy_help for documentation.",
                    None
                )
            })?;

        // Update in the local store
        let now = chrono::Utc::now().timestamp_millis();
        self.conn()
            .execute(
                "UPDATE strategy SET
                    name = ?1, description = ?2, parameters = ?3, variables = ?4,
                    indicators = ?5, entry_rules = ?6, exit_rules = ?7, risk_settings = ?8,
                    version = version + 1, updated_at = ?9
                 WHERE id = ?10",
                params![
                    name, description, parameters, variables,
                    indicators, entry_rules, exit_rules, risk_settings,
                    now, p.id
                ],
            )
            .map_err(|e| db_error("update_strategy", e))?;

        Ok(CallToolResult::success(vec![Content::text(
            format!("Successfully updated strategy \"{}\" (ID: {})", name, p.id)
        )]))
    }

    /// Get help on strategy authoring
    #[tool(description = "Get comprehensive documentation on how to author trading strategies, including data source formats, trigger types, and common mistakes to avoid.")]
    async fn get_strategy_help(&self) -> Result<CallToolResult, McpError> {
        let content = include_str!("../docs/help/strategy-authoring.md");
        Ok(CallToolResult::success(vec![Content::text(content.to_string())]))
    }

    // ========================================================================
    // Trade Tools
    // ========================================================================

    /// Get aggregate trading statistics
    #[tool(description = "Get aggregate trading statistics including P&L, win rate, and trade count, across all synced accounts.")]
    async fn get_account_summary(&self, params: Parameters<GetAccountSummaryParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let pls: Vec<Option<String>> = {
            let conn = self.conn();
            let mut stmt = conn
                .prepare(
                    "SELECT realized_pl FROM trade
                     WHERE state = 'CLOSED'
                     AND (?1 IS NULL OR close_time >= ?1)
                     AND (?2 IS NULL OR close_time <= ?2)",
                )
                .map_err(|e| db_error("get_account_summary", e))?;
            let rows = stmt
                .query_map(
                    params![
                        p.date_from.as_deref().and_then(parse_date_input),
                        p.date_to.as_deref().and_then(parse_date_input)
                    ],
                    |row| row.get::<_, Option<String>>(0),
                )
                .map_err(|e| db_error("get_account_summary", e))?;
            rows.collect::<Result<_, _>>()
                .map_err(|e| db_error("get_account_summary", e))?
        };

        // House rule: money is Decimal, never f64 — realized_pl arrives as an
        // OANDA-precision string and is aggregated losslessly.
        let parsed: Vec<Decimal> = pls.iter()
            .filter_map(|pl| pl.as_ref()?.parse::<Decimal>().ok())
            .collect();
        let total = pls.len();
        let wins = parsed.iter().filter(|&&p| p > Decimal::ZERO).count();
        let losses = parsed.iter().filter(|&&p| p < Decimal::ZERO).count();
        let total_pl: Decimal = parsed.iter().copied().sum();
        // win_rate is a ratio of trade counts, not a money value — f64 is fine.
        let win_rate = if total > 0 { (wins as f64 / total as f64) * 100.0 } else { 0.0 };

        let winning_pls: Vec<Decimal> = parsed.iter().copied().filter(|&p| p > Decimal::ZERO).collect();
        let losing_pls: Vec<Decimal> = parsed.iter().copied().filter(|&p| p < Decimal::ZERO).collect();

        let avg_win = if winning_pls.is_empty() {
            Decimal::ZERO
        } else {
            winning_pls.iter().copied().sum::<Decimal>() / Decimal::from(winning_pls.len())
        };
        let avg_loss = if losing_pls.is_empty() {
            Decimal::ZERO
        } else {
            losing_pls.iter().copied().sum::<Decimal>().abs() / Decimal::from(losing_pls.len())
        };
        let profit_factor = if avg_loss > Decimal::ZERO && losses > 0 {
            (avg_win * Decimal::from(wins)) / (avg_loss * Decimal::from(losses))
        } else {
            Decimal::ZERO
        };

        let summary = serde_json::json!({
            "scope": "all_accounts",
            "total_trades": total,
            "wins": wins,
            "losses": losses,
            "win_rate": format!("{:.1}%", win_rate),
            "total_pl": format_money(total_pl),
            "avg_win": format_money(avg_win),
            "avg_loss": format_money(avg_loss),
            "profit_factor": format!("{:.2}", profit_factor.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero)),
        });

        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&summary).unwrap())]))
    }

    /// Query trade history with filters
    #[tool(description = "Query trade history with optional filters (instrument, state, date range).")]
    async fn get_trades(&self, params: Parameters<GetTradesParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let trades: Vec<FullTradeRow> = {
            let conn = self.conn();
            let mut stmt = conn
                .prepare(
                    "SELECT id, instrument, units, open_price, close_price, open_time, close_time, realized_pl, state, account_id
                     FROM trade
                     WHERE (?1 IS NULL OR instrument = ?1)
                     AND (?2 IS NULL OR state = ?2)
                     AND (?3 IS NULL OR open_time >= ?3)
                     AND (?4 IS NULL OR open_time <= ?4)
                     ORDER BY open_time DESC
                     LIMIT ?5",
                )
                .map_err(|e| db_error("get_trades", e))?;
            let rows = stmt
                .query_map(
                    params![
                        p.instrument,
                        p.state,
                        p.date_from.as_deref().and_then(parse_date_input),
                        p.date_to.as_deref().and_then(parse_date_input),
                        clamp_limit(p.limit)
                    ],
                    FullTradeRow::from_row,
                )
                .map_err(|e| db_error("get_trades", e))?;
            rows.collect::<Result<_, _>>()
                .map_err(|e| db_error("get_trades", e))?
        };

        let formatted: Vec<serde_json::Value> = trades.iter().map(|t| {
            serde_json::json!({
                "id": t.id,
                "instrument": t.instrument,
                "direction": trade_direction(&t.units),
                "units": t.units,
                "open_price": t.open_price,
                "close_price": t.close_price,
                "open_time": format_timestamp(t.open_time),
                "close_time": t.close_time.map(format_timestamp),
                "realized_pl": t.realized_pl.as_ref().map(|p| format_money(p.parse::<Decimal>().unwrap_or(Decimal::ZERO))),
                "state": t.state,
            })
        }).collect();

        let response = serde_json::json!({
            "scope": "all_accounts",
            "count": formatted.len(),
            "trades": formatted,
        });
        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&response).unwrap())]))
    }

    /// Get currently open positions
    #[tool(description = "Get currently open positions")]
    async fn get_open_trades(&self) -> Result<CallToolResult, McpError> {
        let trades: Vec<FullTradeRow> = {
            let conn = self.conn();
            let mut stmt = conn
                .prepare(
                    "SELECT id, instrument, units, open_price, close_price, open_time, close_time, realized_pl, state, account_id
                     FROM trade
                     WHERE state = 'OPEN'
                     ORDER BY open_time DESC
                     LIMIT 100",
                )
                .map_err(|e| db_error("get_open_trades", e))?;
            let rows = stmt
                .query_map([], FullTradeRow::from_row)
                .map_err(|e| db_error("get_open_trades", e))?;
            rows.collect::<Result<_, _>>()
                .map_err(|e| db_error("get_open_trades", e))?
        };

        if trades.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text("No open trades")]));
        }

        let formatted: Vec<serde_json::Value> = trades.iter().map(|t| {
            serde_json::json!({
                "id": t.id,
                "instrument": t.instrument,
                "direction": trade_direction(&t.units),
                "units": t.units,
                "open_price": t.open_price,
                "open_time": format_timestamp(t.open_time),
                "account_id": t.account_id,
            })
        }).collect();

        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&formatted).unwrap())]))
    }

    // ========================================================================
    // Note Tools
    // ========================================================================

    /// Get notes with optional filters
    #[tool(description = "Get notes with optional filters. Can filter by trade_id for trade notes or strategy_id for strategy notes.")]
    async fn get_notes(&self, params: Parameters<GetNotesParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let notes: Vec<NoteRow> = {
            let conn = self.conn();
            let search_pattern = p.search.as_ref().map(|s| format!("%{}%", s));
            let mut stmt = conn
                .prepare(
                    "SELECT id, trade_id, strategy_id, title, content, created_at, updated_at
                     FROM note
                     WHERE (?1 IS NULL OR trade_id = ?1)
                     AND (?2 IS NULL OR strategy_id = ?2)
                     AND (?3 IS NULL OR title LIKE ?3 OR content LIKE ?3)
                     ORDER BY updated_at DESC
                     LIMIT ?4",
                )
                .map_err(|e| db_error("get_notes", e))?;
            let rows = stmt
                .query_map(
                    params![p.trade_id, p.strategy_id, search_pattern, clamp_limit(p.limit)],
                    NoteRow::from_row,
                )
                .map_err(|e| db_error("get_notes", e))?;
            rows.collect::<Result<_, _>>()
                .map_err(|e| db_error("get_notes", e))?
        };

        if notes.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text("No notes found")]));
        }

        // SECURITY: Sanitize user-controlled fields before returning to LLM (AUDIT-012)
        let formatted: Vec<serde_json::Value> = notes.iter().map(|n| {
            serde_json::json!({
                "id": n.id,
                "trade_id": n.trade_id,
                "strategy_id": n.strategy_id,
                "title": sanitize::sanitize_for_tool_result(&n.title),
                "content": sanitize::sanitize_for_tool_result(&n.content),
                "created": format_timestamp(n.created_at),
                "updated": format_timestamp(n.updated_at),
            })
        }).collect();

        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&formatted).unwrap())]))
    }

    /// Create a new note
    #[tool(description = "Create a new note. Can be associated with a trade (trade_id) or strategy (strategy_id).")]
    async fn create_note(&self, params: Parameters<CreateNoteParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;

        // Validate content bounds
        if p.title.len() > MAX_TITLE_LENGTH {
            return Err(McpError::invalid_params(
                format!("Title exceeds maximum length of {} characters", MAX_TITLE_LENGTH),
                None
            ));
        }
        if p.content.len() > MAX_CONTENT_LENGTH {
            return Err(McpError::invalid_params(
                format!("Content exceeds maximum length of {} bytes", MAX_CONTENT_LENGTH),
                None
            ));
        }

        // Verify referenced trade exists
        if let Some(ref trade_id) = p.trade_id {
            let count: i64 = self.conn()
                .query_row("SELECT COUNT(*) FROM trade WHERE id = ?1", params![trade_id], |row| row.get(0))
                .map_err(|e| db_error("create_note", e))?;
            if count == 0 {
                return Err(McpError::invalid_params("Trade not found", None));
            }
        }

        // Verify referenced strategy exists
        if let Some(ref strategy_id) = p.strategy_id {
            let count: i64 = self.conn()
                .query_row("SELECT COUNT(*) FROM strategy WHERE id = ?1", params![strategy_id], |row| row.get(0))
                .map_err(|e| db_error("create_note", e))?;
            if count == 0 {
                return Err(McpError::invalid_params("Strategy not found", None));
            }
        }

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();

        self.conn()
            .execute(
                "INSERT INTO note (id, trade_id, strategy_id, title, content, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, p.trade_id, p.strategy_id, p.title, p.content, now, now],
            )
            .map_err(|e| db_error("create_note", e))?;

        Ok(CallToolResult::success(vec![Content::text(format!("Created note \"{}\" with ID: {}", p.title, id))]))
    }

    /// Update an existing note
    #[tool(description = "Update an existing note")]
    async fn update_note(&self, params: Parameters<UpdateNoteParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;

        // SECURITY: Validate ID format early (AUDIT-014)
        if let Some(err) = validate_id(&p.note_id, "note_id") {
            return Err(McpError::invalid_params(err, None));
        }

        // Validate content bounds
        if let Some(ref title) = p.title {
            if title.len() > MAX_TITLE_LENGTH {
                return Err(McpError::invalid_params(
                    format!("Title exceeds maximum length of {} characters", MAX_TITLE_LENGTH),
                    None
                ));
            }
        }
        if let Some(ref content) = p.content {
            if content.len() > MAX_CONTENT_LENGTH {
                return Err(McpError::invalid_params(
                    format!("Content exceeds maximum length of {} bytes", MAX_CONTENT_LENGTH),
                    None
                ));
            }
        }

        let count: i64 = self.conn()
            .query_row("SELECT COUNT(*) FROM note WHERE id = ?1", params![p.note_id], |row| row.get(0))
            .map_err(|e| db_error("update_note", e))?;
        if count == 0 {
            return Err(McpError::invalid_params("Note not found", None));
        }

        let now = chrono::Utc::now().timestamp_millis();

        self.conn()
            .execute(
                "UPDATE note SET
                    title = COALESCE(?1, title),
                    content = COALESCE(?2, content),
                    updated_at = ?3
                 WHERE id = ?4",
                params![p.title, p.content, now, p.note_id],
            )
            .map_err(|e| db_error("update_note", e))?;

        Ok(CallToolResult::success(vec![Content::text(format!("Updated note {}", p.note_id))]))
    }

    // ========================================================================
    // Zone Tools
    // ========================================================================

    /// Get S/R zones for an instrument
    #[tool(description = "Get support/resistance zones for an instrument")]
    async fn get_sr_zones(&self, params: Parameters<GetZonesParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let zones: Vec<ZoneRow> = {
            let conn = self.conn();
            let mut stmt = conn
                .prepare(
                    "SELECT id, instrument, upper_price, lower_price, label, color, created_at, updated_at
                     FROM sr_zone
                     WHERE instrument = ?1
                     ORDER BY CAST(upper_price AS REAL) DESC",
                )
                .map_err(|e| db_error("get_sr_zones", e))?;
            let rows = stmt
                .query_map(params![p.instrument], ZoneRow::from_row)
                .map_err(|e| db_error("get_sr_zones", e))?;
            rows.collect::<Result<_, _>>()
                .map_err(|e| db_error("get_sr_zones", e))?
        };

        if zones.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(format!("No S/R zones found for {}", p.instrument))]));
        }

        // SECURITY: Sanitize user-controlled label field (AUDIT-012)
        let formatted: Vec<serde_json::Value> = zones.iter().map(|z| {
            serde_json::json!({
                "id": z.id,
                "instrument": z.instrument,
                "upper_price": z.upper_price,
                "lower_price": z.lower_price,
                "label": z.label.as_ref().map(|l| sanitize::sanitize_for_tool_result(l)),
                "color": z.color,
                "created": format_timestamp(z.created_at),
                "updated": format_timestamp(z.updated_at),
            })
        }).collect();

        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&formatted).unwrap())]))
    }

    /// Create a new S/R zone
    #[tool(description = "Create a new support/resistance zone")]
    async fn create_sr_zone(&self, params: Parameters<CreateZoneParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;

        // SECURITY: Validate price format and range (AUDIT-014)
        let (_upper, _lower) = validate_price_range(&p.upper_price, &p.lower_price)
            .map_err(|e| McpError::invalid_params(e, None))?;

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp_millis();

        self.conn()
            .execute(
                "INSERT INTO sr_zone (id, instrument, upper_price, lower_price, label, color, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id, p.instrument, p.upper_price, p.lower_price, p.label, p.color, now, now],
            )
            .map_err(|e| db_error("create_zone", e))?;

        Ok(CallToolResult::success(vec![Content::text(
            format!("Created S/R zone for {} at {}-{} with ID: {}", p.instrument, p.lower_price, p.upper_price, id)
        )]))
    }

    /// Delete an S/R zone
    #[tool(description = "Delete a support/resistance zone. Destructive: requires \
        out-of-band HUMAN confirmation. The first call returns a nonce and an ack-file \
        path; a human must create that file, then call again echoing confirm_token. \
        The assistant cannot self-confirm this action.")]
    async fn delete_sr_zone(&self, params: Parameters<DeleteZoneParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;

        // SECURITY: Validate ID format early (AUDIT-014)
        if let Some(err) = validate_id(&p.zone_id, "zone_id") {
            return Err(McpError::invalid_params(err, None));
        }

        // First verify zone exists
        let zone: Option<(String, Option<String>)> = self.conn()
            .query_row(
                "SELECT instrument, label FROM sr_zone WHERE id = ?1",
                params![p.zone_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(|e| db_error("delete_zone", e))?;

        let (instrument, label) = match zone {
            Some(z) => z,
            None => return Err(McpError::invalid_params("Zone not found", None)),
        };

        // Approval gate (AUDIT-009 / AGT-670): require out-of-band HUMAN
        // confirmation that the model cannot self-satisfy from prompt content.
        let zone_desc = match label {
            Some(l) => format!("SR zone '{}' on {} (ID: {})", l, instrument, p.zone_id),
            None => format!("SR zone on {} (ID: {})", instrument, p.zone_id),
        };
        let resource_key = format!("delete_sr_zone:{}", p.zone_id);
        match self.check_out_of_band_confirmation(&resource_key, p.confirm_token.as_deref(), &zone_desc) {
            ConfirmGate::Confirmed => {}
            ConfirmGate::NeedsConfirmation(msg) => {
                return Ok(CallToolResult::success(vec![Content::text(msg)]));
            }
        }

        // Human confirmed out-of-band - execute deletion
        self.conn()
            .execute("DELETE FROM sr_zone WHERE id = ?1", params![p.zone_id])
            .map_err(|e| db_error("delete_zone", e))?;

        tracing::info!(zone_id = %p.zone_id, "SR zone deleted (confirmed)");
        Ok(CallToolResult::success(vec![Content::text(format!("Deleted zone {}", p.zone_id))]))
    }

    // ========================================================================
    // Backtest Tools
    // ========================================================================

    /// Get backtest results for a strategy
    #[tool(description = "Get backtest results for a strategy")]
    async fn get_backtests(&self, params: Parameters<GetBacktestsParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;

        // SECURITY: Validate ID format early (AUDIT-014)
        if let Some(err) = validate_id(&p.strategy_id, "strategy_id") {
            return Err(McpError::invalid_params(err, None));
        }

        // Verify strategy exists
        let count: i64 = self.conn()
            .query_row("SELECT COUNT(*) FROM strategy WHERE id = ?1", params![p.strategy_id], |row| row.get(0))
            .map_err(|e| db_error("get_backtests", e))?;
        if count == 0 {
            return Err(McpError::invalid_params("Strategy not found", None));
        }

        let jobs: Vec<BacktestJobRow> = {
            let conn = self.conn();
            let mut stmt = conn
                .prepare(
                    "SELECT id, job_type, params, result, created_at, completed_at
                     FROM backtest_job
                     WHERE strategy_id = ?1 AND status = 'completed'
                     ORDER BY created_at DESC
                     LIMIT ?2",
                )
                .map_err(|e| db_error("get_backtests", e))?;
            let rows = stmt
                .query_map(params![p.strategy_id, clamp_limit(p.limit)], BacktestJobRow::from_row)
                .map_err(|e| db_error("get_backtests", e))?;
            rows.collect::<Result<_, _>>()
                .map_err(|e| db_error("get_backtests", e))?
        };

        if jobs.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text("No completed backtests found for this strategy")]));
        }

        let formatted: Vec<serde_json::Value> = jobs.iter().map(|job| {
            let params: serde_json::Value = serde_json::from_str(&job.params).unwrap_or_default();
            let result: serde_json::Value = job.result.as_deref()
                .and_then(|r| serde_json::from_str(r).ok())
                .unwrap_or_default();

            // Extract metrics — structure varies by job_type:
            // - simple_backtest: { metrics: { total_trades, win_rate, ... }, trades: [...] }
            // - walk_forward: { oos_total_trades, oos_win_rate, oos_avg_sharpe, ... } (flat)
            // - optimization: { metrics: { ... }, best_params: { ... } }
            let metrics = if let Some(m) = result.get("metrics") {
                // Simple backtest / optimization: nested metrics object
                serde_json::json!({
                    "total_trades": m.get("total_trades"),
                    "win_rate": m.get("win_rate"),
                    "profit_factor": m.get("profit_factor"),
                    "total_return_pct": m.get("total_return_pct"),
                    "max_drawdown_pct": m.get("max_drawdown_pct"),
                    "sharpe_ratio": m.get("sharpe_ratio"),
                    "total_pnl": m.get("total_pnl"),
                })
            } else {
                // Walk-forward: flat oos_* fields at top level
                serde_json::json!({
                    "total_trades": result.get("oos_total_trades"),
                    "win_rate": result.get("oos_win_rate"),
                    "profit_factor": null,
                    "total_return_pct": result.get("oos_total_return_pct"),
                    "max_drawdown_pct": result.get("oos_max_drawdown_pct"),
                    "sharpe_ratio": result.get("oos_avg_sharpe"),
                    "total_pnl": result.get("oos_total_pnl"),
                    "robustness_score": result.get("robustness_score"),
                    "sharpe_efficiency": result.get("sharpe_efficiency"),
                    "profitable_periods": result.get("profitable_periods"),
                    "total_periods": result.get("total_periods"),
                })
            };

            serde_json::json!({
                "id": job.id,
                "job_type": job.job_type,
                "instrument": params.get("instrument"),
                "period": {
                    "start": params.get("start_date"),
                    "end": params.get("end_date"),
                },
                "metrics": metrics,
                "created": format_timestamp(job.created_at),
                "completed": job.completed_at.map(format_timestamp),
            })
        }).collect();

        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string_pretty(&formatted).unwrap())]))
    }

    // ========================================================================
    // Help Tools
    // ========================================================================

    /// List all available help topics
    #[tool(description = "List all available help documentation topics")]
    async fn list_help_topics(&self) -> Result<CallToolResult, McpError> {
        let topics = vec![
            ("strategy-authoring", "Complete guide to authoring trading strategies (V2 schema)"),
            ("indicators", "Available technical indicators and their configuration"),
        ];

        let formatted = topics.iter()
            .map(|(topic, title)| format!("- {}: {}", topic, title))
            .collect::<Vec<_>>()
            .join("\n");

        Ok(CallToolResult::success(vec![Content::text(format!("Available help topics:\n{}", formatted))]))
    }

    /// Get full help document by topic
    #[tool(description = "Get the full content of a help document by topic name. IMPORTANT: Before creating strategies, always read the 'strategy-authoring' topic for the current V2 schema.")]
    async fn get_help(&self, params: Parameters<GetHelpParams>) -> Result<CallToolResult, McpError> {
        let topic = &params.0.topic;

        let content = match topic.as_str() {
            "strategy-authoring" => include_str!("../docs/help/strategy-authoring.md"),
            "indicators" => include_str!("../docs/help/indicators.md"),
            _ => {
                return Err(McpError::invalid_params(
                    format!("Topic '{}' not found. Use list_help_topics to see available topics.", topic),
                    None
                ));
            }
        };

        Ok(CallToolResult::success(vec![Content::text(content.to_string())]))
    }

    // ========================================================================
    // Strategy Conversion
    // ========================================================================

    /// Convert a trading strategy from Pine Script, MQL4/5, or natural language to wickd JSON
    #[tool(description = "Convert a trading strategy script (Pine Script, MQL4, MQL5, or natural language) to wickd's V2 strategy JSON format using AI. The converted strategy is validated against the same Rust types used by the backtest engine. Source language must be one of: pine_script, mql4, mql5, natural_language.")]
    async fn convert_strategy(&self, params: Parameters<ConvertStrategyParams>) -> Result<CallToolResult, McpError> {
        let p = params.0;

        // Validate source language
        let valid_languages = ["pine_script", "mql4", "mql5", "natural_language"];
        if !valid_languages.contains(&p.source_language.as_str()) {
            return Err(McpError::invalid_params(
                format!("Invalid source_language: '{}'. Must be one of: {}", p.source_language, valid_languages.join(", ")),
                None
            ));
        }

        // Check for API key
        let api_key = self.anthropic_api_key.as_ref().ok_or_else(|| {
            McpError::internal_error(
                "Strategy conversion is not configured. ANTHROPIC_API_KEY is required.",
                None,
            )
        })?;

        // SECURITY: Sanitize script before AI processing
        let sanitized = sanitize::sanitize_script(&p.script).map_err(|e| {
            McpError::invalid_params(e, None)
        })?;

        tracing::info!(
            source_language = %p.source_language,
            sanitized_len = sanitized.len(),
            "[convert_strategy] Script sanitized, calling Anthropic API"
        );

        // Build user message
        let language_label = match p.source_language.as_str() {
            "pine_script" => "Pine Script (TradingView)",
            "mql4" => "MQL4 (MetaTrader 4)",
            "mql5" => "MQL5 (MetaTrader 5)",
            "natural_language" => "plain English description",
            _ => &p.source_language,
        };

        let user_message = format!(
            "Convert the following {} trading strategy to wickd JSON format.\n\n\
             Source script:\n```\n{}\n```\n\n\
             Return ONLY the JSON object, no markdown fences or explanations.",
            language_label, sanitized
        );

        let conversion_prompt = include_str!("../docs/help/conversion-prompt.md");

        // Call Anthropic API directly
        let client = reqwest::Client::builder()
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
            .build()
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to create HTTP client");
                McpError::internal_error("Internal error", None)
            })?;

        let request_body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 8192,
            "system": conversion_prompt,
            "messages": [{
                "role": "user",
                "content": user_message
            }]
        });

        let response = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Anthropic API request failed");
                McpError::internal_error("Strategy conversion failed. Please try again.", None)
            })?;

        let status = response.status();
        if !status.is_success() {
            let _error_text = response.text().await.unwrap_or_default();
            tracing::error!(status = %status, "Anthropic API error");
            return Err(McpError::internal_error(
                "Strategy conversion failed. Please try again.",
                None,
            ));
        }

        let api_response: serde_json::Value = response.json().await.map_err(|e| {
            tracing::error!(error = %e, "Failed to parse Anthropic response");
            McpError::internal_error("Strategy conversion failed.", None)
        })?;

        // Extract text from response
        let text = api_response["content"]
            .as_array()
            .and_then(|arr| arr.iter().find(|c| c["type"] == "text"))
            .and_then(|c| c["text"].as_str())
            .ok_or_else(|| McpError::internal_error("No text in AI response", None))?;

        // Extract JSON from response (handle markdown fences)
        let json_text = extract_json_from_response(text).map_err(|e| {
            McpError::internal_error(e, None)
        })?;

        // Validate the output against StrategyDefinition
        let validation_result = validate_converted_strategy(&json_text);
        if let Err(validation_error) = validation_result {
            tracing::warn!(
                error = %validation_error,
                "[convert_strategy] AI output failed validation"
            );
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "The AI generated a strategy, but it has validation errors:\n\n{}\n\n\
                 Raw JSON (may need manual corrections):\n```json\n{}\n```",
                validation_error, json_text
            ))]));
        }

        tracing::info!("[convert_strategy] Conversion successful, strategy validated");

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Successfully converted {} strategy to wickd JSON:\n\n```json\n{}\n```",
            language_label,
            serde_json::to_string_pretty(
                &serde_json::from_str::<serde_json::Value>(&json_text).unwrap_or_default()
            ).unwrap_or(json_text)
        ))]))
    }
}

/// Helper to parse JSON that might come as a string (Claude clients sometimes send stringified JSON)
fn parse_json_value(val: serde_json::Value) -> serde_json::Value {
    if let serde_json::Value::String(s) = &val {
        serde_json::from_str(s).unwrap_or(val)
    } else {
        val
    }
}

/// Extract JSON from AI response text, handling markdown code fences.
fn extract_json_from_response(text: &str) -> Result<String, String> {
    let trimmed = text.trim();

    // Direct JSON
    if trimmed.starts_with('{') {
        serde_json::from_str::<serde_json::Value>(trimmed)
            .map_err(|e| format!("Invalid JSON: {}", e))?;
        return Ok(trimmed.to_string());
    }

    // Markdown code fence: ```json ... ```
    if let Some(start) = trimmed.find("```json") {
        let json_start = start + 7;
        if let Some(end) = trimmed[json_start..].find("```") {
            let json = trimmed[json_start..json_start + end].trim();
            serde_json::from_str::<serde_json::Value>(json)
                .map_err(|e| format!("Invalid JSON in code block: {}", e))?;
            return Ok(json.to_string());
        }
    }

    // Plain code fence: ``` ... ```
    if let Some(start) = trimmed.find("```") {
        let fence_start = start + 3;
        let content_start = if let Some(newline) = trimmed[fence_start..].find('\n') {
            fence_start + newline + 1
        } else {
            fence_start
        };
        if let Some(end) = trimmed[content_start..].find("```") {
            let json = trimmed[content_start..content_start + end].trim();
            if json.starts_with('{') {
                serde_json::from_str::<serde_json::Value>(json)
                    .map_err(|e| format!("Invalid JSON in code block: {}", e))?;
                return Ok(json.to_string());
            }
        }
    }

    Err("AI response does not contain valid strategy JSON.".to_string())
}

/// Validate a converted strategy JSON against StrategyDefinition.
fn validate_converted_strategy(json: &str) -> Result<(), String> {
    let mut value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| format!("Invalid JSON: {}", e))?;

    // Check schema_version
    let schema_version = value.get("schema_version")
        .and_then(|v| v.as_i64())
        .ok_or("Missing schema_version. Expected schema_version: 2.")?;

    if schema_version != 2 {
        return Err(format!("Unsupported schema_version: {}. Expected 2.", schema_version));
    }

    // Add placeholder fields (same as Tauri's validate_strategy_json)
    if let Some(obj) = value.as_object_mut() {
        if !obj.contains_key("id") {
            obj.insert("id".to_string(), serde_json::Value::String("import-placeholder".to_string()));
        }
        if !obj.contains_key("user_id") {
            obj.insert("user_id".to_string(), serde_json::Value::String("import-placeholder".to_string()));
        }
        if !obj.contains_key("version") {
            obj.insert("version".to_string(), serde_json::Value::Number(1.into()));
        }
        if !obj.contains_key("is_active") {
            obj.insert("is_active".to_string(), serde_json::Value::Bool(true));
        }
        if !obj.contains_key("description") {
            obj.insert("description".to_string(), serde_json::Value::String("".to_string()));
        }
    }

    // Parse against StrategyDefinition
    let _strategy: StrategyDefinition = serde_json::from_value(value)
        .map_err(|e| format!("Invalid strategy: {}", e))?;

    Ok(())
}

#[tool_handler]
impl ServerHandler for WickdMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .build(),
            server_info: Implementation {
                name: "wickd-mcp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                ..Implementation::from_build_env()
            },
            instructions: Some(
                "wickd MCP Server - local trading strategy management over the wickd \
                 local store (~/.wickd/app.db) with built-in validation.\n\n\
                 Key feature: Strategies are validated against the same Rust types used by the \
                 backtest engine, preventing parse errors at runtime.\n\n\
                 Start with get_strategy_help to learn the strategy format.".into()
            ),
        }
    }
}

/// Format a money value for tool output ("$-5.00"). House rule: money is
/// Decimal end-to-end — no f64 on the way to the formatted string.
/// Rounds explicitly (Decimal's `{:.2}` pads but truncates extra digits).
fn format_money(amount: Decimal) -> String {
    format!(
        "${:.2}",
        amount.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
    )
}

/// Trade direction from the signed units string, parsed as Decimal
/// (house rule: position sizes are Decimal, never f64).
fn trade_direction(units: &str) -> &'static str {
    match units.parse::<Decimal>() {
        Ok(u) if u > Decimal::ZERO => "LONG",
        _ => "SHORT",
    }
}

// Helper function to format timestamps
fn format_timestamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ts)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| ts.to_string())
}

/// Parse a date input that may be an ISO 8601 string or epoch milliseconds.
/// Accepts: "2023-01-01", "2023-01-01T00:00:00Z", "2023-01-01T00:00:00+00:00", or "1672531200000"
fn parse_date_input(input: &str) -> Option<i64> {
    use chrono::{DateTime, NaiveDate, Utc};

    // Reasonable epoch-millis range: 2000-01-01 to 2100-01-01
    const MIN_EPOCH_MS: i64 = 946_684_800_000;
    const MAX_EPOCH_MS: i64 = 4_102_444_800_000;

    // Try RFC 3339 / ISO 8601 with time first: "2023-01-01T00:00:00Z"
    if let Ok(dt) = DateTime::parse_from_rfc3339(input) {
        return Some(dt.timestamp_millis());
    }
    // Try date-only: "2023-01-01" → treat as start of day UTC
    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        let dt = date.and_hms_opt(0, 0, 0)?;
        return Some(DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc).timestamp_millis());
    }
    // Try epoch millis last (avoids misinterpreting "20230101" as ~1970)
    if let Ok(ms) = input.parse::<i64>() {
        if ms >= MIN_EPOCH_MS && ms <= MAX_EPOCH_MS {
            return Some(ms);
        }
    }
    None
}

// ============================================================================
// Main Entry Point
// ============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging to STDERR - stdout carries the MCP stdio transport.
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .init();

    // Anthropic API key for strategy conversion (optional)
    let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY").ok();
    if anthropic_api_key.is_some() {
        tracing::info!("Anthropic API key configured - strategy conversion enabled");
    } else {
        tracing::warn!("ANTHROPIC_API_KEY not set - convert_strategy tool will be unavailable");
    }

    // Open the local store (WICKD_DB_PATH override, else ~/.wickd/app.db)
    let db_path = store::resolve_db_path()?;
    let conn = store::open(&db_path)?;
    tracing::info!(path = %db_path.display(), "wickd-mcp serving the local store over stdio");

    // Out-of-band confirmation drop directory: `WICKD_MCP_CONFIRM_DIR` override,
    // else `<db parent>/mcp-confirm`. A human drops `<nonce>.ok` ack files here
    // to authorize destructive tools; the MCP surface can't write files itself.
    let confirm_dir = std::env::var("WICKD_MCP_CONFIRM_DIR")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            db_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("mcp-confirm")
        });
    tracing::info!(path = %confirm_dir.display(), "out-of-band confirmation drop directory");

    let server = WickdMcp::new(Arc::new(Mutex::new(conn)), anthropic_api_key, confirm_dir);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // parse_date_input
    // ------------------------------------------------------------------

    #[test]
    fn test_parse_date_iso_date_only() {
        let ms = parse_date_input("2023-01-01").unwrap();
        assert_eq!(ms, 1672531200000); // 2023-01-01T00:00:00Z
    }

    #[test]
    fn test_parse_date_rfc3339_utc() {
        let ms = parse_date_input("2023-06-15T12:30:00Z").unwrap();
        assert_eq!(ms, 1686832200000);
    }

    #[test]
    fn test_parse_date_rfc3339_offset() {
        let ms = parse_date_input("2023-06-15T12:30:00+00:00").unwrap();
        assert_eq!(ms, 1686832200000);
    }

    #[test]
    fn test_parse_date_epoch_millis() {
        let ms = parse_date_input("1672531200000").unwrap();
        assert_eq!(ms, 1672531200000);
    }

    #[test]
    fn test_parse_date_rejects_compact_yyyymmdd() {
        // "20230101" as epoch millis would be ~1970, outside valid range
        assert!(parse_date_input("20230101").is_none());
    }

    #[test]
    fn test_parse_date_rejects_negative() {
        assert!(parse_date_input("-1").is_none());
    }

    #[test]
    fn test_parse_date_rejects_garbage() {
        assert!(parse_date_input("not-a-date").is_none());
    }

    // ------------------------------------------------------------------
    // Money formatting (Decimal house rule)
    // ------------------------------------------------------------------

    #[test]
    fn test_format_money_decimal() {
        assert_eq!(format_money("5".parse().unwrap()), "$5.00");
        assert_eq!(format_money("-5.006".parse().unwrap()), "$-5.01");
        assert_eq!(format_money("1.999".parse().unwrap()), "$2.00");
        assert_eq!(format_money(Decimal::ZERO), "$0.00");
    }

    #[test]
    fn test_trade_direction_decimal() {
        assert_eq!(trade_direction("100"), "LONG");
        assert_eq!(trade_direction("-0.5"), "SHORT");
        assert_eq!(trade_direction("0"), "SHORT");
        assert_eq!(trade_direction("not-a-number"), "SHORT");
    }

    // ------------------------------------------------------------------
    // Local-store-backed tools (AGT-649)
    // ------------------------------------------------------------------

    /// Server over a fresh temp store. Keeps the TempDir alive.
    fn test_server() -> (tempfile::TempDir, WickdMcp) {
        let dir = tempfile::tempdir().expect("tempdir");
        let conn = store::open(&dir.path().join("app.db")).expect("open temp store");
        let confirm_dir = dir.path().join("mcp-confirm");
        (dir, WickdMcp::new(Arc::new(Mutex::new(conn)), None, confirm_dir))
    }

    /// Extract the concatenated text content of a tool result.
    fn text_of(result: &CallToolResult) -> String {
        serde_json::to_value(&result.content)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["text"].as_str().map(str::to_string))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn seed_strategy(server: &WickdMcp, id: &str, name: &str, archived: bool) {
        server.conn().execute(
            "INSERT INTO strategy (id, name, description, schema_version, indicators, entry_rules,
                                   exit_rules, risk_settings, version, is_active, is_archived,
                                   created_at, updated_at)
             VALUES (?1, ?2, 'seeded', 2, '[]', '[]', '[]', '{}', 1, 1, ?3, 1000, 1000)",
            params![id, name, archived as i64],
        ).unwrap();
    }

    fn seed_trade(server: &WickdMcp, id: &str, instrument: &str, state: &str, pl: Option<&str>, open_time: i64) {
        server.conn().execute(
            "INSERT INTO trade (id, account_id, instrument, units, open_price, close_price,
                                open_time, close_time, realized_pl, state, synced_at, created_at, updated_at)
             VALUES (?1, 'acct-1', ?2, '100', '1.1000', ?5, ?3, ?6, ?4, ?7, 0, 0, 0)",
            params![
                id,
                instrument,
                open_time,
                pl,
                if state == "CLOSED" { Some("1.1050") } else { None },
                if state == "CLOSED" { Some(open_time + 3_600_000) } else { None },
                state
            ],
        ).unwrap();
    }

    #[test]
    fn test_migrations_bring_store_to_latest() {
        let (_dir, server) = test_server();
        let version: i64 = server.conn()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version as usize, store::migrations::MIGRATIONS.len());
    }

    #[tokio::test]
    async fn test_list_strategies_empty() {
        let (_dir, server) = test_server();
        let result = server.list_strategies().await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["count"], 0);
    }

    #[tokio::test]
    async fn test_list_strategies_excludes_archived() {
        let (_dir, server) = test_server();
        seed_strategy(&server, "s1", "Alpha", false);
        seed_strategy(&server, "s2", "Old", true);

        let result = server.list_strategies().await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["count"], 1);
        assert_eq!(v["strategies"][0]["id"], "s1");
        assert_eq!(v["strategies"][0]["name"], "Alpha");
    }

    #[tokio::test]
    async fn test_get_strategy_found_and_not_found() {
        let (_dir, server) = test_server();
        seed_strategy(&server, "s1", "Alpha", false);

        let result = server
            .get_strategy(Parameters(GetStrategyParams { id: "s1".into() }))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["id"], "s1");
        assert_eq!(v["schema_version"], 2);
        assert_eq!(v["is_active"], true);

        let missing = server
            .get_strategy(Parameters(GetStrategyParams { id: "nope".into() }))
            .await;
        assert!(missing.is_err());
    }

    #[tokio::test]
    async fn test_create_strategy_roundtrip() {
        let (_dir, server) = test_server();

        let result = server
            .create_strategy(Parameters(CreateStrategyParams {
                name: "MCP Created".into(),
                description: Some("from test".into()),
                indicators: serde_json::json!([]),
                parameters: None,
                variables: None,
                entry_rules: serde_json::json!([]),
                exit_rules: serde_json::json!([]),
                risk_settings: serde_json::json!({
                    "risk_method": "percent",
                    "risk_value": 1.0,
                    "rr_ratio": 2.0,
                    "spread_buffer_pips": 1.0
                }),
            }))
            .await
            .unwrap();
        let text = text_of(&result);
        assert!(text.contains("Successfully created strategy"), "got: {text}");

        // The created strategy is visible via list_strategies
        let list = server.list_strategies().await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_of(&list)).unwrap();
        assert_eq!(v["count"], 1);
        assert_eq!(v["strategies"][0]["name"], "MCP Created");
    }

    #[tokio::test]
    async fn test_create_strategy_rejects_invalid_risk_settings() {
        let (_dir, server) = test_server();

        let result = server
            .create_strategy(Parameters(CreateStrategyParams {
                name: "Bad".into(),
                description: None,
                indicators: serde_json::json!([]),
                parameters: None,
                variables: None,
                entry_rules: serde_json::json!([]),
                exit_rules: serde_json::json!([]),
                // risk_method is not a valid enum value → StrategyDefinition parse fails
                risk_settings: serde_json::json!({ "risk_method": "yolo" }),
            }))
            .await;
        assert!(result.is_err());

        // Nothing was persisted
        let count: i64 = server.conn()
            .query_row("SELECT COUNT(*) FROM strategy", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_update_strategy_merges_fields() {
        let (_dir, server) = test_server();
        seed_strategy(&server, "s1", "Alpha", false);

        let result = server
            .update_strategy(Parameters(UpdateStrategyParams {
                id: "s1".into(),
                name: Some("Alpha v2".into()),
                description: None,
                indicators: None,
                parameters: None,
                variables: None,
                entry_rules: None,
                exit_rules: None,
                risk_settings: Some(serde_json::json!({
                    "risk_method": "percent",
                    "risk_value": 2.0,
                    "rr_ratio": 3.0,
                    "spread_buffer_pips": 1.0
                })),
            }))
            .await
            .unwrap();
        assert!(text_of(&result).contains("Successfully updated strategy"));

        let (name, version): (String, i64) = server.conn()
            .query_row("SELECT name, version FROM strategy WHERE id = 's1'", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(name, "Alpha v2");
        assert_eq!(version, 2);
    }

    #[tokio::test]
    async fn test_trades_summary_and_filters() {
        let (_dir, server) = test_server();
        seed_trade(&server, "t1", "EUR_USD", "CLOSED", Some("10.00"), 1_700_000_000_000);
        seed_trade(&server, "t2", "EUR_USD", "CLOSED", Some("-5.00"), 1_700_100_000_000);
        seed_trade(&server, "t3", "GBP_USD", "OPEN", None, 1_700_200_000_000);

        // Summary over CLOSED trades
        let summary = server
            .get_account_summary(Parameters(GetAccountSummaryParams { date_from: None, date_to: None }))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_of(&summary)).unwrap();
        assert_eq!(v["total_trades"], 2);
        assert_eq!(v["wins"], 1);
        assert_eq!(v["losses"], 1);
        assert_eq!(v["win_rate"], "50.0%");
        assert_eq!(v["total_pl"], "$5.00");

        // Instrument filter
        let trades = server
            .get_trades(Parameters(GetTradesParams {
                instrument: Some("EUR_USD".into()),
                state: None,
                date_from: None,
                date_to: None,
                limit: 50,
            }))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_of(&trades)).unwrap();
        assert_eq!(v["count"], 2);

        // Open trades
        let open = server.get_open_trades().await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_of(&open)).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["id"], "t3");
    }

    #[tokio::test]
    async fn test_notes_crud_and_search() {
        let (_dir, server) = test_server();
        seed_strategy(&server, "s1", "Alpha", false);

        // Create
        let created = server
            .create_note(Parameters(CreateNoteParams {
                title: "Journal".into(),
                content: "London session breakout worked".into(),
                trade_id: None,
                strategy_id: Some("s1".into()),
            }))
            .await
            .unwrap();
        assert!(text_of(&created).contains("Created note"));

        // Referential check: unknown strategy rejected
        let bad = server
            .create_note(Parameters(CreateNoteParams {
                title: "x".into(),
                content: "y".into(),
                trade_id: None,
                strategy_id: Some("ghost".into()),
            }))
            .await;
        assert!(bad.is_err());

        // Search hits content
        let found = server
            .get_notes(Parameters(GetNotesParams {
                trade_id: None,
                strategy_id: None,
                search: Some("breakout".into()),
                limit: 20,
            }))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_of(&found)).unwrap();
        let note_id = v[0]["id"].as_str().unwrap().to_string();
        assert_eq!(v[0]["title"], "Journal");

        // Update
        let updated = server
            .update_note(Parameters(UpdateNoteParams {
                note_id: note_id.clone(),
                title: Some("Journal (edited)".into()),
                content: None,
            }))
            .await
            .unwrap();
        assert!(text_of(&updated).contains("Updated note"));

        let title: String = server.conn()
            .query_row("SELECT title FROM note WHERE id = ?1", params![note_id], |row| row.get(0))
            .unwrap();
        assert_eq!(title, "Journal (edited)");
    }

    #[tokio::test]
    async fn test_sr_zone_create_get_delete_with_confirm_gate() {
        let (_dir, server) = test_server();

        let created = server
            .create_sr_zone(Parameters(CreateZoneParams {
                instrument: "EUR_USD".into(),
                upper_price: "1.1050".into(),
                lower_price: "1.1000".into(),
                label: Some("daily S1".into()),
                color: None,
            }))
            .await
            .unwrap();
        assert!(text_of(&created).contains("Created S/R zone"));

        let zones = server
            .get_sr_zones(Parameters(GetZonesParams { instrument: "EUR_USD".into() }))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_of(&zones)).unwrap();
        let zone_id = v[0]["id"].as_str().unwrap().to_string();

        let resource_key = format!("delete_sr_zone:{}", zone_id);
        let count = |s: &WickdMcp| -> i64 {
            s.conn()
                .query_row("SELECT COUNT(*) FROM sr_zone", [], |row| row.get(0))
                .unwrap()
        };

        // First call (no token) → issues a nonce, zone survives.
        let gated = server
            .delete_sr_zone(Parameters(DeleteZoneParams { zone_id: zone_id.clone(), confirm_token: None }))
            .await
            .unwrap();
        assert!(text_of(&gated).contains("HUMAN CONFIRMATION REQUIRED"));
        assert_eq!(count(&server), 1);

        // The nonce the server issued (never fabricated by the caller).
        let nonce = server
            .pending_confirmations
            .lock()
            .unwrap()
            .get(&resource_key)
            .cloned()
            .expect("server issued a nonce");

        // Model self-satisfy attempt #1: a fabricated token, no ack file → still gated.
        let forged = server
            .delete_sr_zone(Parameters(DeleteZoneParams {
                zone_id: zone_id.clone(),
                confirm_token: Some("i-approve".into()),
            }))
            .await
            .unwrap();
        assert!(text_of(&forged).contains("HUMAN CONFIRMATION REQUIRED"));
        assert_eq!(count(&server), 1);

        // Model self-satisfy attempt #2: even the REAL server nonce (which the
        // model could read from the tool result) is insufficient without the
        // human's out-of-band ack file → still gated.
        let echoed = server
            .delete_sr_zone(Parameters(DeleteZoneParams {
                zone_id: zone_id.clone(),
                confirm_token: Some(nonce.clone()),
            }))
            .await
            .unwrap();
        assert!(text_of(&echoed).contains("HUMAN CONFIRMATION REQUIRED"));
        assert_eq!(count(&server), 1);

        // Re-fetch the (possibly re-issued) nonce, then simulate the HUMAN
        // creating the ack file out-of-band.
        let nonce = server
            .pending_confirmations
            .lock()
            .unwrap()
            .get(&resource_key)
            .cloned()
            .unwrap();
        std::fs::create_dir_all(&server.confirm_dir).unwrap();
        std::fs::write(server.confirm_dir.join(format!("{nonce}.ok")), b"").unwrap();

        // Now the confirmed call with the matching nonce → deletion runs.
        let deleted = server
            .delete_sr_zone(Parameters(DeleteZoneParams {
                zone_id: zone_id.clone(),
                confirm_token: Some(nonce.clone()),
            }))
            .await
            .unwrap();
        assert!(text_of(&deleted).contains("Deleted zone"));
        assert_eq!(count(&server), 0);

        // Confirmation is single-use: the ack file is consumed.
        assert!(!server.confirm_dir.join(format!("{nonce}.ok")).exists());
    }

    #[tokio::test]
    async fn test_get_backtests_from_local_jobs() {
        let (_dir, server) = test_server();
        seed_strategy(&server, "s1", "Alpha", false);

        server.conn().execute(
            "INSERT INTO backtest_job (id, strategy_id, job_type, status, params, result,
                                       created_at, updated_at, completed_at)
             VALUES ('j1', 's1', 'simple_backtest', 'completed',
                     '{\"instrument\":\"EUR_USD\",\"start_date\":\"2023-01-01\",\"end_date\":\"2023-06-01\"}',
                     '{\"metrics\":{\"total_trades\":42,\"win_rate\":55.0,\"total_pnl\":123.45}}',
                     1000, 2000, 2000)",
            [],
        ).unwrap();
        // A pending job must not appear
        server.conn().execute(
            "INSERT INTO backtest_job (id, strategy_id, job_type, status, params, created_at, updated_at)
             VALUES ('j2', 's1', 'simple_backtest', 'running', '{}', 3000, 3000)",
            [],
        ).unwrap();

        let result = server
            .get_backtests(Parameters(GetBacktestsParams { strategy_id: "s1".into(), limit: 10 }))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        let jobs = v.as_array().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0]["id"], "j1");
        assert_eq!(jobs[0]["metrics"]["total_trades"], 42);
        assert_eq!(jobs[0]["instrument"], "EUR_USD");

        // Unknown strategy → error
        let missing = server
            .get_backtests(Parameters(GetBacktestsParams { strategy_id: "ghost".into(), limit: 10 }))
            .await;
        assert!(missing.is_err());
    }
}

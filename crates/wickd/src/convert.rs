//! Rules-JSON → Rhai conversion (`wickd strategy convert`, AGT-651).
//!
//! Converts strategies authored in the retired visual builder's rules-JSON
//! format (`shared::StrategyDefinition`, `strategy_type: "rules"`) into
//! STRATEGY_ABI-dialect `.rhai` scripts, implementing recipes R1–R6 from
//! `docs/rhai-dialect-diff.md`:
//!
//! - R1 `compare`/`threshold` → SDK comparisons (incl. `is_within` distances)
//! - R2 `cross` → `crossed_above`/`crossed_below`, or the two-comparison
//!   expansion when a price leg / offset / lookback is involved
//! - R3 session givens → `candle_hour()` gates (bounds match
//!   `rules_triggers.rs`: London 08–17, US 13–22, Asian 00–09 UTC)
//! - R4 regime givens → auto-declared `adx`/`sma20`/`sma50`/`bollinger`
//!   indicators + helper fns mirroring `evaluate_givens_trigger`
//!   (ADX > 20 trend gate per PR #252, ranging = ADX < 20 ∧ BB width < 2%)
//! - R5 RR-based exits → exact via ABI v5 (`in_position()`, `entry_price()`)
//!   plus signal-time SL/TP tracked in script globals
//! - R6 bar-count exits → exact via ABI v5 `bars_since_entry()`
//!
//! `strategy_type: "scripted"` definitions are passed through verbatim
//! (their `script_content` already speaks the shared dialect — AGT-637
//! verified all archived scripted strategies validate as-is).
//!
//! What is deliberately NOT supported (the converter says so per strategy
//! instead of guessing): multi-timeframe indicators/sources (needs an MTF
//! ABI extension — "redesign" class), S/R-zone / pivot / pattern /
//! divergence / price-action-regime triggers (rules-engine-only features),
//! strategy variables, partial-close exits, and `capture: at_entry`
//! sources. Unsupported strategies are reported with the reason and skipped.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use shared::{
    ChainOperator, ComparisonOperator, Condition, CrossDirection, DataSource, DistanceUnit,
    EntryRule, ExitRule, IndicatorConfig, IndicatorType, MarketRegime, ParameterizedValue,
    RuleDirection, StopLossSource, StrategyDefinition, TimeCondition, Trigger, TriggerChain,
};
use wickd_core::backtest::validate_script;
use wickd_core::strategy_store::{content_hash, StrategyStore};

/// A conversion failure that names the unsupported construct — surfaced to
/// the user per strategy instead of a silent mistranslation.
type ConvertError = String;

// ============================================================================
// Entry point
// ============================================================================

/// Convert a JSON payload (one `StrategyDefinition` or an array of them) and
/// write the resulting `.rhai` files into `out_dir`. Returns the JSON report.
pub fn convert_file(raw: &str, out_dir: &Path, force: bool) -> Result<Value> {
    let defs: Vec<StrategyDefinition> = if raw.trim_start().starts_with('[') {
        serde_json::from_str(raw).context("parsing strategy definitions array")?
    } else {
        vec![serde_json::from_str(raw).context("parsing strategy definition")?]
    };

    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("creating output directory {}", out_dir.display()))?;

    let mut converted = Vec::new();
    let mut skipped = Vec::new();
    let mut seen_hashes: HashMap<String, String> = HashMap::new(); // content hash -> slug
    let mut used_slugs: HashMap<String, usize> = HashMap::new();

    for def in &defs {
        let script = match script_for(def) {
            Ok(Some(script)) => script,
            Ok(None) => {
                skipped.push(json!({ "name": def.name, "id": def.id, "reason": "empty shell (no entry rules and no indicators)" }));
                continue;
            }
            Err(reason) => {
                skipped.push(json!({ "name": def.name, "id": def.id, "reason": reason }));
                continue;
            }
        };

        // Dedupe identical strategies (the archive holds several
        // byte-identical bodies under different names). Keyed on the
        // semantic content, not the emitted script — the script header
        // embeds the original name/id.
        let hash = dedupe_key(def);
        if let Some(existing) = seen_hashes.get(&hash) {
            skipped.push(json!({ "name": def.name, "id": def.id, "reason": format!("duplicate of '{existing}' (identical script body)") }));
            continue;
        }

        // The generated script must validate — a failure here is a converter
        // bug, not a user error, so surface it loudly instead of writing.
        if let Err(e) = validate_script(&script) {
            skipped.push(json!({ "name": def.name, "id": def.id, "reason": format!("internal: generated script failed validation: {e}") }));
            continue;
        }

        // Slugify + de-collide within this run.
        let base = StrategyStore::slugify(&def.name);
        let n = used_slugs.entry(base.clone()).or_insert(0);
        *n += 1;
        let slug = if *n == 1 { base } else { format!("{}-{}", base, n) };

        let path = out_dir.join(format!("{slug}.rhai"));
        if path.exists() && !force {
            skipped.push(json!({ "name": def.name, "id": def.id, "reason": format!("{} already exists (use --force)", path.display()) }));
            continue;
        }
        std::fs::write(&path, &script)
            .with_context(|| format!("writing {}", path.display()))?;
        seen_hashes.insert(hash, slug.clone());
        converted.push(json!({
            "name": def.name,
            "id": def.id,
            "slug": slug,
            "path": path.display().to_string(),
            "kind": if def.strategy_type == "scripted" { "verbatim" } else { "converted" },
        }));
    }

    Ok(json!({
        "out_dir": out_dir.display().to_string(),
        "converted": converted,
        "skipped": skipped,
    }))
}

/// Content fingerprint over the parts that define behavior (name/id/flags
/// excluded), so renamed copies of the same strategy dedupe.
fn dedupe_key(def: &StrategyDefinition) -> String {
    if def.strategy_type == "scripted" {
        return content_hash(def.script_content.as_deref().unwrap_or(""));
    }
    let semantic = json!({
        "parameters": def.parameters,
        "indicators": def.indicators,
        "variables": def.variables,
        "entry_rules": def.entry_rules,
        "exit_rules": def.exit_rules,
        "risk_settings": def.risk_settings,
    });
    content_hash(&semantic.to_string())
}

/// Produce the `.rhai` source for one definition. `Ok(None)` = empty shell.
fn script_for(def: &StrategyDefinition) -> Result<Option<String>, ConvertError> {
    if def.strategy_type == "scripted" {
        return match &def.script_content {
            Some(s) if !s.trim().is_empty() => Ok(Some(s.clone())),
            _ => Err("scripted strategy has no script_content".to_string()),
        };
    }
    if def.entry_rules.is_empty() && def.indicators.is_empty() {
        return Ok(None);
    }
    convert_rules(def).map(Some)
}

// ============================================================================
// Rules → Rhai codegen
// ============================================================================

/// Which regime helpers a strategy needs (drives helper-fn + indicator emission).
#[derive(Default)]
struct Needs {
    trending: bool,
    ranging: bool,
    volatility: bool,
    rr: bool,
    tp_pct: bool,
    abs: bool,
    entry_time: bool,
}

struct Ctx<'a> {
    def: &'a StrategyDefinition,
    needs: Needs,
    /// Extra indicators the codegen requires (regime set), keyed by id.
    extra_indicators: Vec<IndicatorConfig>,
}

impl<'a> Ctx<'a> {
    fn new(def: &'a StrategyDefinition) -> Self {
        Self { def, needs: Needs::default(), extra_indicators: Vec::new() }
    }

    /// Find (or auto-declare) an indicator of `kind` with the given period
    /// param; returns its id. Used by the regime helpers (R4).
    fn ensure_indicator(&mut self, kind: IndicatorType, period: Option<f64>, fallback_id: &str) -> String {
        let matches_period = |cfg: &IndicatorConfig| match period {
            None => true,
            Some(p) => cfg
                .params
                .get("period")
                .and_then(|v| v.as_fixed())
                .map(|v| (v - p).abs() < f64::EPSILON)
                .unwrap_or(false),
        };
        if let Some(cfg) = self.def.indicators.iter().find(|c| c.indicator_type == kind && matches_period(c)) {
            return cfg.id.clone();
        }
        if let Some(cfg) = self.extra_indicators.iter().find(|c| c.indicator_type == kind && matches_period(c)) {
            return cfg.id.clone();
        }
        let params: Vec<(&str, f64)> = match period {
            Some(p) => vec![("period", p)],
            None => vec![("period", 20.0)],
        };
        self.extra_indicators.push(IndicatorConfig::new_fixed(fallback_id, kind, &params));
        fallback_id.to_string()
    }
}

fn convert_rules(def: &StrategyDefinition) -> Result<String, ConvertError> {
    // MTF is a redesign, not a translation (dialect report: no HTF access in
    // the Rhai ABI) — refuse loudly.
    if let Some(cfg) = def.indicators.iter().find(|c| c.timeframe.as_deref().map_or(false, |t| !t.is_empty())) {
        return Err(format!(
            "multi-timeframe indicator '{}' (timeframe {:?}) — the Rhai ABI has no HTF access; classified 'needs redesign'",
            cfg.id, cfg.timeframe
        ));
    }
    if !def.variables.is_empty() {
        return Err(format!(
            "strategy variables ({}) are not supported by the converter",
            def.variables.iter().map(|v| v.id.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }

    let mut ctx = Ctx::new(def);

    // --- entry rules ---
    let mut entry_blocks = Vec::new();
    for rule in &def.entry_rules {
        entry_blocks.push(entry_block(&mut ctx, rule)?);
    }
    if entry_blocks.is_empty() {
        return Err("no entry rules".to_string());
    }

    // --- exit rules, priority order (higher first, stable otherwise) ---
    let mut exits: Vec<&ExitRule> = def.exit_rules.iter().collect();
    exits.sort_by_key(|r| std::cmp::Reverse(r.priority));
    let mut exit_blocks = Vec::new();
    for rule in exits {
        exit_blocks.push(exit_block(&mut ctx, rule)?);
    }

    // --- assemble ---
    let mut indicators = def.indicators.clone();
    indicators.extend(ctx.extra_indicators.iter().cloned());
    let indicators_json = serde_json::to_string(&indicators).map_err(|e| e.to_string())?;
    let parameters_json = serde_json::to_string(&def.parameters).map_err(|e| e.to_string())?;

    let mut s = String::new();
    s.push_str(&format!(
        "// Converted from CandleSight rules strategy \"{}\" (id {})\n// by `wickd strategy convert` (AGT-651, recipes R1-R6 in docs/rhai-dialect-diff.md).\n",
        def.name.replace('\n', " "),
        def.id
    ));
    if !def.description.trim().is_empty() {
        for line in def.description.lines() {
            s.push_str(&format!("// {}\n", line));
        }
    }
    s.push_str(
        "//\n// Semantics notes: SL/TP are computed on the signal candle's close (like the\n\
         // rules engine); the backtest engine fills at the next candle's open. RR /\n\
         // percent-of-TP / time exits gate on the engine's ACTUAL position via ABI v5\n\
         // (in_position(), entry_price(), bars_since_entry()).\n",
    );
    s.push_str(&format!("// @indicators: {}\n", indicators_json));
    s.push_str(&format!("// @parameters: {}\n\n", parameters_json));

    s.push_str(
        "// Signal-time entry levels (globals persist across on_candle calls).\n\
         let sig_sl = 0.0;\nlet sig_tp = 0.0;\nlet sig_dir = \"\";\n",
    );
    if ctx.needs.entry_time {
        s.push_str("let sig_time = 0;\n");
    }
    s.push('\n');

    // helper fns
    if ctx.needs.abs {
        s.push_str("fn absd(x) {\n    if x < 0.0 { -x } else { x }\n}\n\n");
    }
    if ctx.needs.trending {
        let adx = ctx.ensure_indicator(IndicatorType::Adx, None, "regime_adx");
        let s20 = ctx.ensure_indicator(IndicatorType::Sma, Some(20.0), "regime_sma20");
        let s50 = ctx.ensure_indicator(IndicatorType::Sma, Some(50.0), "regime_sma50");
        s.push_str(&format!(
            "// R4: mirrors rules_triggers::evaluate_trending (ADX > 20 + SMA alignment;\n\
             // indicator() returns 0 while warming up, which blocks like the rules engine).\n\
             fn regime_trending(up) {{\n    let adx = indicator(\"{adx}\", \"value\");\n    let s20 = indicator(\"{s20}\", \"value\");\n    let s50 = indicator(\"{s50}\", \"value\");\n    let p = price(\"close\");\n    if adx <= 20.0 || s20 <= 0.0 || s50 <= 0.0 {{ return false; }}\n    if up {{ p > s20 && s20 > s50 }} else {{ p < s20 && s20 < s50 }}\n}}\n\n"
        ));
    }
    if ctx.needs.ranging {
        let adx = ctx.ensure_indicator(IndicatorType::Adx, None, "regime_adx");
        let bb = ctx.ensure_indicator(IndicatorType::Bollinger, None, "regime_bb");
        s.push_str(&format!(
            "// R4: mirrors rules_triggers Ranging (ADX < 20 and BB width < 2% of middle).\n\
             fn regime_ranging() {{\n    let adx = indicator(\"{adx}\", \"value\");\n    let mid = indicator(\"{bb}\", \"middle\");\n    if adx <= 0.0 || adx >= 20.0 || mid == 0.0 {{ return false; }}\n    let width = (indicator(\"{bb}\", \"upper\") - indicator(\"{bb}\", \"lower\")) / mid;\n    width < 0.02\n}}\n\n"
        ));
    }
    if ctx.needs.rr {
        s.push_str(
            "// R5: profit/risk relative to the engine's actual fill (ABI v5).\n\
             fn rr_profit() {\n    if sig_dir == \"long\" { price(\"close\") - entry_price() } else { entry_price() - price(\"close\") }\n}\n\
             fn rr_risk() {\n    if sig_dir == \"long\" { entry_price() - sig_sl } else { sig_sl - entry_price() }\n}\n\n",
        );
    }
    if ctx.needs.tp_pct {
        s.push_str(
            "fn tp_progress_pct() {\n    let total = if sig_dir == \"long\" { sig_tp - entry_price() } else { entry_price() - sig_tp };\n    if total <= 0.0 { return -1.0; }\n    rr_profit() * 100.0 / total\n}\n\n",
        );
    }

    // on_candle
    s.push_str("fn on_candle() {\n");
    if !exit_blocks.is_empty() {
        s.push_str("    if in_position() {\n");
        for b in &exit_blocks {
            s.push_str(b);
        }
        s.push_str("        return \"hold\";\n    }\n\n");
    } else {
        s.push_str("    if in_position() {\n        return \"hold\";\n    }\n\n");
    }
    for b in &entry_blocks {
        s.push_str(b);
    }
    s.push_str("    \"hold\"\n}\n\n");

    // on_position_closed clears signal-time state
    s.push_str("fn on_position_closed() {\n    sig_sl = 0.0;\n    sig_tp = 0.0;\n    sig_dir = \"\";\n");
    if ctx.needs.entry_time {
        s.push_str("    sig_time = 0;\n");
    }
    s.push_str("}\n");

    Ok(s)
}

fn entry_block(ctx: &mut Ctx, rule: &EntryRule) -> Result<String, ConvertError> {
    if rule.pending_order.is_some() {
        // Translatable in principle (the engine supports pending_order maps),
        // but resolving the DataSource-based trigger price is out of scope.
        return Err(format!("entry rule '{}' uses a pending order config", rule.id));
    }
    let (signal, dir, sl_cmp) = match rule.direction {
        RuleDirection::Long => ("buy", "long", "<"),
        RuleDirection::Short => ("sell", "short", ">"),
        RuleDirection::Both => {
            return Err(format!("entry rule '{}' has direction 'both' (ambiguous for signal emission)", rule.id))
        }
    };
    let conds = rule_conditions_expr(ctx, &rule.conditions, &rule.trigger_chain, false)?;
    let sl = stop_loss_expr(ctx, rule.direction)?;
    let rr = rr_ratio_expr(ctx, rule.direction)?;
    let (risk_expr, tp_op) = match rule.direction {
        RuleDirection::Short => ("sl - entry", "-"),
        _ => ("entry - sl", "+"),
    };
    let label = rule
        .name
        .clone()
        .unwrap_or_else(|| rule.id.clone());
    let mut b = String::new();
    b.push_str(&format!("    // entry: {} ({})\n", label.replace('\n', " "), dir));
    b.push_str(&format!("    if {conds} {{\n"));
    b.push_str("        let entry = price(\"close\");\n");
    b.push_str(&format!("        let sl = {sl};\n"));
    b.push_str(&format!("        if sl {sl_cmp} entry {{\n"));
    b.push_str(&format!("            let tp = entry {tp_op} ({risk_expr}) * ({rr});\n"));
    b.push_str(&format!("            sig_sl = sl;\n            sig_tp = tp;\n            sig_dir = \"{dir}\";\n"));
    if ctx.needs.entry_time {
        b.push_str("            sig_time = candle_time();\n");
    }
    b.push_str(&format!(
        "            return #{{ signal: \"{signal}\", rule_name: \"{}\", stop_loss: sl, take_profit: tp }};\n",
        escape_str(&label)
    ));
    b.push_str("        }\n    }\n");
    Ok(b)
}

fn exit_block(ctx: &mut Ctx, rule: &ExitRule) -> Result<String, ConvertError> {
    let close_pct = rule
        .close_percent
        .as_fixed()
        .ok_or_else(|| format!("exit rule '{}' has a parameterized close_percent", rule.id))?;
    if (close_pct - 100.0).abs() > f64::EPSILON {
        return Err(format!(
            "exit rule '{}' closes {}% — partial closes are not expressible in the scripted ABI",
            rule.id, close_pct
        ));
    }
    let gate = match rule.direction {
        RuleDirection::Long => "sig_dir == \"long\" && ",
        RuleDirection::Short => "sig_dir == \"short\" && ",
        RuleDirection::Both => "",
    };
    let conds = rule_conditions_expr(ctx, &rule.conditions, &rule.trigger_chain, true)?;
    let label = rule.name.clone().unwrap_or_else(|| rule.id.clone());
    Ok(format!(
        "        // exit: {} ({:?}, priority {})\n        if {gate}{conds} {{\n            return #{{ signal: \"close\", exit_reason: \"{}\" }};\n        }}\n",
        label.replace('\n', " "),
        rule.direction,
        rule.priority,
        escape_str(&label)
    ))
}

/// Combined boolean expression for a rule's conditions (AND'd), supporting
/// both the modern `conditions` form and the legacy `trigger_chain`.
fn rule_conditions_expr(
    ctx: &mut Ctx,
    conditions: &[Condition],
    legacy: &Option<TriggerChain>,
    in_exit: bool,
) -> Result<String, ConvertError> {
    let mut parts = Vec::new();
    for cond in conditions {
        // `disabled` semantics: non-zero = the condition is skipped (passes).
        match &cond.disabled {
            Some(ParameterizedValue::Fixed(v)) if *v != 0.0 => continue,
            Some(ParameterizedValue::Reference(r)) => {
                let inner = condition_expr(ctx, cond, in_exit)?;
                parts.push(format!("(param(\"{}\") != 0.0 || {})", r.param_id, inner));
                continue;
            }
            _ => {}
        }
        parts.push(condition_expr(ctx, cond, in_exit)?);
    }
    if let Some(chain) = legacy {
        let mut expr = trigger_expr(ctx, &chain.primary, in_exit)?;
        for link in &chain.chain {
            let rhs = trigger_expr(ctx, &link.trigger, in_exit)?;
            let op = match link.operator {
                ChainOperator::And => "&&",
                ChainOperator::Or => "||",
            };
            expr = format!("({expr} {op} {rhs})");
        }
        parts.push(expr);
    }
    if parts.is_empty() {
        return Err("rule has no conditions".to_string());
    }
    Ok(parts.into_iter().map(|p| format!("({p})")).collect::<Vec<_>>().join(" && "))
}

fn condition_expr(ctx: &mut Ctx, cond: &Condition, in_exit: bool) -> Result<String, ConvertError> {
    let mut expr = {
        let t = trigger_expr(ctx, &cond.primary.trigger, in_exit)?;
        if cond.primary.negated { format!("!({t})") } else { t }
    };
    for link in &cond.chain {
        let t = trigger_expr(ctx, &link.trigger.trigger, in_exit)?;
        let t = if link.trigger.negated { format!("!({t})") } else { t };
        let op = match link.operator {
            ChainOperator::And => "&&",
            ChainOperator::Or => "||",
        };
        expr = format!("({expr} {op} {t})");
    }
    Ok(expr)
}

fn trigger_expr(ctx: &mut Ctx, trigger: &Trigger, in_exit: bool) -> Result<String, ConvertError> {
    match trigger {
        Trigger::Compare(t) => {
            let lookback = fixed_lookback(&t.lookback)?;
            let mut alts = Vec::new();
            for i in 0..lookback {
                let l = source_expr(ctx, &t.left, i)?;
                let r = source_expr(ctx, &t.right, i)?;
                alts.push(match t.operator {
                    ComparisonOperator::IsWithin => {
                        let d = t
                            .distance
                            .as_ref()
                            .ok_or_else(|| "is_within comparison without a distance config".to_string())?;
                        ctx.needs.abs = true;
                        let dist = num_expr(&d.value);
                        match d.unit {
                            DistanceUnit::Pips => format!("absd(({l}) - ({r})) <= ({dist}) * pip_value()"),
                            DistanceUnit::Percent => format!("absd(({l}) - ({r})) <= (({dist}) / 100.0) * absd({r})"),
                            DistanceUnit::Atr => {
                                let atr = ctx
                                    .def
                                    .indicators
                                    .iter()
                                    .find(|c| c.indicator_type == IndicatorType::Atr)
                                    .map(|c| c.id.clone())
                                    .ok_or_else(|| "is_within(atr) requires an ATR indicator in the strategy".to_string())?;
                                format!("absd(({l}) - ({r})) <= ({dist}) * indicator(\"{atr}\", \"value\")")
                            }
                        }
                    }
                    op => format!("({l}) {} ({r})", cmp_op(op)?),
                });
            }
            Ok(join_or(alts))
        }
        Trigger::Threshold(t) => {
            let lookback = fixed_lookback(&t.lookback)?;
            let v = num_expr(&t.value);
            let mut alts = Vec::new();
            for i in 0..lookback {
                let s = source_expr(ctx, &t.source, i)?;
                let op = cmp_op(t.operator)?;
                alts.push(format!("({s}) {op} ({v})"));
            }
            Ok(join_or(alts))
        }
        Trigger::Cross(t) => {
            let lookback = fixed_lookback(&t.lookback)?;
            // Fast path (R2): indicator↔indicator at offset 0, lookback 1 →
            // the SDK's native cross detector.
            if lookback == 1 {
                if let (DataSource::Indicator(a), DataSource::Indicator(b)) = (&t.left, &t.right) {
                    if a.offset == 0 && b.offset == 0 && plain_indicator(a) && plain_indicator(b) {
                        let f = match t.direction {
                            CrossDirection::Above => "crossed_above",
                            CrossDirection::Below => "crossed_below",
                        };
                        return Ok(format!(
                            "{f}(\"{}\", \"{}\", \"{}\", \"{}\")",
                            a.indicator, a.output, b.indicator, b.output
                        ));
                    }
                }
            }
            // General expansion: prev-vs-current comparison per lookback step.
            let mut alts = Vec::new();
            for i in 0..lookback {
                let cl = source_expr(ctx, &t.left, i)?;
                let cr = source_expr(ctx, &t.right, i)?;
                let pl = source_expr(ctx, &t.left, i + 1)?;
                let pr = source_expr(ctx, &t.right, i + 1)?;
                alts.push(match t.direction {
                    CrossDirection::Above => format!("(({pl}) <= ({pr}) && ({cl}) > ({cr}))"),
                    CrossDirection::Below => format!("(({pl}) >= ({pr}) && ({cl}) < ({cr}))"),
                });
            }
            Ok(join_or(alts))
        }
        Trigger::Givens(t) => match t.regime {
            MarketRegime::TrendingUp => {
                ctx.needs.trending = true;
                Ok("regime_trending(true)".to_string())
            }
            MarketRegime::TrendingDown => {
                ctx.needs.trending = true;
                Ok("regime_trending(false)".to_string())
            }
            MarketRegime::Ranging => {
                ctx.needs.ranging = true;
                Ok("regime_ranging()".to_string())
            }
            MarketRegime::LondonSession => Ok("(candle_hour() >= 8 && candle_hour() < 17)".to_string()),
            MarketRegime::UsSession => Ok("(candle_hour() >= 13 && candle_hour() < 22)".to_string()),
            MarketRegime::AsianSession => Ok("(candle_hour() < 9)".to_string()),
            MarketRegime::HighVolatility | MarketRegime::LowVolatility => {
                ctx.needs.volatility = true;
                let atr = ctx.ensure_indicator(IndicatorType::Atr, Some(14.0), "regime_atr");
                Ok(if t.regime == MarketRegime::HighVolatility {
                    format!("(indicator(\"{atr}\", \"value\") > 0.0075)")
                } else {
                    format!("(indicator(\"{atr}\", \"value\") > 0.0 && indicator(\"{atr}\", \"value\") < 0.0025)")
                })
            }
            other => Err(format!(
                "givens regime {:?} is rules-engine-only (S/R zones / price action / divergence have no scripted-ABI equivalent)",
                other
            )),
        },
        Trigger::RiskReward(t) => {
            if !in_exit {
                return Err("risk_reward_reached used outside an exit rule".to_string());
            }
            ctx.needs.rr = true;
            Ok(format!("rr_profit() >= rr_risk() * ({})", num_expr(&t.ratio)))
        }
        Trigger::PercentOfTp(t) => {
            if !in_exit {
                return Err("percent_of_tp_reached used outside an exit rule".to_string());
            }
            ctx.needs.rr = true;
            ctx.needs.tp_pct = true;
            Ok(format!("tp_progress_pct() >= ({})", num_expr(&t.percent)))
        }
        Trigger::Time(t) => {
            if !in_exit {
                return Err("time trigger used outside an exit rule".to_string());
            }
            match t.condition {
                TimeCondition::BarCount => Ok(format!("bars_since_entry() >= ({})", num_expr(&t.value))),
                TimeCondition::Minutes | TimeCondition::Hours => {
                    let secs_per_unit = if t.condition == TimeCondition::Minutes { 60.0 } else { 3600.0 };
                    let v = t.value.as_fixed().ok_or_else(|| {
                        "parameterized minutes/hours time triggers are not supported".to_string()
                    })?;
                    ctx.needs.entry_time = true;
                    Ok(format!(
                        "(sig_time > 0 && candle_time() - sig_time >= {})",
                        (v * secs_per_unit) as i64
                    ))
                }
            }
        }
        Trigger::TimeInRange(t) => {
            if t.start_minute != 0 || t.end_minute != 0 {
                return Err("time_in_range with sub-hour bounds is not supported (candle_hour() is hour-granular)".to_string());
            }
            let (s, e) = (t.start_hour, t.end_hour);
            Ok(if s <= e {
                format!("(candle_hour() >= {s} && candle_hour() < {e})")
            } else {
                format!("(candle_hour() >= {s} || candle_hour() < {e})")
            })
        }
        Trigger::DayOfWeek(_) => Err("day_of_week triggers are not supported (no weekday accessor in the scripted ABI)".to_string()),
    }
}

/// A `DataSource` at `extra_offset` bars back (trigger lookback), rejecting
/// rules-engine-only sources.
fn source_expr(ctx: &mut Ctx, src: &DataSource, extra_offset: usize) -> Result<String, ConvertError> {
    match src {
        DataSource::Indicator(s) => {
            if !plain_indicator(s) {
                return Err(format!(
                    "indicator source '{}' uses capture/trail/timeframe/symbol options the scripted ABI cannot express",
                    s.indicator
                ));
            }
            if !ctx.def.indicators.iter().any(|c| c.id == s.indicator) {
                return Err(format!("indicator source references undeclared indicator '{}'", s.indicator));
            }
            let off = s.offset + extra_offset;
            Ok(if off == 0 {
                format!("indicator(\"{}\", \"{}\")", s.indicator, s.output)
            } else {
                format!("indicator_at(\"{}\", \"{}\", {})", s.indicator, s.output, off)
            })
        }
        DataSource::Price(s) => {
            if s.capture != shared::CaptureMode::EachCandle
                || s.trail.as_ref().map_or(false, |t| t.enabled)
                || s.timeframe.as_deref().map_or(false, |t| !t.is_empty())
            {
                return Err("price source uses capture/trail/timeframe options the scripted ABI cannot express".to_string());
            }
            let off = s.offset + extra_offset;
            Ok(if off == 0 {
                format!("price(\"{}\")", s.value.as_str())
            } else {
                format!("price_at(\"{}\", {})", s.value.as_str(), off)
            })
        }
        DataSource::Fixed(s) => Ok(num_literal(s.fixed)),
        DataSource::Numeric(v) => Ok(num_literal(*v)),
        DataSource::Parameter(p) => Ok(format!("param(\"{}\")", p.param_id)),
        DataSource::Variable(_) => Err("variable data sources are not supported".to_string()),
        DataSource::SRZone(_) => Err("S/R zone data sources are rules-engine-only".to_string()),
        DataSource::Pivot(_) => Err("pivot data sources are rules-engine-only".to_string()),
        DataSource::Pattern(_) => Err("candlestick-pattern data sources are rules-engine-only".to_string()),
    }
}

fn plain_indicator(s: &shared::IndicatorSource) -> bool {
    s.capture == shared::CaptureMode::EachCandle
        && !s.trail.as_ref().map_or(false, |t| t.enabled)
        && s.timeframe.as_deref().map_or(true, |t| t.is_empty())
        && s.symbol.as_deref().map_or(true, |t| t.is_empty())
}

/// Stop-loss expression per `risk_settings` (mirrors `calculate_stop_loss`).
fn stop_loss_expr(ctx: &mut Ctx, direction: RuleDirection) -> Result<String, ConvertError> {
    let rs = &ctx.def.risk_settings;
    let source = match direction {
        RuleDirection::Short => rs.stop_loss_source_short.as_ref().or(rs.stop_loss_source.as_ref()),
        _ => rs.stop_loss_source.as_ref(),
    };
    let sign = if direction == RuleDirection::Short { "+" } else { "-" };
    Ok(match source {
        Some(StopLossSource::Indicator { indicator, output, .. }) => {
            if !ctx.def.indicators.iter().any(|c| c.id == *indicator) {
                return Err(format!("stop_loss_source references undeclared indicator '{indicator}'"));
            }
            format!("indicator(\"{indicator}\", \"{output}\")")
        }
        Some(StopLossSource::FixedPips { pips }) => {
            format!("entry {sign} ({}) * pip_value()", num_expr(pips))
        }
        Some(StopLossSource::Percent { percent }) => {
            let op = if direction == RuleDirection::Short { "1.0 +" } else { "1.0 -" };
            format!("entry * ({op} ({}) / 100.0)", num_expr(percent))
        }
        Some(StopLossSource::Variable { variable, .. }) => {
            return Err(format!("stop_loss_source variable '{variable}' is not supported"));
        }
        // Rules-engine default: 2% of entry.
        None => {
            let op = if direction == RuleDirection::Short { "1.02" } else { "0.98" };
            format!("entry * {op}")
        }
    })
}

fn rr_ratio_expr(ctx: &Ctx, direction: RuleDirection) -> Result<String, ConvertError> {
    let rs = &ctx.def.risk_settings;
    let v = match direction {
        RuleDirection::Short => rs.rr_ratio_short.as_ref().unwrap_or(&rs.rr_ratio),
        _ => &rs.rr_ratio,
    };
    Ok(num_expr(v))
}

// ============================================================================
// Small helpers
// ============================================================================

fn cmp_op(op: ComparisonOperator) -> Result<&'static str, ConvertError> {
    Ok(match op {
        ComparisonOperator::GreaterThan => ">",
        ComparisonOperator::GreaterThanOrEqual => ">=",
        ComparisonOperator::LessThan => "<",
        ComparisonOperator::LessThanOrEqual => "<=",
        ComparisonOperator::Equal => "==",
        ComparisonOperator::IsWithin => return Err("is_within is only valid on compare triggers".to_string()),
    })
}

fn fixed_lookback(v: &ParameterizedValue) -> Result<usize, ConvertError> {
    match v.as_fixed() {
        Some(f) if f >= 1.0 && f <= 24.0 => Ok(f as usize),
        Some(f) if f < 1.0 => Ok(1),
        Some(f) => Err(format!("lookback {f} is unreasonably large for expansion")),
        None => Err("parameterized trigger lookback is not supported".to_string()),
    }
}

fn join_or(alts: Vec<String>) -> String {
    if alts.len() == 1 {
        alts.into_iter().next().unwrap()
    } else {
        format!("({})", alts.join(" || "))
    }
}

/// A `ParameterizedValue` as a Rhai expression (`param("id")` or a literal).
fn num_expr(v: &ParameterizedValue) -> String {
    match v {
        ParameterizedValue::Fixed(f) => num_literal(*f),
        ParameterizedValue::Reference(r) => format!("param(\"{}\")", r.param_id),
    }
}

/// Numeric literal in the no_float dialect: always decimal-formed so it
/// parses as Decimal (`5` → `5.0`).
fn num_literal(v: f64) -> String {
    let s = format!("{v}");
    if s.contains('.') || s.contains('e') {
        s
    } else {
        format!("{s}.0")
    }
}

fn escape_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_def(entry_rules: Value, exit_rules: Value, indicators: Value) -> StrategyDefinition {
        serde_json::from_value(json!({
            "id": "test-1",
            "user_id": "u",
            "name": "Test Strategy",
            "description": "converter fixture",
            "parameters": [
                { "id": "rsi_oversold", "name": "RSI oversold", "description": null, "type": "number",
                  "default": 30.0, "min": 10.0, "max": 50.0, "step": 1.0, "options": null, "group": null }
            ],
            "indicators": indicators,
            "variables": [],
            "entry_rules": entry_rules,
            "entry_logic": { "mode": "all", "min_score": null },
            "exit_rules": exit_rules,
            "risk_settings": {
                "risk_method": "percent",
                "risk_value": 1.0,
                "rr_ratio": 2.0,
                "spread_buffer_pips": 1.0
            },
            "version": 1,
            "is_active": true,
            "schema_version": 2,
            "strategy_type": "rules",
            "script_content": null
        }))
        .unwrap()
    }

    fn rsi_indicator() -> Value {
        json!([{ "id": "rsi", "type": "rsi", "params": { "period": 14.0 }, "symbol": null, "timeframe": null }])
    }

    /// R1 + R5 + R6: threshold entry with param reference, RR + bar-count exits.
    #[test]
    fn rr_exit_strategy_converts_and_validates() {
        let def = base_def(
            json!([{
                "id": "e1", "name": "RSI dip", "direction": "long",
                "conditions": [{
                    "primary": { "trigger": { "type": "threshold", "source": { "indicator": "rsi", "output": "value", "offset": 0, "symbol": null, "timeframe": null }, "operator": "<", "value": { "$param": "rsi_oversold" } }, "negated": false },
                    "chain": []
                }]
            }]),
            json!([
                { "id": "x1", "name": "RR hit", "direction": "long", "conditions": [{ "primary": { "trigger": { "type": "risk_reward_reached", "ratio": 2.0 }, "negated": false }, "chain": [] }], "close_percent": 100.0, "priority": 1 },
                { "id": "x2", "name": "Stale", "direction": "both", "conditions": [{ "primary": { "trigger": { "type": "time", "condition": "bar_count", "value": 10.0 }, "negated": false }, "chain": [] }], "close_percent": 100.0, "priority": 0 }
            ]),
            rsi_indicator(),
        );
        let script = convert_rules(&def).unwrap();
        validate_script(&script).unwrap_or_else(|e| panic!("generated script invalid: {e}\n{script}"));
        assert!(script.contains("param(\"rsi_oversold\")"), "{script}");
        assert!(script.contains("rr_profit() >= rr_risk() * (2.0)"), "{script}");
        assert!(script.contains("bars_since_entry() >= (10.0)"), "{script}");
        assert!(script.contains("in_position()"), "{script}");
        // RR exit (priority 1) is evaluated before the bar-count exit (priority 0)
        let rr_pos = script.find("RR hit").unwrap();
        let stale_pos = script.find("Stale").unwrap();
        assert!(rr_pos < stale_pos, "exit priority order violated\n{script}");
    }

    /// R2: price↔indicator cross expands to the two-comparison form; an
    /// indicator↔indicator cross uses the native SDK detector.
    #[test]
    fn cross_recipes() {
        let def = base_def(
            json!([{
                "id": "e1", "name": "price cross", "direction": "long",
                "conditions": [
                    { "primary": { "trigger": { "type": "cross", "left": { "source": "price", "value": "close", "offset": 0, "symbol": null, "timeframe": null }, "right": { "indicator": "ema_fast", "output": "value", "offset": 0, "symbol": null, "timeframe": null }, "direction": "above", "lookback": 1.0 }, "negated": false }, "chain": [] },
                    { "primary": { "trigger": { "type": "cross", "left": { "indicator": "ema_fast", "output": "value", "offset": 0, "symbol": null, "timeframe": null }, "right": { "indicator": "ema_slow", "output": "value", "offset": 0, "symbol": null, "timeframe": null }, "direction": "above", "lookback": 1.0 }, "negated": false }, "chain": [] }
                ]
            }]),
            json!([]),
            json!([
                { "id": "ema_fast", "type": "ema", "params": { "period": 10.0 }, "symbol": null, "timeframe": null },
                { "id": "ema_slow", "type": "ema", "params": { "period": 30.0 }, "symbol": null, "timeframe": null }
            ]),
        );
        let script = convert_rules(&def).unwrap();
        validate_script(&script).unwrap_or_else(|e| panic!("generated script invalid: {e}\n{script}"));
        assert!(script.contains("crossed_above(\"ema_fast\", \"value\", \"ema_slow\", \"value\")"), "{script}");
        assert!(script.contains("price_at(\"close\", 1)"), "{script}");
    }

    /// R3 + R4: session givens become candle_hour gates; regime givens
    /// auto-declare their indicator set and emit the helper.
    #[test]
    fn givens_recipes() {
        let def = base_def(
            json!([{
                "id": "e1", "name": "regime gated", "direction": "short",
                "conditions": [
                    { "primary": { "trigger": { "type": "givens", "regime": "trending_down" }, "negated": false }, "chain": [] },
                    { "primary": { "trigger": { "type": "givens", "regime": "london_session" }, "negated": false },
                      "chain": [{ "operator": "or", "trigger": { "trigger": { "type": "givens", "regime": "us_session" }, "negated": false } }] }
                ]
            }]),
            json!([]),
            rsi_indicator(),
        );
        let script = convert_rules(&def).unwrap();
        validate_script(&script).unwrap_or_else(|e| panic!("generated script invalid: {e}\n{script}"));
        assert!(script.contains("regime_trending(false)"), "{script}");
        assert!(script.contains("candle_hour() >= 8 && candle_hour() < 17"), "{script}");
        assert!(script.contains("\"regime_adx\""), "auto-declared regime indicators missing: {script}");
        assert!(script.contains("\"regime_sma20\""), "{script}");
        // signal is a sell with the short-side SL guard
        assert!(script.contains("signal: \"sell\""), "{script}");
        assert!(script.contains("if sl > entry"), "{script}");
    }

    /// Unsupported constructs are refused with a reason, not mistranslated.
    #[test]
    fn unsupported_constructs_are_named() {
        // MTF indicator → redesign
        let def = base_def(
            json!([{ "id": "e1", "name": null, "direction": "long", "conditions": [{ "primary": { "trigger": { "type": "givens", "regime": "trending_up" }, "negated": false }, "chain": [] }] }]),
            json!([]),
            json!([{ "id": "daily_ema", "type": "ema", "params": { "period": 20.0 }, "symbol": null, "timeframe": "D" }]),
        );
        let err = convert_rules(&def).unwrap_err();
        assert!(err.contains("multi-timeframe"), "{err}");

        // divergence givens → rules-engine-only
        let def = base_def(
            json!([{ "id": "e1", "name": null, "direction": "long", "conditions": [{ "primary": { "trigger": { "type": "givens", "regime": "at_demand_zone" }, "negated": false }, "chain": [] }] }]),
            json!([]),
            rsi_indicator(),
        );
        let err = convert_rules(&def).unwrap_err();
        assert!(err.contains("rules-engine-only"), "{err}");

        // partial close → unsupported
        let def = base_def(
            json!([{ "id": "e1", "name": null, "direction": "long", "conditions": [{ "primary": { "trigger": { "type": "threshold", "source": { "indicator": "rsi", "output": "value", "offset": 0, "symbol": null, "timeframe": null }, "operator": "<", "value": 30.0 }, "negated": false }, "chain": [] }] }]),
            json!([{ "id": "x1", "name": null, "direction": "long", "conditions": [{ "primary": { "trigger": { "type": "risk_reward_reached", "ratio": 1.0 }, "negated": false }, "chain": [] }], "close_percent": 50.0, "priority": 0 }]),
            rsi_indicator(),
        );
        let err = convert_rules(&def).unwrap_err();
        assert!(err.contains("partial closes"), "{err}");
    }

    /// convert_file end-to-end: array input, verbatim scripted passthrough,
    /// dedupe, and the empty-shell drop.
    #[test]
    fn convert_file_end_to_end() {
        let rules = serde_json::to_value(base_def(
            json!([{ "id": "e1", "name": "dip", "direction": "long", "conditions": [{ "primary": { "trigger": { "type": "threshold", "source": { "indicator": "rsi", "output": "value", "offset": 0, "symbol": null, "timeframe": null }, "operator": "<", "value": 30.0 }, "negated": false }, "chain": [] }] }]),
            json!([]),
            rsi_indicator(),
        ))
        .unwrap();
        let mut dup = rules.clone();
        dup["id"] = json!("test-2");
        dup["name"] = json!("Test Strategy Copy"); // same body → dedupe
        let scripted = json!({
            "id": "s-1", "user_id": "u", "name": "Verbatim", "description": "",
            "parameters": [], "indicators": [], "variables": [],
            "entry_rules": [], "entry_logic": { "mode": "all", "min_score": null }, "exit_rules": [],
            "risk_settings": { "risk_method": "percent", "risk_value": 1.0, "rr_ratio": 2.0, "spread_buffer_pips": 1.0 },
            "version": 1, "is_active": true, "schema_version": 2,
            "strategy_type": "scripted",
            "script_content": "fn on_candle() { \"hold\" }"
        });
        let empty = json!({
            "id": "empty-1", "user_id": "u", "name": "Empty Shell", "description": "",
            "parameters": [], "indicators": [], "variables": [],
            "entry_rules": [], "entry_logic": { "mode": "all", "min_score": null }, "exit_rules": [],
            "risk_settings": { "risk_method": "percent", "risk_value": 1.0, "rr_ratio": 2.0, "spread_buffer_pips": 1.0 },
            "version": 1, "is_active": true, "schema_version": 2,
            "strategy_type": "rules", "script_content": null
        });
        let payload = serde_json::to_string(&json!([rules, dup, scripted, empty])).unwrap();

        let dir = std::env::temp_dir().join(format!(
            "wickd-convert-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let report = convert_file(&payload, &dir, false).unwrap();
        let converted = report["converted"].as_array().unwrap();
        let skipped = report["skipped"].as_array().unwrap();
        assert_eq!(converted.len(), 2, "{report}");
        assert_eq!(skipped.len(), 2, "{report}");
        assert!(dir.join("test_strategy.rhai").is_file());
        assert!(dir.join("verbatim.rhai").is_file());
        assert!(skipped.iter().any(|s| s["reason"].as_str().unwrap().contains("duplicate")), "{report}");
        assert!(skipped.iter().any(|s| s["reason"].as_str().unwrap().contains("empty shell")), "{report}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

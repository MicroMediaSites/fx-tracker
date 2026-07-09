//! Golden-script corpus round-trip (AGT-606, AC6).
//!
//! Every `.rhai` file under `tests/golden_scripts/` is a worked example referenced
//! by `STRATEGY_ABI.md` (the versioned authoring contract for Rhai strategy
//! scripts). This test walks the corpus and runs each one through
//! `validate_script` — the exact function `wickd strategy run`, `wickd backtest`,
//! and the live watcher all call before loading a script (see
//! `crates/wickd/src/commands/scripted.rs::load_scripted_strategy`) — so the
//! documented ABI can never silently drift from what the implementation actually
//! accepts.
//!
//! If you add or change a worked example in STRATEGY_ABI.md, add or update the
//! matching `.rhai` file here so this test keeps covering it.

use wickd_core::backtest::validate_script;

const GOLDEN_SCRIPTS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden_scripts");

#[test]
fn golden_script_corpus_all_validate() {
    let dir = std::fs::read_dir(GOLDEN_SCRIPTS_DIR).unwrap_or_else(|e| {
        panic!("failed to read golden script corpus dir {GOLDEN_SCRIPTS_DIR}: {e}")
    });

    let mut checked: Vec<String> = Vec::new();

    for entry in dir {
        let entry = entry.expect("readdir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rhai") {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unknown>")
            .to_string();

        let script = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

        let result = validate_script(&script);
        assert!(
            result.is_ok(),
            "golden script {} failed validate_script: {}",
            path.display(),
            result.unwrap_err()
        );

        checked.push(name);
    }

    assert!(
        !checked.is_empty(),
        "no .rhai files found in {GOLDEN_SCRIPTS_DIR} — the golden corpus is empty"
    );

    // Sanity check the corpus actually covers what STRATEGY_ABI.md documents:
    // a script with no metadata, one with @indicators + @parameters, and one
    // exercising the extended #{...} signal + on_position_closed() hook.
    assert!(checked.iter().any(|n| n.contains("minimal")), "missing a no-metadata example script");
    assert!(checked.iter().any(|n| n.contains("parameters")), "missing an @indicators/@parameters example script");
    assert!(checked.iter().any(|n| n.contains("risk")), "missing an extended-signal (stop_loss/take_profit) example script");
    assert!(checked.iter().any(|n| n.contains("surprise")), "missing a surprise-feed (ABI v4) example script");
}

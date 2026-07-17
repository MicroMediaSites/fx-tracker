//! Updater helper commands.
//!
//! Local builds keep tauri.conf.json's placeholder endpoint
//! (`https://localhost/updates/latest.json`); release.yml swaps in the real
//! GitHub Releases endpoint at release-build time (AGT-650). A local build
//! therefore can never self-update — the frontend uses this command to show
//! a "local build, self-update disabled" state instead of surfacing the raw
//! connection error from the doomed localhost request.

use tauri::Manager;

/// True when `updater_config` has no usable endpoint: missing config, an
/// empty endpoint list, or the local-build placeholder.
fn endpoints_are_placeholder(updater_config: Option<&serde_json::Value>) -> bool {
    updater_config
        .and_then(|updater| updater.get("endpoints"))
        .and_then(|endpoints| endpoints.as_array())
        .map(|endpoints| {
            endpoints.is_empty()
                || endpoints
                    .iter()
                    .any(|e| e.as_str().is_some_and(|s| s.contains("//localhost")))
        })
        .unwrap_or(true)
}

/// Whether this build's bundled updater endpoint is the local-build
/// placeholder — i.e. self-update cannot work in this binary.
#[tauri::command]
pub fn updater_is_placeholder(app: tauri::AppHandle) -> bool {
    let _ = &app; // Manager is only needed for config()
    endpoints_are_placeholder(app.config().plugins.0.get("updater"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn localhost_placeholder_is_detected() {
        // The verbatim local-build shape from tauri.conf.json.
        let cfg = json!({ "endpoints": ["https://localhost/updates/latest.json"] });
        assert!(endpoints_are_placeholder(Some(&cfg)));
    }

    #[test]
    fn release_endpoint_is_not_a_placeholder() {
        // The shape release.yml's sed produces.
        let cfg = json!({
            "endpoints": [
                "https://github.com/MicroMediaSites/fx-tracker/releases/latest/download/latest.json"
            ]
        });
        assert!(!endpoints_are_placeholder(Some(&cfg)));
    }

    #[test]
    fn missing_or_empty_config_counts_as_placeholder() {
        assert!(endpoints_are_placeholder(None));
        assert!(endpoints_are_placeholder(Some(&json!({}))));
        assert!(endpoints_are_placeholder(Some(&json!({ "endpoints": [] }))));
    }
}

//! AGT-629: the launchd supervision artifacts under `deploy/launchd/` must stay
//! valid and keep the properties the acceptance criteria depend on:
//!
//!   AC1 — a `wickd stream` hub job AND a parameterized per-strategy
//!         `wickd watch <script> <instruments> --auto --account <name>` job,
//!         both with `KeepAlive`/restart-on-crash and stdout/stderr to files.
//!   AC2 — install/uninstall scripts exist and validate before loading.
//!
//! These tests read the shipped templates (no launchctl, nothing loaded),
//! assert the load-bearing keys/args, and render the placeholders through
//! `plutil -lint` on macOS so a malformed template can't ship. They intentionally
//! never call `launchctl bootstrap`/`load` — actually loading the jobs is the
//! human launch step (AGT-633).

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

fn deploy_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("deploy/launchd")
}

fn read(name: &str) -> String {
    let path = deploy_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// Substitute every `__PLACEHOLDER__=value` pair, then assert no `__…__` token
/// is left unrendered (a typo'd placeholder would otherwise ship silently).
fn render(template: &str, subs: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in subs {
        out = out.replace(k, v);
    }
    assert!(
        !out.contains("__"),
        "rendered plist still contains an unsubstituted __PLACEHOLDER__:\n{out}"
    );
    out
}

/// Render → temp file → `plutil -lint`. macOS only (that's the target + where
/// plutil lives); a no-op elsewhere.
fn assert_plutil_valid(rendered: &str, tag: &str) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let dir = std::env::temp_dir();
    let file = dir.join(format!("wickd-{tag}-{}.plist", std::process::id()));
    std::fs::write(&file, rendered).expect("writing rendered plist");
    let out = Command::new("plutil")
        .arg("-lint")
        .arg(&file)
        .output()
        .expect("running plutil -lint");
    let _ = std::fs::remove_file(&file);
    assert!(
        out.status.success(),
        "plutil -lint failed for {tag}:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

// --- AC1: the two job templates ---------------------------------------------

#[test]
fn stream_template_supervises_the_hub_with_restart_and_logs() {
    let t = read("com.openthink.wickd-stream.plist");

    assert!(t.contains("<string>com.openthink.wickd-stream</string>"), "hub label");
    assert!(t.contains("<string>stream</string>"), "invokes `wickd stream`");
    // Restart on crash, not on a clean stop.
    assert!(t.contains("<key>KeepAlive</key>"), "KeepAlive present");
    assert!(t.contains("<key>SuccessfulExit</key>"), "KeepAlive.SuccessfulExit present");
    assert!(t.contains("<key>RunAtLoad</key>"), "RunAtLoad present");
    // stdout + stderr to files.
    assert!(t.contains("<key>StandardOutPath</key>"), "stdout log path");
    assert!(t.contains("<key>StandardErrorPath</key>"), "stderr log path");
    assert!(t.contains("stream.out.log") && t.contains("stream.err.log"), "hub log filenames");

    let rendered = render(
        &t,
        &[
            ("__WICKD_BIN__", "/usr/local/bin/wickd"),
            ("__HOME__", "/Users/tester"),
            ("__LOG_DIR__", "/Users/tester/Library/Logs/wickd"),
            ("__INSTRUMENTS__", "EUR_USD,GBP_USD"),
            ("__ENV__", "practice"),
            ("__ACCOUNT__", "h004"),
        ],
    );
    assert_plutil_valid(&rendered, "stream");
}

#[test]
fn watch_template_is_parameterized_auto_and_practice_only() {
    let t = read("com.openthink.wickd-watch.plist");

    // Parameterized per strategy: label + logs are placeholders, not fixed.
    assert!(t.contains("<string>__LABEL__</string>"), "per-strategy label placeholder");
    assert!(t.contains("watch.__SLUG__.out.log"), "per-slug stdout log");
    assert!(t.contains("watch.__SLUG__.err.log"), "per-slug stderr log");

    // The exact command shape AC1 names: watch <script> <instruments> --auto --account <name>.
    assert!(t.contains("<string>watch</string>"), "invokes `wickd watch`");
    assert!(t.contains("<string>__STRATEGY__</string>"), "strategy placeholder");
    assert!(t.contains("<string>__INSTRUMENTS__</string>"), "instruments placeholder");
    assert!(t.contains("<string>--auto</string>"), "autonomous execution armed");
    assert!(t.contains("<string>--account</string>"), "threads --account");
    assert!(t.contains("<string>__ACCOUNT__</string>"), "account placeholder");

    // Practice-only: `--env practice` is hardcoded, never a placeholder, and
    // the word `live` must not appear as an argument.
    assert!(t.contains("<string>--env</string>"), "threads --env");
    assert!(t.contains("<string>practice</string>"), "env hardcoded to practice");
    assert!(!t.contains("<string>live</string>"), "must never arm live in a supervised job");

    // Restart on crash + logs (same guarantees as the hub).
    assert!(t.contains("<key>KeepAlive</key>"), "KeepAlive present");
    assert!(t.contains("<key>SuccessfulExit</key>"), "KeepAlive.SuccessfulExit present");
    assert!(t.contains("<key>RunAtLoad</key>"), "RunAtLoad present");
    assert!(t.contains("<key>StandardOutPath</key>"), "stdout log path");
    assert!(t.contains("<key>StandardErrorPath</key>"), "stderr log path");

    let rendered = render(
        &t,
        &[
            ("__WICKD_BIN__", "/usr/local/bin/wickd"),
            ("__HOME__", "/Users/tester"),
            ("__LOG_DIR__", "/Users/tester/Library/Logs/wickd"),
            ("__LABEL__", "com.openthink.wickd-watch.rsi-h004"),
            ("__SLUG__", "rsi-h004"),
            ("__STRATEGY__", "rsi"),
            ("__INSTRUMENTS__", "EUR_USD,GBP_USD"),
            ("__GRANULARITY__", "H1"),
            ("__ACCOUNT__", "h004"),
            ("__UNITS__", "1000"),
        ],
    );
    // The rendered job carries a concrete --auto + practice invocation.
    assert!(rendered.contains("com.openthink.wickd-watch.rsi-h004"), "label rendered");
    assert!(rendered.contains("watch.rsi-h004.out.log"), "log rendered");
    assert_plutil_valid(&rendered, "watch");
}

// --- AC2: install / uninstall scripts ---------------------------------------

#[test]
fn install_script_dispatches_both_jobs_and_validates_before_loading() {
    let s = read("install.sh");
    // Dispatches both job kinds.
    assert!(s.contains("install_stream"), "has a stream installer");
    assert!(s.contains("install_watch"), "has a watch installer");
    // Watch installs wire --auto through and a practice env (AC1 command shape).
    assert!(s.contains("--auto"), "watch job is armed for autonomous execution");
    // Validates the rendered plist before it is copied/loaded.
    assert!(s.contains("plutil -lint"), "validates rendered plist");
    // Uses the modern bootstrap API with a legacy load fallback.
    assert!(s.contains("launchctl bootstrap"), "modern load path");
    assert!(s.contains("launchctl load"), "legacy load fallback");
    // A dry-run path exists so the artifacts can be validated without loading.
    assert!(s.contains("dry-run"), "supports --dry-run (validate without loading)");

    // Executable bit set (so `./install.sh …` works as documented).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(deploy_dir().join("install.sh")).unwrap().permissions().mode();
        assert!(mode & 0o111 != 0, "install.sh is not executable (mode {mode:o})");
    }
}

#[test]
fn uninstall_script_handles_stream_watch_and_all() {
    let s = read("uninstall.sh");
    assert!(s.contains("com.openthink.wickd-stream"), "can remove the hub");
    assert!(s.contains("com.openthink.wickd-watch"), "can remove a watcher");
    assert!(s.contains("--all"), "can remove every wickd job");
    assert!(s.contains("--purge-logs"), "can optionally purge logs");
    // Stops via the modern bootout with a legacy unload fallback.
    assert!(s.contains("launchctl bootout"), "modern stop path");
    assert!(s.contains("launchctl unload"), "legacy stop fallback");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(deploy_dir().join("uninstall.sh")).unwrap().permissions().mode();
        assert!(mode & 0o111 != 0, "uninstall.sh is not executable (mode {mode:o})");
    }
}

// --- Candle watchdog: the external liveness check ----------------------------

#[test]
fn watchdog_template_is_a_readonly_periodic_oneshot() {
    let t = read("com.openthink.wickd-watchdog.plist");

    assert!(
        t.contains("<string>com.openthink.wickd-watchdog</string>"),
        "watchdog label"
    );
    assert!(t.contains("<string>/usr/bin/python3</string>"), "runs via system python3");
    assert!(t.contains("<string>__SCRIPT__</string>"), "script path placeholder");
    assert!(t.contains("<string>--grace</string>"), "threads --grace");
    assert!(t.contains("<string>--realert</string>"), "threads --realert");

    // Periodic one-shot like the books collector: interval + run-at-load,
    // and NO KeepAlive — exit 1 (problems found) is normal operation, the
    // next StartInterval firing is the retry.
    assert!(t.contains("<key>StartInterval</key>"), "fires on an interval");
    assert!(t.contains("<key>RunAtLoad</key>"), "checks once at login");
    assert!(!t.contains("<key>KeepAlive</key>"), "one-shot must not be kept alive");

    assert!(t.contains("watchdog.out.log") && t.contains("watchdog.err.log"), "log filenames");

    let rendered = render(
        &t,
        &[
            (
                "__SCRIPT__",
                "/Users/tester/Library/Application Support/wickd-watchdog/wickd-candle-watchdog.py",
            ),
            ("__HOME__", "/Users/tester"),
            ("__LOG_DIR__", "/Users/tester/Library/Logs/wickd"),
            ("__INTERVAL__", "300"),
            ("__GRACE__", "1200"),
            ("__REALERT__", "3600"),
        ],
    );
    assert_plutil_valid(&rendered, "watchdog");
}

#[test]
fn install_and_uninstall_scripts_cover_the_watchdog() {
    let i = read("install.sh");
    assert!(i.contains("install_watchdog"), "install.sh has a watchdog installer");
    assert!(i.contains("wickd-candle-watchdog.py"), "install.sh copies the script");

    let u = read("uninstall.sh");
    assert!(u.contains("com.openthink.wickd-watchdog"), "uninstall.sh removes the watchdog");

    // The watchdog script itself ships in the deploy dir and is executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(deploy_dir().join("wickd-candle-watchdog.py"))
            .unwrap()
            .permissions()
            .mode();
        assert!(mode & 0o111 != 0, "wickd-candle-watchdog.py is not executable (mode {mode:o})");
    }
}

#[test]
fn feed_template_is_a_subscription_authed_periodic_oneshot() {
    let t = read("com.openthink.wickd-feed.plist");

    assert!(
        t.contains("<string>com.openthink.wickd-feed</string>"),
        "feed label"
    );
    assert!(t.contains("<string>feed</string>") && t.contains("<string>tick</string>"), "runs wickd feed tick");
    assert!(t.contains("<string>--model</string>") && t.contains("<string>__MODEL__</string>"), "threads --model");

    // Periodic one-shot like calendar/books: interval + run-at-load, NO
    // KeepAlive — a failed tick just waits for the next firing (and the
    // command's own feed.lock makes an overlapping fire a clean no-op).
    assert!(t.contains("<key>StartInterval</key>"), "fires on an interval");
    assert!(t.contains("<key>RunAtLoad</key>"), "ticks once at login");
    assert!(!t.contains("<key>KeepAlive</key>"), "one-shot must not be kept alive");

    // Subscription auth plumbing: claude's directory joins PATH and the
    // Claude Code account is pinned via CLAUDE_CONFIG_DIR.
    assert!(t.contains("__CLAUDE_DIR__:"), "claude binary dir leads PATH");
    assert!(t.contains("<key>CLAUDE_CONFIG_DIR</key>"), "account pinned via CLAUDE_CONFIG_DIR");

    assert!(t.contains("feed.out.log") && t.contains("feed.err.log"), "log filenames");

    let rendered = render(
        &t,
        &[
            ("__WICKD_BIN__", "/usr/local/bin/wickd"),
            ("__HOME__", "/Users/tester"),
            ("__LOG_DIR__", "/Users/tester/Library/Logs/wickd"),
            ("__INTERVAL__", "900"),
            ("__MODEL__", "sonnet"),
            ("__CLAUDE_DIR__", "/Users/tester/.local/bin"),
            ("__CLAUDE_CONFIG_DIR__", "/Users/tester/.claude"),
        ],
    );
    assert_plutil_valid(&rendered, "feed");
}

#[test]
fn install_and_uninstall_scripts_cover_the_feed_producer() {
    let i = read("install.sh");
    assert!(i.contains("install_feed"), "install.sh has a feed installer");
    assert!(i.contains("__CLAUDE_CONFIG_DIR__="), "install.sh renders the claude config dir");

    let u = read("uninstall.sh");
    assert!(u.contains("com.openthink.wickd-feed"), "uninstall.sh removes the feed producer");
}

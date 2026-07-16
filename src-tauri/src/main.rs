// wickd Desktop Application
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewWindowBuilder};
use tauri::window::Color;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tokio::sync::Mutex;
use tracing::info;
use candlesight_lib::{Config, CompileTimeConfig, ClaudeClient, focus_watcher_window_on_activation, send_test_notification, set_notifications_enabled, hub_stream::HubStreamState, oanda::{OandaClient, PriceStreamer}, crypto::{self, CredentialVault, RateLimiter, DeviceManager}};

mod commands;
mod tray;

use commands::window::{
    LOCALHOST_PORT, create_webview_url, disable_magnification,
    open_backtest_window, open_backtest_window_with_strategy, open_chart_window,
    open_startup_windows,
};
use commands::credentials::{
    check_password_strength_local, check_password_strength,
    get_device_id, check_unlock_rate_limit, get_rate_limit_status,
    encrypt_credentials, unlock_vault, lock_vault, is_vault_unlocked,
    has_practice_credentials, has_live_credentials, clear_live_credentials,
    add_live_credentials, update_api_key_with_vault, update_api_key,
    validate_account_id,
};
use commands::streaming::{
    subscribe_to_prices, unsubscribe_from_prices, hub_stream_status,
};
use commands::spread_stats::get_spread_stats;
use commands::oanda::{
    switch_oanda_environment, get_oanda_environment,
    get_oanda_credentials, save_oanda_credentials,
};
use commands::data::{
    fetch_instruments, calculate_pivot_points,
    sync_trades, get_candles, get_indicator_data,
};
use commands::backtest::{
    run_backtest, run_custom_backtest, run_backtest_debug,
    optimize_strategy, run_walk_forward, cancel_walk_forward,
    is_walk_forward_running, run_parameter_sweep, validate_strategy_json,
    convert_strategy_script,
};
use commands::daemon::{daemon_status, daemon_queue_list, daemon_pending_list, start_watcher, stop_watcher};
use commands::chat::{
    chat_stream, chat_cancel, is_chat_enabled, check_ai_model, create_chat_compaction,
    recover_strategy_error, ChatSessionState,
};
use commands::local_store::{
    LocalStoreState, local_store_path, local_list_strategies, local_get_strategy,
    local_save_strategy, local_delete_strategy,
    local_list_sr_zones, local_save_sr_zone, local_delete_sr_zone, local_clear_sr_zones,
    local_list_notes, local_save_note, local_delete_note,
    local_get_chart_config, local_set_chart_config,
    local_list_trades, local_list_closed_trades_by_instrument,
    local_list_trade_scores, local_get_trade_score_by_trade, local_save_trade_score,
    local_list_backtests, local_save_backtest, local_delete_backtests_for_strategy,
    local_list_backtest_jobs, local_get_backtest_job, local_save_backtest_job,
    local_record_promotion,
    local_list_labels, local_save_label,
    local_list_trade_labels, local_add_trade_label, local_delete_trade_label,
    local_list_strategy_labels, local_add_strategy_label, local_delete_strategy_label,
    local_list_strategy_trades, local_save_strategy_trade,
    local_list_strategy_watchers, local_save_strategy_watcher, local_delete_strategy_watcher,
    local_get_credential, local_save_credential, local_delete_credentials,
};
use commands::strategy_store::{store_list_strategies, store_read_strategy};

pub struct AppState {
    pub client: Arc<tokio::sync::RwLock<OandaClient>>,
    pub config: Arc<candlesight_lib::Config>,
    pub streamer: Arc<Mutex<PriceStreamer>>,
    pub claude: Option<ClaudeClient>,
    // Crypto state for credential management
    pub device_manager: Arc<DeviceManager>,
    pub rate_limiter: Arc<Mutex<RateLimiter>>,
    pub credential_vault: Arc<Mutex<Option<CredentialVault>>>,
    /// Walk-forward cancellation token
    pub wf_cancel_token: Arc<AtomicBool>,
    /// Whether a walk-forward job is currently running (BUG-028: prevents duplicate jobs)
    pub wf_running: Arc<AtomicBool>,
}

// Types and commands moved to specialized modules:
// - commands/trading.rs: Account, Position, Order, HistoricalTrade, OrderConfirmation, CloseConfirmation
// - commands/streaming.rs: subscribe_to_prices, unsubscribe_from_prices, etc.
// - commands/data.rs: CandleData, sync_trades, get_candles, get_indicator_data, etc.
// - commands/backtest.rs: BacktestResultData, TradeData, run_backtest, optimize_strategy, etc.
// - commands/analysis.rs: All analysis and AI commands

// ============================================================================
// Window Management Commands
// ============================================================================

/// Send a test notification to verify notifications are working
#[tauri::command]
fn test_notification() -> bool {
    send_test_notification()
}

/// Enable or disable desktop notifications globally
#[tauri::command]
fn set_desktop_notifications_enabled(enabled: bool) {
    set_notifications_enabled(enabled)
}

#[tokio::main]
async fn main() {
    // Initialize logging with info level
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // Load config with compile-time fallbacks for CI builds
    // Priority: runtime .env > compile-time option_env!()
    // Note: ANTHROPIC_API_KEY is NEVER embedded at compile time for security
    // (AGT-657) — it resolves solely at runtime from the encrypted vault or
    // process env. Local dev can use a .env file with ANTHROPIC_API_KEY.
    let config = Arc::new(Config::load_with_compile_time(CompileTimeConfig {
        build_mode: option_env!("BUILD_MODE"),
    }).expect("Failed to load config"));
    let client = Arc::new(tokio::sync::RwLock::new(
        OandaClient::new(&config).expect("Failed to create OANDA client")
    ));
    // Streamer for the degrade-to-direct path (AGT-652): only engaged for
    // instruments the wickd stream hub does not cover, or when the app hosts
    // the hub itself. Credentials update after vault unlock, as before.
    let api_key = config.api_key.clone().unwrap_or_default();
    let account_id = config.account_id.clone().unwrap_or_default();
    let streamer = PriceStreamer::new(&api_key, &account_id, &config.environment);

    // Initialize Claude client if API key is configured
    // User selects model directly (Haiku/Sonnet/Opus) in settings
    let claude = if let Some(ref api_key) = config.anthropic_api_key {
        // Security: never log any part of the API key (prefix/suffix/length).
        match ClaudeClient::new(api_key.clone()) {
            Ok(client) => {
                info!("Claude client initialized (models: Haiku/Sonnet/Opus available)");
                Some(client)
            }
            Err(e) => {
                eprintln!("Warning: Failed to initialize Claude client: {}. AI features disabled.", e);
                None
            }
        }
    } else {
        info!("ANTHROPIC_API_KEY not set. AI features disabled.");
        None
    };


    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::SIZE,
                )
                .build(),
        )
        .plugin(tauri_plugin_localhost::Builder::new(LOCALHOST_PORT).build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init());

    // Windows-only: single instance plugin so a second launch focuses the app
    #[cfg(target_os = "windows")]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.set_focus();
        }
    }));

    builder.setup(move |app| {
            // In release mode, navigate the default window to localhost so all
            // windows share one http://localhost origin (localStorage etc.).
            // AGT-642: the boot window is now the local-first "main" window
            // (?window=local) — no Clerk/Zero on this path.
            #[cfg(not(debug_assertions))]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let url: tauri::Url = format!("http://localhost:{}/index.html?window=local", LOCALHOST_PORT)
                        .parse()
                        .expect("Failed to parse localhost URL for initial window navigation");
                    let _ = window.navigate(url);
                }
            }

            // On Windows, resize the initial window to account for scrollbar space
            // and different font rendering. The tauri.conf.json sizes are optimized for macOS.
            #[cfg(target_os = "windows")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    use tauri::LogicalSize;
                    // Add 20px width and 30px height for Windows WebView2 differences
                    let _ = window.set_size(LogicalSize::new(980.0, 670.0));
                }
            }

            // BUG-040: Disable pinch-to-zoom on macOS by setting allowsMagnification = false
            #[cfg(target_os = "macos")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.with_webview(|webview| {
                        use objc::{msg_send, sel, sel_impl, runtime::Object};
                        unsafe {
                            let wkwebview: *mut Object = webview.inner().cast();
                            let _: () = msg_send![wkwebview, setAllowsMagnification: false];
                        }
                    });
                }
            }

            // =================================================================
            // macOS Menu Bar
            // =================================================================

            // App Menu (wickd) - macOS convention for first menu
            let app_menu = Submenu::with_items(
                app,
                "wickd",
                true,
                &[
                    &PredefinedMenuItem::about(app, Some("About wickd"), None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::services(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::hide(app, None)?,
                    &PredefinedMenuItem::hide_others(app, None)?,
                    &PredefinedMenuItem::show_all(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::quit(app, None)?,
                ],
            )?;

            // File Menu
            let file_menu = Submenu::with_items(
                app,
                "File",
                true,
                &[
                    &PredefinedMenuItem::close_window(app, None)?,
                ],
            )?;

            // Edit Menu - standard copy/paste (required for text input on macOS)
            let edit_menu = Submenu::with_items(
                app,
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::undo(app, None)?,
                    &PredefinedMenuItem::redo(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::cut(app, None)?,
                    &PredefinedMenuItem::copy(app, None)?,
                    &PredefinedMenuItem::paste(app, None)?,
                    &PredefinedMenuItem::select_all(app, None)?,
                ],
            )?;

            // View Menu - Application windows. Every window is reachable here,
            // keyed Cmd+1..4 in menu order (home, live monitor, research, chart).
            let home_item = MenuItem::with_id(app, "home", "Home", true, Some("CmdOrCtrl+1"))?;
            let watcher_item = MenuItem::with_id(app, "watcher", "Live Monitor", true, Some("CmdOrCtrl+2"))?;
            let backtest_item = MenuItem::with_id(app, "backtest", "Research", true, Some("CmdOrCtrl+3"))?;
            let chart_item = MenuItem::with_id(app, "chart", "Chart", true, Some("CmdOrCtrl+4"))?;
            let check_updates_item = MenuItem::with_id(app, "check_updates", "Check for Updates...", true, None::<&str>)?;

            let view_menu = Submenu::with_items(
                app,
                "View",
                true,
                &[
                    &home_item,
                    &watcher_item,
                    &backtest_item,
                    &chart_item,
                    &PredefinedMenuItem::separator(app)?,
                    &check_updates_item,
                ],
            )?;

            // Window Menu - standard macOS window management
            let window_menu = Submenu::with_items(
                app,
                "Window",
                true,
                &[
                    &PredefinedMenuItem::minimize(app, None)?,
                    &PredefinedMenuItem::maximize(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::fullscreen(app, None)?,
                ],
            )?;

            // Build complete menu bar
            let menu = Menu::with_items(app, &[&app_menu, &file_menu, &edit_menu, &view_menu, &window_menu])?;
            app.set_menu(menu)?;

            // Handle custom menu events
            app.on_menu_event(move |app_handle, event| {
                let handle = app_handle.clone();
                let menu_id = event.id().0.as_str();

                match menu_id {
                    "home" => {
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = open_home_window_internal(&handle).await {
                                eprintln!("Failed to open home window: {}", e);
                            }
                        });
                    }
                    "chart" => {
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = open_chart_window_internal(&handle).await {
                                eprintln!("Failed to open chart window: {}", e);
                            }
                        });
                    }
                    "watcher" => {
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = open_watcher_window_internal(&handle).await {
                                eprintln!("Failed to open watcher window: {}", e);
                            }
                        });
                    }
                    "backtest" => {
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) = open_backtest_window_internal(&handle).await {
                                eprintln!("Failed to open backtest window: {}", e);
                            }
                        });
                    }
                    "check_updates" => {
                        // Emit event to frontend to trigger update check
                        let _ = handle.emit("check-for-updates", ());
                    }
                    _ => {}
                }
            });

            // =================================================================
            // Hub-first price streaming (AGT-652)
            // =================================================================
            // The app is a client of the wickd stream hub: attach to a running
            // `wickd stream`, degrade uncovered instruments to a direct OANDA
            // subscription, or host the hub when nothing else is streaming.
            {
                let app_state: tauri::State<AppState> = app.state();
                let hub_state: tauri::State<HubStreamState> = app.state();
                hub_state.start(
                    app.handle().clone(),
                    candlesight_lib::hub_stream::StreamerDirectPort {
                        app: app.handle().clone(),
                        streamer: app_state.streamer.clone(),
                    },
                );
            }

            // =================================================================
            // System Tray - Always enabled for all users
            // =================================================================
            if let Err(e) = tray::setup_tray(app.handle()) {
                eprintln!("Failed to setup system tray: {}", e);
            }

            // =================================================================
            Ok(())
        })
        .manage({
            // Initialize crypto infrastructure
            let app_data_dir = crypto::get_default_app_data_dir()
                .expect("Failed to get app data directory");
            let device_manager = Arc::new(DeviceManager::new(app_data_dir.clone()));
            let rate_limiter = Arc::new(Mutex::new(
                RateLimiter::new(Some(device_manager.rate_limit_path()))
            ));

            AppState {
                client,
                config,
                streamer: Arc::new(Mutex::new(streamer)),
                claude,
                device_manager,
                rate_limiter,
                credential_vault: Arc::new(Mutex::new(None)),
                wf_cancel_token: Arc::new(AtomicBool::new(false)),
                wf_running: Arc::new(AtomicBool::new(false)),
            }
        })
        .manage(HubStreamState::default())
        .manage(ChatSessionState::default())
        .manage(LocalStoreState::default())
        .invoke_handler(tauri::generate_handler![
            fetch_instruments,
            calculate_pivot_points,
            // Hub-first price streaming (AGT-652)
            subscribe_to_prices,
            unsubscribe_from_prices,
            hub_stream_status,
            // Historical spread stats for the spread-bar coloring (read-only,
            // sampled by the wickd CLI into ~/.wickd/spreads.db)
            get_spread_stats,
            // Watch-daemon client surface (AGT-652)
            daemon_status,
            daemon_queue_list,
            daemon_pending_list,
            start_watcher,
            stop_watcher,
            sync_trades,
            get_candles,
            get_indicator_data,
            run_backtest,
            run_custom_backtest,
            run_backtest_debug,
            optimize_strategy,
            run_walk_forward,
            cancel_walk_forward,
            is_walk_forward_running,
            run_parameter_sweep,
            validate_strategy_json,
            convert_strategy_script,
            open_backtest_window,
            open_backtest_window_with_strategy,
            open_chart_window,
            // Chat streaming commands
            chat_stream,
            chat_cancel,
            is_chat_enabled,
            check_ai_model,
            recover_strategy_error,
            create_chat_compaction,
            switch_oanda_environment,
            get_oanda_environment,
            get_oanda_credentials,
            save_oanda_credentials,
            open_startup_windows,
            test_notification,
            set_desktop_notifications_enabled,
            // Crypto commands
            check_password_strength_local,
            check_password_strength,
            get_device_id,
            check_unlock_rate_limit,
            get_rate_limit_status,
            encrypt_credentials,
            unlock_vault,
            lock_vault,
            is_vault_unlocked,
            has_practice_credentials,
            has_live_credentials,
            clear_live_credentials,
            add_live_credentials,
            update_api_key,
            update_api_key_with_vault,
            validate_account_id,
            // Local-first store commands (AGT-642, ~/.wickd/app.db)
            local_store_path,
            local_list_strategies,
            local_get_strategy,
            local_save_strategy,
            local_delete_strategy,
            // Charting domain datasets (AGT-646)
            local_list_sr_zones,
            local_save_sr_zone,
            local_delete_sr_zone,
            local_clear_sr_zones,
            local_list_notes,
            local_save_note,
            local_delete_note,
            local_get_chart_config,
            local_set_chart_config,
            // Trades/analysis domain (AGT-647)
            local_list_trades,
            local_list_closed_trades_by_instrument,
            local_list_trade_scores,
            local_get_trade_score_by_trade,
            local_save_trade_score,
            // Strategies + backtests domain (AGT-645)
            local_list_backtests,
            local_save_backtest,
            local_delete_backtests_for_strategy,
            local_list_backtest_jobs,
            local_get_backtest_job,
            local_save_backtest_job,
            local_record_promotion,
            // Unified .rhai strategy store, read-only (AGT-651, ~/.wickd/strategies/)
            store_list_strategies,
            store_read_strategy,
            // Zero-removal sweep datasets (AGT-650)
            local_list_labels,
            local_save_label,
            local_list_trade_labels,
            local_add_trade_label,
            local_delete_trade_label,
            local_list_strategy_labels,
            local_add_strategy_label,
            local_delete_strategy_label,
            local_list_strategy_trades,
            local_save_strategy_trade,
            local_list_strategy_watchers,
            local_save_strategy_watcher,
            local_delete_strategy_watcher,
            local_get_credential,
            local_save_credential,
            local_delete_credentials
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match event {
                // Handle app activation (e.g., from notification click or dock icon click)
                // Note: Reopen event is macOS-only (dock icon click when no windows visible)
                #[cfg(target_os = "macos")]
                RunEvent::Reopen { has_visible_windows, .. } => {
                    info!("[App] Reopen event (has_visible_windows: {})", has_visible_windows);
                    if !has_visible_windows {
                        focus_watcher_window_on_activation(app_handle);
                    }
                }
                // AGT-652: release the hub socket if the app was hosting it.
                RunEvent::Exit => {
                    if let Some(hub) = app_handle.try_state::<HubStreamState>() {
                        let _ = hub.send(candlesight_lib::hub_stream::Cmd::Shutdown);
                    }
                }
                // Keep app running when all windows closed (for background notifications)
                // Only on macOS - Windows users expect close = quit (BUG-055)
                #[cfg(target_os = "macos")]
                RunEvent::ExitRequested { api, .. } => {
                    info!("[App] Exit requested - preventing to stay in tray");
                    api.prevent_exit();
                }
                _ => {}
            }
        });
}

/// Internal function to open backtest window (called from menu handler)
async fn open_backtest_window_internal(app_handle: &AppHandle) -> Result<(), String> {
    // Check if window already exists
    if let Some(window) = app_handle.get_webview_window("backtest") {
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    // Create new backtest window
    let url = create_webview_url("index.html?window=backtest");

    let window = WebviewWindowBuilder::new(app_handle, "backtest", url)
        .title("wickd - Research")
        .inner_size(1236.0, 800.0)
        .resizable(true)
        .initialization_script(crate::commands::window::DISABLE_ZOOM_SCRIPT)
        .background_color(Color(0x0e, 0x11, 0x17, 0xff))
        .build()
        .map_err(|e| e.to_string())?;
    disable_magnification(&window);

    Ok(())
}

/// Internal function to open watcher window (called from menu handler)
async fn open_watcher_window_internal(app_handle: &AppHandle) -> Result<(), String> {
    // Check if window already exists
    if let Some(window) = app_handle.get_webview_window("watcher") {
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    // Create new watcher window
    let url = create_webview_url("index.html?window=watcher");

    let window = WebviewWindowBuilder::new(app_handle, "watcher", url)
        .title("wickd - Live Monitor")
        .inner_size(900.0, 700.0)
        .resizable(true)
        .initialization_script(crate::commands::window::DISABLE_ZOOM_SCRIPT)
        .background_color(Color(0x0e, 0x11, 0x17, 0xff))
        .build()
        .map_err(|e| e.to_string())?;
    disable_magnification(&window);

    Ok(())
}

/// Internal function to open/focus the home (local) window (called from menu handler).
/// The main window is created at boot from tauri.conf; if the user closed it we
/// recreate it here so Home always has something to focus.
async fn open_home_window_internal(app_handle: &AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("main") {
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    let url = create_webview_url("index.html?window=local");

    let window = WebviewWindowBuilder::new(app_handle, "main", url)
        .title("wickd")
        .inner_size(960.0, 640.0)
        .min_inner_size(640.0, 480.0)
        .resizable(true)
        .initialization_script(crate::commands::window::DISABLE_ZOOM_SCRIPT)
        .background_color(Color(0x0e, 0x11, 0x17, 0xff))
        .build()
        .map_err(|e| e.to_string())?;
    disable_magnification(&window);

    Ok(())
}

/// Internal function to open/focus a chart window (called from menu handler).
/// Opens a bare chart (frontend defaults to EUR_USD H1); reuses a single
/// menu-owned "chart" window rather than the timestamped multi-chart labels
/// created by contextual open-chart flows.
async fn open_chart_window_internal(app_handle: &AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("chart") {
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    let url = create_webview_url("index.html?window=chart");

    let window = WebviewWindowBuilder::new(app_handle, "chart", url)
        .title("wickd - Chart")
        .inner_size(1400.0, 800.0)
        .resizable(true)
        .initialization_script(crate::commands::window::DISABLE_ZOOM_SCRIPT)
        .background_color(Color(0x0e, 0x11, 0x17, 0xff))
        .build()
        .map_err(|e| e.to_string())?;
    disable_magnification(&window);

    Ok(())
}

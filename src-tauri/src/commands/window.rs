//! Window management commands.
//!
//! Handles opening and managing application windows.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri::webview::WebviewWindow;
use tauri::window::Color;
use tracing::info;

/// BUG-040: JavaScript to disable pinch-to-zoom
/// Injected via initialization_script before HTML is parsed
pub const DISABLE_ZOOM_SCRIPT: &str = r#"
(function() {
    // Prevent pinch-to-zoom gestures (macOS trackpad)
    document.addEventListener('gesturestart', function(e) { e.preventDefault(); }, { passive: false, capture: true });
    document.addEventListener('gesturechange', function(e) { e.preventDefault(); }, { passive: false, capture: true });
    document.addEventListener('gestureend', function(e) { e.preventDefault(); }, { passive: false, capture: true });

    // Prevent Ctrl/Cmd + scroll zoom
    document.addEventListener('wheel', function(e) {
        if (e.ctrlKey || e.metaKey) { e.preventDefault(); }
    }, { passive: false, capture: true });

    // Inject viewport meta tag to disable user scaling
    var observer = new MutationObserver(function(mutations, obs) {
        var head = document.head || document.getElementsByTagName('head')[0];
        if (head) {
            var meta = document.createElement('meta');
            meta.name = 'viewport';
            meta.content = 'width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no';
            head.insertBefore(meta, head.firstChild);
            obs.disconnect();
        }
    });
    observer.observe(document.documentElement, { childList: true, subtree: true });
})();
"#;

/// BUG-040: Disable pinch-to-zoom on macOS by setting allowsMagnification = false
/// Call this after building any WebviewWindow
#[allow(unused_variables)]
pub fn disable_magnification(window: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    {
        let _ = window.with_webview(|webview| {
            use objc::{msg_send, sel, sel_impl, runtime::Object};
            unsafe {
                let wkwebview: *mut Object = webview.inner().cast();
                let _: () = msg_send![wkwebview, setAllowsMagnification: false];
            }
        });
    }
}

/// Port used for the localhost plugin in production builds
pub const LOCALHOST_PORT: u16 = 14201;

/// Returns window size with platform-specific adjustments.
/// Windows WebView2 renders content slightly larger and has visible scrollbars,
/// so we add padding to window dimensions on Windows.
fn platform_size(width: f64, height: f64) -> (f64, f64) {
    #[cfg(target_os = "windows")]
    {
        // Windows needs extra width for scrollbar (16px) and height for title bar differences
        (width + 20.0, height + 30.0)
    }
    #[cfg(not(target_os = "windows"))]
    {
        (width, height)
    }
}

/// Create a WebviewUrl that works in both dev and production
/// In dev: uses tauri:// protocol (WebviewUrl::App)
/// In production: uses http://localhost:PORT (WebviewUrl::External)
pub fn create_webview_url(path: &str) -> WebviewUrl {
    #[cfg(debug_assertions)]
    {
        WebviewUrl::App(path.into())
    }
    #[cfg(not(debug_assertions))]
    {
        let url = format!("http://localhost:{}/{}", LOCALHOST_PORT, path);
        WebviewUrl::External(url.parse().expect("Invalid URL"))
    }
}

/// Open the backtest/research window
#[tauri::command]
pub async fn open_backtest_window(app_handle: AppHandle) -> Result<(), String> {
    // Check if window already exists
    if let Some(window) = app_handle.get_webview_window("backtest") {
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    // Create new backtest window
    let url = create_webview_url("index.html?window=backtest");
    let (w, h) = platform_size(1236.0, 800.0);

    let window = WebviewWindowBuilder::new(&app_handle, "backtest", url)
        .title("wickd - Strategy")
        .inner_size(w, h)
        .resizable(true)
        .initialization_script(DISABLE_ZOOM_SCRIPT)
        .background_color(Color(0x0e, 0x11, 0x17, 0xff))
        .build()
        .map_err(|e| e.to_string())?;
    disable_magnification(&window);

    Ok(())
}

/// Open the backtest window with a specific strategy pre-selected
/// Used for deep-linking from watcher errors
#[tauri::command]
pub async fn open_backtest_window_with_strategy(
    app_handle: AppHandle,
    strategy_id: String,
) -> Result<(), String> {
    use tauri::Emitter;

    // Check if window already exists
    if let Some(window) = app_handle.get_webview_window("backtest") {
        // Emit event to select the strategy
        window.emit("select-strategy", &strategy_id)
            .map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    // Create new backtest window with strategy param
    let url = create_webview_url(&format!("index.html?window=backtest&strategy={}", strategy_id));
    let (w, h) = platform_size(1236.0, 800.0);

    let window = WebviewWindowBuilder::new(&app_handle, "backtest", url)
        .title("wickd - Strategy")
        .inner_size(w, h)
        .resizable(true)
        .initialization_script(DISABLE_ZOOM_SCRIPT)
        .background_color(Color(0x0e, 0x11, 0x17, 0xff))
        .build()
        .map_err(|e| e.to_string())?;
    disable_magnification(&window);

    Ok(())
}

/// Open a chart window with optional preloaded parameters
///
/// # Arguments
/// * `instrument` - Optional instrument to preload (e.g., "EUR_USD")
/// * `granularity` - Optional timeframe to preload (e.g., "H4")
/// * `count` - Optional candle count to preload
/// * `trades` - Optional JSON string of trade markers for backtest visualization
/// * `strategy_id` - Optional strategy ID to load indicators from
/// * `signal_direction` - Optional signal direction ("long" or "short") for filtering indicators
/// * `signal_id` - Optional signal ID for tracking execution
/// * `stop_loss` - Optional stop loss price for trade execution
/// * `take_profit` - Optional take profit price for trade execution
/// * `entry_price` - Optional entry price for trade execution
#[tauri::command]
pub async fn open_chart_window(
    app_handle: AppHandle,
    instrument: Option<String>,
    granularity: Option<String>,
    count: Option<u32>,
    from: Option<String>,
    to: Option<String>,
    trades: Option<String>,
    strategy_id: Option<String>,
    signal_direction: Option<String>,
    signal_id: Option<String>,
    stop_loss: Option<String>,
    take_profit: Option<String>,
    entry_price: Option<String>,
    position_size: Option<String>,
    indicators: Option<String>,
) -> Result<(), String> {
    // Build URL with query params (all string values URL-encoded for safety)
    let mut url_str = "index.html?window=chart".to_string();

    if let Some(ref inst) = instrument {
        url_str.push_str(&format!("&instrument={}", urlencoding::encode(inst)));
    }
    if let Some(ref gran) = granularity {
        url_str.push_str(&format!("&granularity={}", urlencoding::encode(gran)));
    }
    if let Some(c) = count {
        url_str.push_str(&format!("&count={}", c));
    }
    if let Some(ref f) = from {
        url_str.push_str(&format!("&from={}", urlencoding::encode(f)));
    }
    if let Some(ref t) = to {
        url_str.push_str(&format!("&to={}", urlencoding::encode(t)));
    }
    if let Some(ref t) = trades {
        url_str.push_str(&format!("&trades={}", urlencoding::encode(t)));
    }
    if let Some(ref sid) = strategy_id {
        url_str.push_str(&format!("&strategyId={}", urlencoding::encode(sid)));
    }
    if let Some(ref dir) = signal_direction {
        url_str.push_str(&format!("&signalDirection={}", urlencoding::encode(dir)));
    }
    if let Some(ref sig_id) = signal_id {
        url_str.push_str(&format!("&signalId={}", urlencoding::encode(sig_id)));
    }
    if let Some(ref sl) = stop_loss {
        url_str.push_str(&format!("&stopLoss={}", urlencoding::encode(sl)));
    }
    if let Some(ref tp) = take_profit {
        url_str.push_str(&format!("&takeProfit={}", urlencoding::encode(tp)));
    }
    if let Some(ref ep) = entry_price {
        url_str.push_str(&format!("&entryPrice={}", urlencoding::encode(ep)));
    }
    if let Some(ref ps) = position_size {
        url_str.push_str(&format!("&positionSize={}", urlencoding::encode(ps)));
    }
    if let Some(ref ind) = indicators {
        url_str.push_str(&format!("&indicators={}", urlencoding::encode(ind)));
    }

    let url = create_webview_url(&url_str);

    let title = if let Some(ref inst) = instrument {
        let gran = granularity.as_deref().unwrap_or("H1");
        format!("Chart - {} {}", inst.replace('_', "/"), gran)
    } else {
        "Chart".to_string()
    };

    // Use unique window label with timestamp to allow multiple charts
    // This allows opening the same instrument multiple times (different timeframes, etc.)
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let window_label = format!("chart-{}", timestamp);
    let (w, h) = platform_size(1400.0, 800.0);

    let window = WebviewWindowBuilder::new(&app_handle, &window_label, url)
        .title(&title)
        .inner_size(w, h)
        .resizable(true)
        .initialization_script(DISABLE_ZOOM_SCRIPT)
        .background_color(Color(0x0e, 0x11, 0x17, 0xff))
        .build()
        .map_err(|e| e.to_string())?;
    disable_magnification(&window);

    Ok(())
}

/// Open the configured startup windows
///
/// # Arguments
/// * `windows` - Array of window types to open: "backtest", "watcher", "chart"
#[tauri::command]
pub async fn open_startup_windows(
    app_handle: AppHandle,
    windows: Vec<String>,
) -> Result<(), String> {
    info!("Opening startup windows: {:?}", windows);

    for window_type in windows {
        match window_type.as_str() {
            // "backtest" or "backtesting" window types (alias for same window)
            "backtest" | "backtesting" => {
                // Check if already exists - focus it
                if let Some(window) = app_handle.get_webview_window("backtest") {
                    let _ = window.set_focus();
                    continue;
                }

                let url = create_webview_url("index.html?window=backtest");
                let (w, h) = platform_size(1236.0, 800.0);
                let window = WebviewWindowBuilder::new(&app_handle, "backtest", url)
                    .title("wickd - Strategy")
                    .inner_size(w, h)
                    .resizable(true)
                    .initialization_script(DISABLE_ZOOM_SCRIPT)
                    .background_color(Color(0x0e, 0x11, 0x17, 0xff))
                    .build()
                    .map_err(|e| e.to_string())?;
                disable_magnification(&window);
            }
            "watcher" => {
                // Check if already exists - focus it
                if let Some(window) = app_handle.get_webview_window("watcher") {
                    let _ = window.set_focus();
                    continue;
                }

                let url = create_webview_url("index.html?window=watcher");
                let (w, h) = platform_size(900.0, 700.0);
                let window = WebviewWindowBuilder::new(&app_handle, "watcher", url)
                    .title("wickd - Live Monitor")
                    .inner_size(w, h)
                    .resizable(true)
                    .initialization_script(DISABLE_ZOOM_SCRIPT)
                    .background_color(Color(0x0e, 0x11, 0x17, 0xff))
                    .build()
                    .map_err(|e| e.to_string())?;
                disable_magnification(&window);
            }
            "charting" | "chart" => {
                // Reuse open_chart_window with no params for a default chart
                open_chart_window(
                    app_handle.clone(),
                    None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                ).await?;
            }
            other => {
                info!("Unknown window type: {}", other);
            }
        }
    }

    Ok(())
}

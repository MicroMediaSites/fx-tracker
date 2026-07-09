//! System tray support.
//!
//! Keeps the app reachable while windows are closed. Since AGT-652 the app
//! hosts no watcher engine — monitors run in the `wickd watch` daemon — so the
//! tray only shows/creates windows and quits; there is nothing in-process to
//! stop.

use tauri::{
    tray::TrayIconBuilder,
    menu::{Menu, MenuItem},
    image::Image,
    window::Color,
    AppHandle, Emitter, Manager, WebviewWindowBuilder,
};
use tracing::{info, warn};

use crate::commands::window::disable_magnification;

/// Tray icon ID
pub const TRAY_ID: &str = "wickd-tray";

/// Initialize system tray (call when background mode is enabled)
pub fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    info!("Setting up system tray");

    // Check if tray already exists
    if app.tray_by_id(TRAY_ID).is_some() {
        info!("Tray already exists, skipping setup");
        return Ok(());
    }

    // Load tray icon - try template icon first, fall back to default
    let icon = load_tray_icon(app)?;

    // Create menu items
    let show = MenuItem::with_id(app, "tray_show", "Show wickd", true, None::<&str>)?;
    let separator = tauri::menu::PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "tray_quit", "Quit", true, None::<&str>)?;

    // Build menu
    let menu = Menu::with_items(app, &[&show, &separator, &quit])?;

    // Build tray icon with explicit ID
    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true) // macOS: use as template image (auto-colorizes)
        .menu(&menu)
        .show_menu_on_left_click(true) // Both left and right click show menu
        .tooltip("wickd")
        .on_menu_event(|app, event| {
            let menu_id = event.id.as_ref();
            info!("Tray menu event: {}", menu_id);
            match menu_id {
                "tray_show" => show_main_window(app),
                "tray_quit" => {
                    info!("Quit from tray menu - exiting immediately");
                    app.exit(0);
                }
                _ => {
                    info!("Unknown menu event: {}", menu_id);
                }
            }
        })
        .build(app)?;

    info!("System tray setup complete with ID: {}", tray.id().as_ref());
    Ok(())
}

/// Load the tray icon, trying template icon first
fn load_tray_icon(app: &AppHandle) -> Result<Image<'static>, Box<dyn std::error::Error>> {
    // Try to load the template icon from resources
    if let Ok(resource_dir) = app.path().resource_dir() {
        let resource_path = resource_dir.join("icons/tray-iconTemplate.png");
        if resource_path.exists() {
            info!("Loading tray icon from: {:?}", resource_path);
            return Image::from_path(resource_path).map_err(|e| e.into());
        }
    }

    // Fall back to default window icon - need to convert to owned
    info!("Template icon not found, using default window icon");
    let icon = app.default_window_icon()
        .ok_or_else(|| -> Box<dyn std::error::Error> { "No default window icon found".into() })?;

    // Create an owned copy of the icon data
    let rgba = icon.rgba().to_vec();
    Ok(Image::new_owned(rgba, icon.width(), icon.height()))
}

/// Show a window: prefer the watcher (Live Monitor), then any existing window,
/// else create a fresh watcher window.
fn show_main_window(app: &AppHandle) {
    info!("Showing main window from tray");

    // Try watcher window first
    if let Some(window) = app.get_webview_window("watcher") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        info!("Showed existing watcher window");
        return;
    }

    // Try any other window
    for (label, window) in app.webview_windows() {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        info!("Showed existing {} window", label);
        return;
    }

    // No windows exist - create the watcher window (the daemon's front window)
    info!("No existing windows, creating watcher window");
    if let Err(e) = create_watcher_window(app) {
        warn!("Failed to create watcher window: {}", e);
        // Emit event as fallback (in case frontend is somehow running)
        let _ = app.emit("tray-show-window", ());
    }
}

/// Create a new watcher window (the daemon's front window).
fn create_watcher_window(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    // Port 14201 matches LOCALHOST_PORT in commands/window.rs
    #[cfg(debug_assertions)]
    let url = tauri::WebviewUrl::App("index.html?window=watcher".into());

    #[cfg(not(debug_assertions))]
    let url = {
        let url_str = "http://localhost:14201/index.html?window=watcher".to_string();
        tauri::WebviewUrl::External(url_str.parse()?)
    };

    let window = WebviewWindowBuilder::new(app, "watcher", url)
        .title("wickd - Live Monitor")
        .inner_size(900.0, 700.0)
        .resizable(true)
        .zoom_hotkeys_enabled(true)
        .initialization_script(crate::commands::window::DISABLE_ZOOM_SCRIPT)
        .background_color(Color(0x0e, 0x11, 0x17, 0xff))
        .build()?;
    disable_magnification(&window);

    info!("Created new watcher window from tray");
    Ok(())
}

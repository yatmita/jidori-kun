mod mcp;
mod settings;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine;
use mcp::CaptureBridge;
use settings::Settings;
use tauri::{AppHandle, Manager, State};

struct ServerHandle {
    addr: String,
    enabled: Arc<AtomicBool>,
    _server: Arc<tiny_http::Server>, // kept bound for the app's lifetime
}

/// The HTTP MCP socket is bound once and the accept thread runs for the app's
/// lifetime. Start/stop just toggle `enabled` — we never drop/rebind the same
/// socket, because tiny_http doesn't reliably release the port on drop.
#[derive(Default)]
struct HttpState {
    inner: Mutex<Option<ServerHandle>>,
}

impl HttpState {
    fn running(&self) -> bool {
        self.inner
            .lock()
            .unwrap()
            .as_ref()
            .map(|h| h.enabled.load(Ordering::SeqCst))
            .unwrap_or(false)
    }
}

// --------------------------------------------------------------------------- //
// Capture commands
// --------------------------------------------------------------------------- //

#[tauri::command]
fn write_capture(path: String, data_url: String) -> Result<(), String> {
    let b64 = data_url.split(',').nth(1).unwrap_or("");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn deliver_capture(state: State<'_, CaptureBridge>, id: u64, data_url: String) {
    state.fulfill(id, data_url);
}

#[tauri::command]
fn frontend_ready(state: State<'_, CaptureBridge>) {
    state.mark_ready();
}

/// Open an http(s) URL in the user's default browser (for the manual-update
/// "download" link — we never download/run anything ourselves).
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("invalid url".into());
    }
    #[cfg(windows)]
    let r = std::process::Command::new("cmd")
        .args(["/C", "start", "", &url])
        .spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(&url).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

// --------------------------------------------------------------------------- //
// Settings + network (HTTP) MCP server
// --------------------------------------------------------------------------- //

#[tauri::command]
fn get_settings(app: AppHandle) -> Settings {
    settings::load(&app)
}

#[tauri::command]
fn save_settings(app: AppHandle, new: Settings) -> Result<(), String> {
    settings::save(&app, &new)
}

#[tauri::command]
fn local_ip() -> Option<String> {
    settings::local_ip()
}

#[tauri::command]
fn http_status(state: State<'_, HttpState>) -> bool {
    state.running()
}

#[tauri::command]
fn stop_http(state: State<'_, HttpState>) {
    if let Some(h) = state.inner.lock().unwrap().as_ref() {
        h.enabled.store(false, Ordering::SeqCst); // socket stays bound; requests get 503
    }
}

/// Enable the HTTP MCP server. Re-enables the existing socket if the address is
/// unchanged; otherwise binds the new address (port/host change).
#[tauri::command]
fn start_http(app: AppHandle, state: State<'_, HttpState>) -> Result<String, String> {
    let s = settings::load(&app);
    let host = if s.lan { "0.0.0.0" } else { "127.0.0.1" };
    let addr = format!("{host}:{}", s.port);

    {
        let mut guard = state.inner.lock().unwrap();
        let reuse = matches!(guard.as_ref(), Some(h) if h.addr == addr);
        if reuse {
            guard.as_ref().unwrap().enabled.store(true, Ordering::SeqCst);
        } else {
            // Address changed (or first start): retire the old server, bind new.
            if let Some(old) = guard.take() {
                old.enabled.store(false, Ordering::SeqCst);
                old._server.unblock();
            }
            let server = Arc::new(
                tiny_http::Server::http(addr.as_str())
                    .map_err(|e| format!("ポート {} を開けませんでした: {e}", s.port))?,
            );
            let enabled = Arc::new(AtomicBool::new(true));
            {
                let app = app.clone();
                let server = server.clone();
                let enabled = enabled.clone();
                std::thread::spawn(move || mcp::serve_http_on(app, server, enabled));
            }
            *guard = Some(ServerHandle {
                addr,
                enabled,
                _server: server,
            });
        }
    }

    let shown = if s.lan {
        settings::local_ip().unwrap_or_else(|| "127.0.0.1".into())
    } else {
        "127.0.0.1".to_string()
    };
    Ok(format!("http://{shown}:{}/mcp", s.port))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // `--mcp` = also serve MCP over stdio (local, Claude spawns it).
    let mcp_stdio = std::env::args().any(|a| a == "--mcp");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(CaptureBridge::new())
        .manage(HttpState::default())
        .invoke_handler(tauri::generate_handler![
            write_capture,
            deliver_capture,
            frontend_ready,
            open_url,
            get_settings,
            save_settings,
            local_ip,
            http_status,
            start_http,
            stop_http
        ])
        .setup(move |app| {
            if mcp_stdio {
                let handle = app.handle().clone();
                std::thread::spawn(move || mcp::serve(handle));
            }
            // Auto-start the network MCP server if enabled in settings.
            let s = settings::load(app.handle());
            if s.http_enabled {
                let state = app.state::<HttpState>();
                let _ = start_http(app.handle().clone(), state);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

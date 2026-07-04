//! Minimal MCP server over stdio (newline-delimited JSON-RPC 2.0).
//!
//! Hand-rolled rather than pulling a fast-moving SDK: the protocol surface we
//! need is tiny (initialize / tools/list / tools/call). The one tool, `snap`,
//! bridges into the running app's webview — it emits an event, the frontend
//! counts down and captures a webcam frame, and delivers it back here.

use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use base64::Engine;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

/// Shared state: pending capture requests keyed by id, awaiting the webview.
pub struct CaptureBridge {
    pending: Mutex<HashMap<u64, Sender<String>>>,
    next_id: AtomicU64,
    ready: Mutex<bool>,
    ready_cv: Condvar,
}

impl CaptureBridge {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            ready: Mutex::new(false),
            ready_cv: Condvar::new(),
        }
    }

    /// Called by the `deliver_capture` command when the webview returns a frame.
    pub fn fulfill(&self, id: u64, data_url: String) {
        if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
            let _ = tx.send(data_url);
        }
    }

    /// The webview signals it has loaded and attached its event listener.
    pub fn mark_ready(&self) {
        *self.ready.lock().unwrap() = true;
        self.ready_cv.notify_all();
    }

    /// Block until the webview is ready (so emitted events aren't lost).
    fn wait_ready(&self, timeout: Duration) {
        let guard = self.ready.lock().unwrap();
        if *guard {
            return;
        }
        let _ = self.ready_cv.wait_timeout(guard, timeout);
    }
}

/// HTTP (Streamable-HTTP-style) MCP transport, for remote Claude clients.
/// One endpoint: `POST /mcp` with a JSON-RPC message, `Bearer <token>` auth.
/// The server is bound by the caller (so bind errors surface synchronously);
/// this runs the accept loop until `flag` is cleared. Reuses `handle()`.
/// The socket is bound once and the thread runs for the app's lifetime. Stop
/// and start toggle `enabled` rather than dropping/rebinding the socket (which
/// tiny_http doesn't reliably release). The token is read per-request, so
/// regenerating it takes effect without a rebind.
pub fn serve_http_on(app: AppHandle, server: Arc<tiny_http::Server>, enabled: Arc<AtomicBool>) {
    use tiny_http::{Header, Method, Response};

    let json_ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    let session = Header::from_bytes(&b"Mcp-Session-Id"[..], &b"jidori-kun"[..]).unwrap();

    loop {
        let mut req = match server.recv() {
            Ok(r) => r,
            Err(_) => break,
        };

        if !enabled.load(Ordering::SeqCst) {
            let _ = req.respond(Response::from_string("server stopped").with_status_code(503));
            continue;
        }

        // Auth against the current token (skip only if none configured).
        let token = crate::settings::load(&app).token;
        let authed = token.is_empty()
            || req.headers().iter().any(|h| {
                h.field.equiv("Authorization") && h.value.as_str().trim() == format!("Bearer {token}")
            });
        if !authed {
            let _ = req.respond(Response::from_string("unauthorized").with_status_code(401));
            continue;
        }
        if *req.method() != Method::Post {
            let _ = req.respond(Response::from_string("use POST /mcp").with_status_code(405));
            continue;
        }

        let mut body = String::new();
        if req.as_reader().read_to_string(&mut body).is_err() {
            let _ = req.respond(Response::from_string("bad body").with_status_code(400));
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&body) else {
            let _ = req.respond(Response::from_string("bad json").with_status_code(400));
            continue;
        };

        match handle(&app, &value) {
            Some(v) => {
                let out = serde_json::to_string(&v).unwrap_or_default();
                let resp = Response::from_string(out)
                    .with_header(json_ct.clone())
                    .with_header(session.clone());
                let _ = req.respond(resp);
            }
            None => {
                let _ = req.respond(Response::from_string("").with_status_code(202));
            }
        }
    }
}

/// Blocking stdio loop. Run on a dedicated thread.
pub fn serve(app: AppHandle) {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(resp) = handle(&app, &req) {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{}", serde_json::to_string(&resp).unwrap_or_default());
            let _ = out.flush();
        }
    }
}

fn handle(app: &AppHandle, req: &Value) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");

    match method {
        "initialize" => Some(json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "jidori-kun", "version": "0.1.0" }
            }
        })),
        "tools/list" => Some(json!({
            "jsonrpc": "2.0", "id": id,
            "result": { "tools": [ snap_schema() ] }
        })),
        "tools/call" => Some(handle_call(app, id, req.get("params"))),
        // Notifications (initialized, cancelled, ...) get no response.
        _ if id.is_none() || method.starts_with("notifications/") => None,
        _ => Some(json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32601, "message": "method not found" }
        })),
    }
}

fn snap_schema() -> Value {
    json!({
        "name": "snap",
        "description": "Count down and capture a webcam photo to use as a pose \
                        reference for image generation. Shows the app window; the \
                        person poses; returns the captured PNG.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "countdown": {
                    "type": "integer",
                    "description": "seconds before capture (default 3)"
                }
            }
        }
    })
}

fn handle_call(app: &AppHandle, id: Option<Value>, params: Option<&Value>) -> Value {
    let name = params
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if name != "snap" {
        return json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32602, "message": format!("unknown tool: {name}") }
        });
    }
    let countdown = params
        .and_then(|p| p.get("arguments"))
        .and_then(|a| a.get("countdown"))
        .and_then(Value::as_i64)
        .unwrap_or(3);

    match do_snap(app, countdown) {
        Ok((path, b64)) => json!({
            "jsonrpc": "2.0", "id": id,
            "result": { "content": [
                { "type": "text", "text": format!("Captured pose reference: {path}") },
                { "type": "image", "data": b64, "mimeType": "image/png" }
            ] }
        }),
        Err(e) => json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "content": [ { "type": "text", "text": format!("snap failed: {e}") } ],
                "isError": true
            }
        }),
    }
}

/// Ask the webview to capture, wait for the frame, save it, return (path, base64).
fn do_snap(app: &AppHandle, countdown: i64) -> Result<(String, String), String> {
    let bridge = app.state::<CaptureBridge>();
    let id = bridge.next_id.fetch_add(1, Ordering::SeqCst);

    let (tx, rx) = channel::<String>();
    bridge.pending.lock().unwrap().insert(id, tx);

    // Wait for the webview to be loaded so the event isn't emitted into the void.
    bridge.wait_ready(Duration::from_secs(20));

    app.emit("mcp-snap", json!({ "id": id, "countdown": countdown }))
        .map_err(|e| e.to_string())?;

    // Countdown + posing time, so allow a generous timeout.
    let data_url = rx
        .recv_timeout(Duration::from_secs(120))
        .map_err(|_| "timed out waiting for webview capture".to_string())?;

    let b64 = data_url.split(',').nth(1).unwrap_or("").to_string();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&b64)
        .map_err(|e| e.to_string())?;

    // Save unobtrusively to temp; the image is also returned to the caller.
    let dir = std::env::temp_dir().join("jidori-kun");
    std::fs::create_dir_all(&dir).ok();
    let path = dir.join(format!("pose-{id}.png"));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;

    Ok((path.to_string_lossy().to_string(), b64))
}

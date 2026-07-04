//! Persisted settings for the network (HTTP) MCP server.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    /// Auto-start the HTTP MCP server on launch.
    pub http_enabled: bool,
    pub port: u16,
    /// Bearer token required by remote clients (empty = no auth — discouraged).
    pub token: String,
    /// true = bind 0.0.0.0 (reachable on the LAN); false = 127.0.0.1 only.
    pub lan: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            http_enabled: false,
            port: 8790,
            token: String::new(),
            lan: true,
        }
    }
}

fn file(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).ok();
    Ok(dir.join("settings.json"))
}

pub fn load(app: &AppHandle) -> Settings {
    file(app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle, s: &Settings) -> Result<(), String> {
    let p = file(app)?;
    std::fs::write(p, serde_json::to_string_pretty(s).unwrap_or_default()).map_err(|e| e.to_string())
}

/// Best-effort primary LAN IP (no external traffic; just reads the chosen route).
pub fn local_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    sock.local_addr().ok().map(|a| a.ip().to_string())
}

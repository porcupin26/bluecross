use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::screen::detect_screen_size;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScreenPosition {
    Left,
    Right,
    Top,
    Bottom,
}

impl ScreenPosition {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub screen_width: i32,
    pub screen_height: i32,
    pub edge_threshold: i32,
    pub clipboard_sharing: bool,
    pub psk: String,
    pub clients: HashMap<String, ScreenPosition>,
}

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub server_host: String,
    pub server_port: u16,
    pub screen_width: i32,
    pub screen_height: i32,
    pub name: String,
    pub clipboard_sharing: bool,
    pub psk: String,
}

#[derive(Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    server: Option<RawServerConfig>,
    #[serde(default)]
    client: Option<RawClientConfig>,
}

#[derive(Deserialize, Default)]
struct RawServerConfig {
    #[serde(default = "default_host")]
    host: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default)]
    screen_width: i32,
    #[serde(default)]
    screen_height: i32,
    #[serde(default = "default_edge_threshold")]
    edge_threshold: i32,
    #[serde(default = "default_true")]
    clipboard_sharing: bool,
    #[serde(default)]
    psk: String,
    #[serde(default)]
    clients: HashMap<String, String>,
}

#[derive(Deserialize, Default)]
struct RawClientConfig {
    #[serde(default = "default_localhost")]
    server_host: String,
    #[serde(default = "default_port")]
    server_port: u16,
    #[serde(default)]
    screen_width: i32,
    #[serde(default)]
    screen_height: i32,
    #[serde(default = "default_client_name")]
    name: String,
    #[serde(default = "default_true")]
    clipboard_sharing: bool,
    #[serde(default)]
    psk: String,
}

// Bind to loopback by default: this is a network input tool and must not be
// exposed on all interfaces unless the operator explicitly opts in.
fn default_host() -> String {
    "127.0.0.1".to_string()
}
fn default_localhost() -> String {
    "127.0.0.1".to_string()
}
fn default_port() -> u16 {
    12345
}
fn default_edge_threshold() -> i32 {
    5
}
fn default_true() -> bool {
    true
}
fn default_client_name() -> String {
    "client1".to_string()
}

/// Minimum acceptable pre-shared key length. Short keys are trivially brute-forced.
const MIN_PSK_LEN: usize = 16;

fn validate_psk(psk: &str) -> anyhow::Result<()> {
    if psk.trim().is_empty() {
        anyhow::bail!(
            "no 'psk' (pre-shared key) configured. BlueCross refuses to run without one: \
             keystrokes and clipboard travel over the network and the client injects whatever \
             the server sends. Set the same random 'psk' (>= {} chars) on server and client, \
             e.g. generate one with: head -c 32 /dev/urandom | base64",
            MIN_PSK_LEN
        );
    }
    if psk.trim().len() < MIN_PSK_LEN {
        anyhow::bail!(
            "'psk' is too short; use at least {} characters",
            MIN_PSK_LEN
        );
    }
    Ok(())
}

fn resolve_dimensions(width: i32, height: i32) -> anyhow::Result<(i32, i32)> {
    let mut w = width;
    let mut h = height;
    if w == 0 || h == 0 {
        let (dw, dh) = detect_screen_size();
        if w == 0 {
            w = dw;
        }
        if h == 0 {
            h = dh;
        }
    }
    if w <= 0 || h <= 0 {
        anyhow::bail!(
            "invalid screen dimensions {}x{}; set screen_width/screen_height in config",
            w,
            h
        );
    }
    Ok((w, h))
}

fn load_raw(path: &str) -> anyhow::Result<RawConfig> {
    if Path::new(path).exists() {
        let data = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str::<RawConfig>(&data)?)
    } else {
        Ok(RawConfig::default())
    }
}

pub fn load_server_config(path: &str) -> anyhow::Result<ServerConfig> {
    let s = load_raw(path)?.server.unwrap_or_default();
    validate_psk(&s.psk)?;
    let (screen_width, screen_height) = resolve_dimensions(s.screen_width, s.screen_height)?;

    let mut clients = HashMap::new();
    for (name, pos_str) in &s.clients {
        let pos = ScreenPosition::from_str(pos_str).ok_or_else(|| {
            anyhow::anyhow!("Invalid position '{}' for client '{}'", pos_str, name)
        })?;
        clients.insert(name.clone(), pos);
    }

    Ok(ServerConfig {
        host: s.host,
        port: s.port,
        screen_width,
        screen_height,
        edge_threshold: s.edge_threshold,
        clipboard_sharing: s.clipboard_sharing,
        psk: s.psk.trim().to_string(),
        clients,
    })
}

pub fn load_client_config(path: &str) -> anyhow::Result<ClientConfig> {
    let c = load_raw(path)?.client.unwrap_or_default();
    validate_psk(&c.psk)?;
    let (screen_width, screen_height) = resolve_dimensions(c.screen_width, c.screen_height)?;

    Ok(ClientConfig {
        server_host: c.server_host,
        server_port: c.server_port,
        screen_width,
        screen_height,
        name: c.name,
        clipboard_sharing: c.clipboard_sharing,
        psk: c.psk.trim().to_string(),
    })
}

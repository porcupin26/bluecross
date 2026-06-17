use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use socket2::{SockRef, TcpKeepalive};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::clipboard::ClipboardManager;
use crate::config::{ScreenPosition, ServerConfig};
use crate::input_capture::{self, CaptureEvent};
use crate::protocol::{Message, MessageType, ProtocolReader, ProtocolWriter};
use crate::secure;

/// How long a peer has to complete the Noise + application handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Per-client outbound queue depth. A client that cannot keep up has excess
/// messages dropped rather than stalling the whole server event loop.
const CLIENT_QUEUE: usize = 1024;

struct ClientInfo {
    position: ScreenPosition,
    screen_width: i32,
    screen_height: i32,
    /// Outbound queue drained by a dedicated per-client writer task.
    tx: mpsc::Sender<Message>,
}

enum ServerEvent {
    Input(CaptureEvent),
    NewClient {
        name: String,
        position: ScreenPosition,
        screen_width: i32,
        screen_height: i32,
        tx: mpsc::Sender<Message>,
    },
    ClientMessage {
        name: String,
        msg: Message,
    },
    ClientDisconnected(String),
    ClipboardChanged(String),
}

struct Server {
    config: ServerConfig,
    clients: HashMap<String, ClientInfo>,
    active_client: Option<String>,
    device_fds: Vec<i32>,
    grabbed: bool,
    mouse_x: i32,
    mouse_y: i32,
    pending_dx: i32,
    pending_dy: i32,
    last_flush: Instant,
    clipboard: ClipboardManager,
    clipboard_paused: bool,
}

impl Server {
    fn new(config: ServerConfig, clipboard: ClipboardManager) -> Self {
        let mx = config.screen_width / 2;
        let my = config.screen_height / 2;
        Self {
            config,
            clients: HashMap::new(),
            active_client: None,
            device_fds: Vec::new(),
            grabbed: false,
            mouse_x: mx,
            mouse_y: my,
            pending_dx: 0,
            pending_dy: 0,
            last_flush: Instant::now(),
            clipboard,
            clipboard_paused: false,
        }
    }

    fn grab_devices(&mut self) {
        if !self.grabbed {
            input_capture::grab_devices(&self.device_fds);
            self.grabbed = true;
        }
    }

    fn ungrab_devices(&mut self) {
        if self.grabbed {
            input_capture::ungrab_devices(&self.device_fds);
            self.grabbed = false;
        }
    }

    /// Reset the local cursor to the center so returning control from a client
    /// does not immediately re-trigger the edge that switched away.
    fn center_cursor(&mut self) {
        self.mouse_x = self.config.screen_width / 2;
        self.mouse_y = self.config.screen_height / 2;
        self.pending_dx = 0;
        self.pending_dy = 0;
    }

    /// Queue a message to a client. Returns false if the client is gone
    /// (writer task ended); a full queue drops the message but is not fatal.
    fn send_to(&self, name: &str, msg: Message) -> bool {
        match self.clients.get(name) {
            Some(client) => match client.tx.try_send(msg) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    log::debug!("client '{}' queue full; dropping message", name);
                    true
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            },
            None => false,
        }
    }

    fn check_edge(&self, x: i32, y: i32) -> Option<(String, i32, i32)> {
        let threshold = self.config.edge_threshold;
        let server_w = self.config.screen_width;
        let server_h = self.config.screen_height;
        let entry_offset = 100;

        for (name, client) in &self.clients {
            let pos = client.position;
            let client_w = client.screen_width;
            let client_h = client.screen_height;

            match pos {
                ScreenPosition::Left if x <= threshold => {
                    let entry_x = client_w - entry_offset;
                    let entry_y = y * client_h / server_h;
                    return Some((name.clone(), entry_x, entry_y));
                }
                ScreenPosition::Right if x >= server_w - threshold - 1 => {
                    let entry_x = entry_offset;
                    let entry_y = y * client_h / server_h;
                    return Some((name.clone(), entry_x, entry_y));
                }
                ScreenPosition::Top if y <= threshold => {
                    let entry_x = x * client_w / server_w;
                    let entry_y = client_h - entry_offset;
                    return Some((name.clone(), entry_x, entry_y));
                }
                ScreenPosition::Bottom if y >= server_h - threshold - 1 => {
                    let entry_x = x * client_w / server_w;
                    let entry_y = entry_offset;
                    return Some((name.clone(), entry_x, entry_y));
                }
                _ => {}
            }
        }

        None
    }

    fn handle_input(&mut self, event: CaptureEvent) {
        match event {
            CaptureEvent::MouseMove { dx, dy } => {
                self.mouse_x = (self.mouse_x + dx).clamp(0, self.config.screen_width - 1);
                self.mouse_y = (self.mouse_y + dy).clamp(0, self.config.screen_height - 1);

                if let Some(active_name) = self.active_client.clone() {
                    self.pending_dx += dx;
                    self.pending_dy += dy;

                    let now = Instant::now();
                    if now.duration_since(self.last_flush) >= Duration::from_millis(2) {
                        let msg = Message::mouse_move(
                            self.mouse_x,
                            self.mouse_y,
                            self.pending_dx,
                            self.pending_dy,
                        );
                        if self.send_to(&active_name, msg) {
                            self.pending_dx = 0;
                            self.pending_dy = 0;
                            self.last_flush = now;
                        } else {
                            self.deactivate(&active_name);
                        }
                    }
                } else if let Some((name, entry_x, entry_y)) =
                    self.check_edge(self.mouse_x, self.mouse_y)
                {
                    self.active_client = Some(name.clone());
                    self.grab_devices();
                    self.clipboard_paused = true;
                    log::info!("Switching to client '{}'", name);

                    if !self.send_to(&name, Message::switch_to_client(entry_x, entry_y)) {
                        log::error!("Failed to switch to client '{}'", name);
                        self.deactivate(&name);
                    }
                }
            }
            CaptureEvent::Key { code, value } => {
                self.forward_to_active(Message::key_event(code, value), true);
            }
            CaptureEvent::MouseButton { code, value } => {
                self.forward_to_active(Message::mouse_button(code, value), true);
            }
            CaptureEvent::MouseScroll { dx, dy } => {
                self.forward_to_active(Message::mouse_scroll(dx, dy), false);
            }
        }
    }

    /// Forward a message to the active client, optionally flushing pending mouse
    /// movement first so motion stays ordered ahead of keys/buttons.
    fn forward_to_active(&mut self, msg: Message, flush_motion: bool) {
        let Some(active_name) = self.active_client.clone() else {
            return;
        };
        if flush_motion && (self.pending_dx != 0 || self.pending_dy != 0) {
            let motion =
                Message::mouse_move(self.mouse_x, self.mouse_y, self.pending_dx, self.pending_dy);
            self.pending_dx = 0;
            self.pending_dy = 0;
            if !self.send_to(&active_name, motion) {
                self.deactivate(&active_name);
                return;
            }
        }
        if !self.send_to(&active_name, msg) {
            self.deactivate(&active_name);
        }
    }

    /// Clear the active client and restore local control.
    fn deactivate(&mut self, name: &str) {
        if self.active_client.as_deref() == Some(name) {
            self.active_client = None;
            self.ungrab_devices();
            self.clipboard_paused = false;
            self.center_cursor();
        }
    }

    async fn handle_client_message(&mut self, client_name: &str, msg: Message) {
        match msg.msg_type {
            MessageType::SwitchToServer => {
                if self.active_client.as_deref() == Some(client_name) {
                    self.deactivate(client_name);
                    log::info!("Control returned from client '{}'", client_name);
                }
            }
            MessageType::ClipboardData => {
                if self.config.clipboard_sharing {
                    if let Some(content) = msg.payload.get("content").and_then(|v| v.as_str()) {
                        log::debug!("Received clipboard from {}", client_name);
                        // Applying the value records it as last-seen, so the
                        // local watcher will not rebroadcast it.
                        self.clipboard.set(content).await;
                        let broadcast = Message::clipboard_data(content);
                        let others: Vec<String> = self
                            .clients
                            .keys()
                            .filter(|n| n.as_str() != client_name)
                            .cloned()
                            .collect();
                        for name in others {
                            self.send_to(&name, broadcast.clone());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_client_disconnect(&mut self, client_name: &str) {
        self.clients.remove(client_name);
        self.deactivate(client_name);
        log::info!("Client '{}' disconnected", client_name);
    }

    fn broadcast_clipboard(&mut self, content: &str) {
        if self.clients.is_empty() || self.clipboard_paused {
            return;
        }
        log::debug!("Broadcasting clipboard to {} client(s)", self.clients.len());
        let msg = Message::clipboard_data(content);
        let names: Vec<String> = self.clients.keys().cloned().collect();
        for name in names {
            self.send_to(&name, msg.clone());
        }
    }
}

/// Apply aggressive TCP keepalive so half-open connections are detected and the
/// reader task observes an error within ~30s instead of hours.
fn apply_keepalive(stream: &TcpStream) {
    let ka = TcpKeepalive::new()
        .with_time(Duration::from_secs(15))
        .with_interval(Duration::from_secs(5))
        .with_retries(3);
    if let Err(e) = SockRef::from(stream).set_tcp_keepalive(&ka) {
        log::debug!("Could not set TCP keepalive: {}", e);
    }
}

/// Spawn the dedicated writer task for one client.
fn spawn_client_writer(
    mut writer: ProtocolWriter,
    mut rx: mpsc::Receiver<Message>,
    event_tx: mpsc::Sender<ServerEvent>,
    name: String,
) {
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if writer.send(&msg).await.is_err() || writer.flush().await.is_err() {
                break;
            }
        }
        let _ = event_tx.send(ServerEvent::ClientDisconnected(name)).await;
    });
}

async fn handle_new_connection(
    stream: TcpStream,
    addr: std::net::SocketAddr,
    config: Arc<ServerConfig>,
    event_tx: mpsc::Sender<ServerEvent>,
) -> anyhow::Result<()> {
    log::info!("Client connecting from {}", addr);
    apply_keepalive(&stream);

    // Encrypted, mutually-authenticated channel (fails here on a wrong PSK).
    let (secure_reader, secure_writer) = match tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        secure::server_handshake(stream, &config.psk),
    )
    .await
    {
        Ok(res) => res?,
        Err(_) => anyhow::bail!("secure handshake timed out for {}", addr),
    };
    let mut reader = ProtocolReader::new(secure_reader);
    let mut writer = ProtocolWriter::new(secure_writer);

    // Application handshake.
    let msg = match tokio::time::timeout(HANDSHAKE_TIMEOUT, reader.receive()).await {
        Ok(res) => res?.ok_or_else(|| anyhow::anyhow!("Connection closed during handshake"))?,
        Err(_) => anyhow::bail!("application handshake timed out for {}", addr),
    };

    if msg.msg_type != MessageType::Handshake {
        anyhow::bail!("Invalid handshake from {}", addr);
    }

    let client_name = msg
        .payload
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing client name"))?
        .to_string();
    let screen_width = msg
        .payload
        .get("screen_width")
        .and_then(|v| v.as_i64())
        .unwrap_or(1920) as i32;
    let screen_height = msg
        .payload
        .get("screen_height")
        .and_then(|v| v.as_i64())
        .unwrap_or(1080) as i32;

    let position = config
        .clients
        .get(&client_name)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("Unknown client '{}' from {}", client_name, addr))?;

    writer
        .send(&Message::handshake_ack(
            position.as_str(),
            config.screen_width,
            config.screen_height,
        ))
        .await?;
    writer.flush().await?;

    log::info!(
        "Client '{}' connected from {} (position: {}, screen: {}x{})",
        client_name,
        addr,
        position.as_str(),
        screen_width,
        screen_height,
    );

    // Hand the writer to a dedicated task; the main loop only queues messages.
    let (cmd_tx, cmd_rx) = mpsc::channel::<Message>(CLIENT_QUEUE);
    spawn_client_writer(writer, cmd_rx, event_tx.clone(), client_name.clone());

    event_tx
        .send(ServerEvent::NewClient {
            name: client_name.clone(),
            position,
            screen_width,
            screen_height,
            tx: cmd_tx,
        })
        .await?;

    // Reader task feeds messages into the main event loop.
    let tx = event_tx.clone();
    let name = client_name.clone();
    tokio::spawn(async move {
        loop {
            match reader.receive().await {
                Ok(Some(msg)) => {
                    if tx
                        .send(ServerEvent::ClientMessage {
                            name: name.clone(),
                            msg,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                _ => {
                    let _ = tx.send(ServerEvent::ClientDisconnected(name)).await;
                    break;
                }
            }
        }
    });

    Ok(())
}

pub fn run(config_path: &str, foreground: bool, debug: bool) -> anyhow::Result<()> {
    use crate::daemon;

    if daemon::is_running("server") {
        eprintln!("BlueCross server is already running");
        std::process::exit(1);
    }

    // Load and validate config before daemonizing so errors reach the terminal
    // and a failed start leaves no stale PID file.
    let config = crate::config::load_server_config(config_path)?;

    if !foreground {
        daemon::daemonize()?;
    }

    daemon::setup_logging("server", debug, foreground)?;
    daemon::write_pid_file("server")?;

    if foreground {
        println!("BlueCross Server");
        println!("================");
        println!("Press Ctrl+C to stop");
        println!();
    }

    log::info!("BlueCross server starting");

    let rt = tokio::runtime::Runtime::new()?;
    let result = rt.block_on(async { run_async(config).await });

    daemon::cleanup_pid("server");
    log::info!("BlueCross server stopped");

    result
}

async fn run_async(config: ServerConfig) -> anyhow::Result<()> {
    let (event_tx, mut event_rx) = mpsc::channel::<ServerEvent>(4096);

    // Start input capture
    let (mut capture_rx, device_fds) =
        input_capture::start_capture(config.screen_width, config.screen_height)?;

    // Forward capture events to server event channel
    let input_tx = event_tx.clone();
    tokio::spawn(async move {
        while let Some(ev) = capture_rx.recv().await {
            if input_tx.send(ServerEvent::Input(ev)).await.is_err() {
                break;
            }
        }
    });

    // Start TCP listener
    let listener = TcpListener::bind((&*config.host, config.port)).await?;
    let addr = listener.local_addr()?;
    log::info!("Server listening on {}:{}", addr.ip(), addr.port());
    if config.host == "0.0.0.0" || config.host == "::" {
        log::warn!(
            "Listening on all interfaces ({}). Ensure the network is trusted; access is \
             protected only by the pre-shared key.",
            config.host
        );
    }
    log::info!(
        "Screen size: {}x{}",
        config.screen_width,
        config.screen_height
    );
    log::info!("Edge threshold: {}px", config.edge_threshold);
    log::info!(
        "Configured clients: {:?}",
        config.clients.keys().collect::<Vec<_>>()
    );
    log::info!(
        "Clipboard sharing: {}",
        if config.clipboard_sharing {
            "enabled"
        } else {
            "disabled"
        }
    );

    let shared_config = Arc::new(config.clone());

    let listener_tx = event_tx.clone();
    let listener_config = shared_config.clone();
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let tx = listener_tx.clone();
                    let cfg = listener_config.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_new_connection(stream, addr, cfg, tx).await {
                            log::error!("Connection error from {}: {}", addr, e);
                        }
                    });
                }
                Err(e) => {
                    log::error!("Accept error: {}", e);
                }
            }
        }
    });

    // Shared clipboard manager: the same instance applies remote updates and
    // watches for local changes, so applied values are not echoed back out.
    let clipboard = ClipboardManager::new();
    if config.clipboard_sharing {
        let clip_tx = event_tx.clone();
        let monitor = clipboard.clone();
        let (content_tx, mut content_rx) = mpsc::channel::<String>(32);
        tokio::spawn(async move {
            monitor.monitor(content_tx).await;
        });
        tokio::spawn(async move {
            while let Some(content) = content_rx.recv().await {
                let _ = clip_tx.send(ServerEvent::ClipboardChanged(content)).await;
            }
        });
    }

    let mut server = Server::new(config, clipboard);
    server.device_fds = device_fds;

    // Handle shutdown signals
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    loop {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                match event {
                    ServerEvent::Input(ev) => server.handle_input(ev),
                    ServerEvent::NewClient { name, position, screen_width, screen_height, tx } => {
                        server.clients.insert(name.clone(), ClientInfo {
                            position,
                            screen_width,
                            screen_height,
                            tx,
                        });
                        log::info!("Client '{}' registered", name);
                    }
                    ServerEvent::ClientMessage { name, msg } => {
                        server.handle_client_message(&name, msg).await;
                    }
                    ServerEvent::ClientDisconnected(name) => {
                        server.handle_client_disconnect(&name);
                    }
                    ServerEvent::ClipboardChanged(content) => {
                        server.broadcast_clipboard(&content);
                    }
                }
            }
            _ = sigterm.recv() => {
                log::info!("Received SIGTERM, shutting down");
                break;
            }
            _ = sigint.recv() => {
                log::info!("Received SIGINT, shutting down");
                break;
            }
        }
    }

    server.ungrab_devices();
    Ok(())
}

use std::time::{Duration, Instant};

use socket2::{SockRef, TcpKeepalive};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::clipboard::ClipboardManager;
use crate::config::{ClientConfig, ScreenPosition};
use crate::input_inject::InputInjector;
use crate::protocol::{Message, MessageType, ProtocolReader, ProtocolWriter};
use crate::secure;

struct ClientState {
    config: ClientConfig,
    position: Option<ScreenPosition>,
    server_width: i32,
    server_height: i32,
    active: bool,
    mouse_x: i32,
    mouse_y: i32,
    switch_time: Instant,
    edge_push_start: Instant,
    edge_pushing: bool,
}

impl ClientState {
    fn new(config: ClientConfig) -> Self {
        Self {
            config,
            position: None,
            server_width: 1920,
            server_height: 1080,
            active: false,
            mouse_x: 0,
            mouse_y: 0,
            switch_time: Instant::now(),
            edge_push_start: Instant::now(),
            edge_pushing: false,
        }
    }

    fn check_exit_edge(&mut self, x: i32, y: i32, dx: i32, dy: i32) -> bool {
        let position = match self.position {
            Some(p) => p,
            None => return false,
        };

        // Grace period after switching - ignore exit edge for 200ms
        if self.switch_time.elapsed() < Duration::from_millis(200) {
            return false;
        }

        let w = self.config.screen_width;
        let h = self.config.screen_height;

        let (at_edge, pushing_exit, moving_away) = match position {
            ScreenPosition::Left => (x >= w - 1, dx > 0, dx < 0),
            ScreenPosition::Right => (x <= 0, dx < 0, dx > 0),
            ScreenPosition::Top => (y >= h - 1, dy > 0, dy < 0),
            ScreenPosition::Bottom => (y <= 0, dy < 0, dy > 0),
        };

        if at_edge && pushing_exit {
            if !self.edge_pushing {
                self.edge_push_start = Instant::now();
                self.edge_pushing = true;
                log::debug!("Edge timer started at x={}", x);
            }
            let elapsed = self.edge_push_start.elapsed();
            if elapsed >= Duration::from_millis(300) {
                log::debug!("Edge timer completed: {:.3}s", elapsed.as_secs_f64());
                self.edge_pushing = false;
                return true;
            }
        } else if moving_away || !at_edge {
            if self.edge_pushing {
                log::debug!(
                    "Edge timer reset at x={}, moving_away={}, at_edge={}",
                    x,
                    moving_away,
                    at_edge
                );
            }
            self.edge_pushing = false;
        }
        // If at edge with zero delta (stationary), keep the timer running

        false
    }
}

/// Apply a single message: inject input (sync) or apply clipboard (async).
/// Returns accumulated (dx, dy) for mouse-move messages, (0, 0) otherwise.
async fn apply_message(
    msg: &Message,
    state: &mut ClientState,
    injector: &mut InputInjector,
    clipboard: &ClipboardManager,
) -> (i32, i32) {
    match msg.msg_type {
        MessageType::SwitchToClient => {
            state.active = true;
            state.switch_time = Instant::now();
            state.edge_pushing = false;
            state.mouse_x =
                msg.payload
                    .get("entry_x")
                    .and_then(|v| v.as_i64())
                    .unwrap_or((state.config.screen_width / 2) as i64) as i32;
            state.mouse_y =
                msg.payload
                    .get("entry_y")
                    .and_then(|v| v.as_i64())
                    .unwrap_or((state.config.screen_height / 2) as i64) as i32;
            log::info!(
                "Control received (entry point: {}, {})",
                state.mouse_x,
                state.mouse_y
            );
            (0, 0)
        }
        MessageType::ClipboardData => {
            let content = msg
                .payload
                .get("content")
                .and_then(|v| v.as_str())
                .filter(|_| state.config.clipboard_sharing);
            if let Some(content) = content {
                log::debug!("Received clipboard");
                clipboard.set(content).await;
            }
            (0, 0)
        }
        _ if !state.active => (0, 0),
        MessageType::KeyEvent => {
            let code = msg
                .payload
                .get("code")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as u16;
            let value = msg
                .payload
                .get("value")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            injector.inject_key(code, value);
            (0, 0)
        }
        MessageType::MouseMove => {
            let dx = msg.payload.get("dx").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let dy = msg.payload.get("dy").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            (dx, dy)
        }
        MessageType::MouseButton => {
            let button = msg
                .payload
                .get("button")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as u16;
            let value = msg
                .payload
                .get("value")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32;
            injector.inject_mouse_button(button, value);
            (0, 0)
        }
        MessageType::MouseScroll => {
            let dx = msg.payload.get("dx").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let dy = msg.payload.get("dy").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            injector.inject_mouse_scroll(dx, dy);
            (0, 0)
        }
        _ => (0, 0),
    }
}

pub fn run(config_path: &str, foreground: bool, debug: bool) -> anyhow::Result<()> {
    use crate::daemon;

    if daemon::is_running("client") {
        eprintln!("BlueCross client is already running");
        std::process::exit(1);
    }

    // Load and validate config before daemonizing so errors reach the terminal
    // and a failed start leaves no stale PID file.
    let config = crate::config::load_client_config(config_path)?;

    if !foreground {
        daemon::daemonize()?;
    }

    daemon::setup_logging("client", debug, foreground)?;
    daemon::write_pid_file("client")?;

    if foreground {
        println!("BlueCross Client");
        println!("================");
        println!("Press Ctrl+C to stop");
        println!();
    }

    log::info!("BlueCross client starting");

    let rt = tokio::runtime::Runtime::new()?;
    let result = rt.block_on(async { run_async(config).await });

    daemon::cleanup_pid("client");
    log::info!("BlueCross client stopped");

    result
}

async fn run_async(config: ClientConfig) -> anyhow::Result<()> {
    let mut injector = InputInjector::new();
    injector.setup()?;

    log::info!(
        "Connecting to server at {}:{}",
        config.server_host,
        config.server_port
    );

    let stream = TcpStream::connect((&*config.server_host, config.server_port)).await?;
    let ka = TcpKeepalive::new()
        .with_time(Duration::from_secs(15))
        .with_interval(Duration::from_secs(5))
        .with_retries(3);
    if let Err(e) = SockRef::from(&stream).set_tcp_keepalive(&ka) {
        log::debug!("Could not set TCP keepalive: {}", e);
    }

    // Establish the encrypted, mutually-authenticated channel.
    let (secure_reader, secure_writer) = secure::client_handshake(stream, &config.psk).await?;
    let mut reader = ProtocolReader::new(secure_reader);
    let mut writer = ProtocolWriter::new(secure_writer);

    // Application handshake
    writer
        .send(&Message::handshake(
            &config.name,
            config.screen_width,
            config.screen_height,
        ))
        .await?;
    writer.flush().await?;

    let msg = reader
        .receive()
        .await?
        .ok_or_else(|| anyhow::anyhow!("Connection closed during handshake"))?;

    if msg.msg_type != MessageType::HandshakeAck {
        anyhow::bail!("Handshake failed");
    }

    let mut state = ClientState::new(config.clone());
    state.position = msg
        .payload
        .get("position")
        .and_then(|v| v.as_str())
        .and_then(ScreenPosition::from_str);
    state.server_width = msg
        .payload
        .get("server_width")
        .and_then(|v| v.as_i64())
        .unwrap_or(1920) as i32;
    state.server_height = msg
        .payload
        .get("server_height")
        .and_then(|v| v.as_i64())
        .unwrap_or(1080) as i32;

    log::info!("Connected as '{}'", config.name);
    log::info!(
        "Position: {} of server screen",
        state.position.map(|p| p.as_str()).unwrap_or("unknown")
    );
    log::info!(
        "Server screen: {}x{}",
        state.server_width,
        state.server_height
    );
    log::info!(
        "Client screen: {}x{}",
        config.screen_width,
        config.screen_height
    );
    log::info!(
        "Clipboard sharing: {}",
        if config.clipboard_sharing {
            "enabled"
        } else {
            "disabled"
        }
    );
    log::info!("Waiting for input...");

    // Shared clipboard manager (apply remote + watch local without echo loop).
    let clipboard = ClipboardManager::new();
    let (clip_tx, mut clip_rx) = mpsc::channel::<String>(32);
    if config.clipboard_sharing {
        let monitor = clipboard.clone();
        tokio::spawn(async move {
            monitor.monitor(clip_tx).await;
        });
    }

    // Reader task: decode messages and forward them over a channel. Using a
    // channel keeps the main select! loop cancel-safe (receive() is not).
    let (msg_tx, mut msg_rx) = mpsc::channel::<Message>(4096);
    tokio::spawn(async move {
        // On EOF or error, returning drops msg_tx, which signals the main loop.
        while let Ok(Some(msg)) = reader.receive().await {
            if msg_tx.send(msg).await.is_err() {
                return;
            }
            for buffered in reader.receive_all_buffered() {
                if msg_tx.send(buffered).await.is_err() {
                    return;
                }
            }
        }
    });

    // Handle shutdown signals
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    // Periodic application-level heartbeat keeps NAT/middlebox state alive.
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                writer.send(&Message::heartbeat()).await?;
                writer.flush().await?;
            }
            maybe_msg = msg_rx.recv() => {
                let Some(first) = maybe_msg else {
                    log::warn!("Connection lost");
                    break;
                };
                // Process this message plus any already queued, coalescing motion.
                let (mut total_dx, mut total_dy) =
                    apply_message(&first, &mut state, &mut injector, &clipboard).await;
                while let Ok(m) = msg_rx.try_recv() {
                    let (dx, dy) = apply_message(&m, &mut state, &mut injector, &clipboard).await;
                    total_dx += dx;
                    total_dy += dy;
                }

                if total_dx != 0 || total_dy != 0 {
                    state.mouse_x = (state.mouse_x + total_dx).clamp(0, state.config.screen_width - 1);
                    state.mouse_y = (state.mouse_y + total_dy).clamp(0, state.config.screen_height - 1);

                    if state.check_exit_edge(state.mouse_x, state.mouse_y, total_dx, total_dy) {
                        state.active = false;
                        log::info!("Returning control to server (pos={},{})", state.mouse_x, state.mouse_y);
                        writer.send(&Message::switch_to_server()).await?;
                        writer.flush().await?;
                    } else {
                        injector.inject_mouse_move(total_dx, total_dy);
                    }
                }
            }
            Some(content) = clip_rx.recv() => {
                writer.send(&Message::clipboard_data(&content)).await?;
                writer.flush().await?;
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

    injector.close();
    Ok(())
}

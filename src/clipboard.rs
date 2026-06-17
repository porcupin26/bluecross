//! Wayland clipboard integration via `wl-clipboard` (`wl-copy` / `wl-paste`).
//!
//! BlueCross is Wayland-only; X11 helpers (xclip/xsel) are intentionally not used.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

/// Manages reading, writing and watching the Wayland clipboard.
///
/// The `last_content` handle is shared between [`ClipboardManager::set`] (which
/// writes the clipboard from a *remote* update) and [`ClipboardManager::monitor`]
/// (which watches for *local* changes). Sharing it means a clipboard value we
/// applied from the network is recognized as "already seen" and not echoed back
/// out, breaking the rebroadcast loop.
#[derive(Clone)]
pub struct ClipboardManager {
    last_content: Arc<Mutex<String>>,
}

impl ClipboardManager {
    pub fn new() -> Self {
        Self {
            last_content: Arc::new(Mutex::new(String::new())),
        }
    }

    /// Read the current clipboard contents (empty string if unavailable).
    pub async fn get_clipboard(&self) -> String {
        match tokio::process::Command::new("wl-paste")
            .arg("--no-newline")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).to_string()
            }
            _ => String::new(),
        }
    }

    /// Set the clipboard to `content` (a value received from a peer) and record
    /// it as the last-seen value so the watcher does not rebroadcast it.
    pub async fn set(&self, content: &str) {
        if content.is_empty() {
            return;
        }
        // Record before writing so a watcher firing on the write sees a match.
        if let Ok(mut last) = self.last_content.lock() {
            *last = content.to_string();
        }

        let child = tokio::process::Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();

        if let Ok(mut child) = child {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(content.as_bytes()).await;
                let _ = stdin.shutdown().await;
            }
            let _ = child.wait().await;
        } else {
            log::warn!("wl-copy not available; clipboard sharing requires wl-clipboard");
        }
    }

    /// Watch for local clipboard changes, sending new content to `tx`.
    /// Spawn this as a task; it runs until the channel closes or the watcher dies.
    pub async fn monitor(self, tx: mpsc::Sender<String>) {
        {
            let initial = self.get_clipboard().await;
            if let Ok(mut last) = self.last_content.lock() {
                *last = initial;
            }
        }

        if let Err(e) = self.monitor_wayland(&tx).await {
            log::warn!(
                "Wayland clipboard watch unavailable ({}); falling back to polling",
                e
            );
            self.monitor_polling(&tx).await;
        }
    }

    fn take_if_changed(&self, content: &str) -> bool {
        if content.is_empty() {
            return false;
        }
        let mut last = match self.last_content.lock() {
            Ok(l) => l,
            Err(_) => return false,
        };
        if *last == content {
            return false;
        }
        *last = content.to_string();
        true
    }

    async fn monitor_wayland(&self, tx: &mpsc::Sender<String>) -> anyhow::Result<()> {
        let mut child = tokio::process::Command::new("wl-paste")
            .args(["--watch", "cat"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null())
            .spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stdout from wl-paste"))?;
        let mut reader = tokio::io::BufReader::new(stdout);

        loop {
            let mut line = String::new();
            match reader.read_line(&mut line).await {
                Ok(0) => break, // EOF: watcher exited
                Ok(_) => {
                    // Coalesce immediately-available trailing lines.
                    let mut content = line;
                    loop {
                        let mut more = String::new();
                        match tokio::time::timeout(
                            std::time::Duration::from_millis(20),
                            reader.read_line(&mut more),
                        )
                        .await
                        {
                            Ok(Ok(0)) | Err(_) | Ok(Err(_)) => break,
                            Ok(Ok(_)) => content.push_str(&more),
                        }
                    }
                    let content = content.trim_end_matches('\n').to_string();
                    if self.take_if_changed(&content) {
                        log::debug!("Clipboard changed");
                        if tx.send(content).await.is_err() {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }

        let _ = child.kill().await;
        Ok(())
    }

    async fn monitor_polling(&self, tx: &mpsc::Sender<String>) {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let content = self.get_clipboard().await;
            if self.take_if_changed(&content) {
                log::debug!("Clipboard changed");
                if tx.send(content).await.is_err() {
                    break;
                }
            }
        }
    }
}

impl Default for ClipboardManager {
    fn default() -> Self {
        Self::new()
    }
}

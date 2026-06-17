use std::fmt;

use serde_json::Value;

use crate::secure::{SecureReader, SecureWriter};

/// Hard cap on a single message payload (decoded JSON). Bounds memory use when a
/// peer announces a length; also the ceiling for clipboard payloads.
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;

/// Fixed message header size: 1 type byte + 4-byte big-endian length.
const HEADER_LEN: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    Handshake = 1,
    HandshakeAck = 2,
    KeyEvent = 3,
    MouseMove = 4,
    MouseButton = 5,
    MouseScroll = 6,
    SwitchToClient = 7,
    SwitchToServer = 8,
    Heartbeat = 9,
    ClipboardData = 10,
}

impl MessageType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Handshake),
            2 => Some(Self::HandshakeAck),
            3 => Some(Self::KeyEvent),
            4 => Some(Self::MouseMove),
            5 => Some(Self::MouseButton),
            6 => Some(Self::MouseScroll),
            7 => Some(Self::SwitchToClient),
            8 => Some(Self::SwitchToServer),
            9 => Some(Self::Heartbeat),
            10 => Some(Self::ClipboardData),
            _ => None,
        }
    }
}

impl fmt::Display for MessageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub msg_type: MessageType,
    pub payload: Value,
}

impl Message {
    pub fn encode(&self) -> Vec<u8> {
        let payload_bytes =
            serde_json::to_vec(&self.payload).expect("payload is always serializable");
        let mut buf = Vec::with_capacity(HEADER_LEN + payload_bytes.len());
        buf.push(self.msg_type as u8);
        buf.extend_from_slice(&(payload_bytes.len() as u32).to_be_bytes());
        buf.extend_from_slice(&payload_bytes);
        buf
    }

    /// Decode a message from a buffer. Returns (message, bytes_consumed) or None if incomplete.
    pub fn decode(data: &[u8]) -> anyhow::Result<Option<(Self, usize)>> {
        if data.len() < HEADER_LEN {
            return Ok(None);
        }
        let msg_type_byte = data[0];
        let payload_len = u32::from_be_bytes([data[1], data[2], data[3], data[4]]) as usize;
        if payload_len > MAX_PAYLOAD {
            anyhow::bail!(
                "payload length {} exceeds maximum {}",
                payload_len,
                MAX_PAYLOAD
            );
        }
        if data.len() < HEADER_LEN + payload_len {
            return Ok(None);
        }
        let msg_type = MessageType::from_u8(msg_type_byte)
            .ok_or_else(|| anyhow::anyhow!("Invalid message type: {}", msg_type_byte))?;
        let payload: Value = serde_json::from_slice(&data[HEADER_LEN..HEADER_LEN + payload_len])?;
        Ok(Some((Self { msg_type, payload }, HEADER_LEN + payload_len)))
    }

    pub fn handshake(name: &str, screen_width: i32, screen_height: i32) -> Self {
        Self {
            msg_type: MessageType::Handshake,
            payload: serde_json::json!({
                "name": name,
                "screen_width": screen_width,
                "screen_height": screen_height,
            }),
        }
    }

    pub fn handshake_ack(position: &str, server_width: i32, server_height: i32) -> Self {
        Self {
            msg_type: MessageType::HandshakeAck,
            payload: serde_json::json!({
                "position": position,
                "server_width": server_width,
                "server_height": server_height,
            }),
        }
    }

    pub fn key_event(code: u16, value: i32) -> Self {
        Self {
            msg_type: MessageType::KeyEvent,
            payload: serde_json::json!({"code": code, "value": value}),
        }
    }

    pub fn mouse_move(x: i32, y: i32, dx: i32, dy: i32) -> Self {
        Self {
            msg_type: MessageType::MouseMove,
            payload: serde_json::json!({"x": x, "y": y, "dx": dx, "dy": dy}),
        }
    }

    pub fn mouse_button(button: u16, value: i32) -> Self {
        Self {
            msg_type: MessageType::MouseButton,
            payload: serde_json::json!({"button": button, "value": value}),
        }
    }

    pub fn mouse_scroll(dx: i32, dy: i32) -> Self {
        Self {
            msg_type: MessageType::MouseScroll,
            payload: serde_json::json!({"dx": dx, "dy": dy}),
        }
    }

    pub fn switch_to_client(entry_x: i32, entry_y: i32) -> Self {
        Self {
            msg_type: MessageType::SwitchToClient,
            payload: serde_json::json!({"entry_x": entry_x, "entry_y": entry_y}),
        }
    }

    pub fn switch_to_server() -> Self {
        Self {
            msg_type: MessageType::SwitchToServer,
            payload: serde_json::json!({}),
        }
    }

    pub fn heartbeat() -> Self {
        Self {
            msg_type: MessageType::Heartbeat,
            payload: serde_json::json!({}),
        }
    }

    pub fn clipboard_data(content: &str) -> Self {
        Self {
            msg_type: MessageType::ClipboardData,
            payload: serde_json::json!({"content": content}),
        }
    }
}

/// Reads length-framed [`Message`]s off the encrypted [`SecureReader`].
pub struct ProtocolReader {
    reader: SecureReader,
    buffer: Vec<u8>,
}

impl ProtocolReader {
    pub fn new(reader: SecureReader) -> Self {
        Self {
            reader,
            buffer: Vec::new(),
        }
    }

    pub async fn receive(&mut self) -> anyhow::Result<Option<Message>> {
        loop {
            if let Some((msg, consumed)) = Message::decode(&self.buffer)? {
                self.buffer.drain(..consumed);
                return Ok(Some(msg));
            }
            match self.reader.read_chunk().await? {
                Some(plaintext) => self.buffer.extend_from_slice(&plaintext),
                None => return Ok(None),
            }
        }
    }

    /// Drain any additional complete messages already decrypted into the buffer,
    /// without performing another network read.
    pub fn receive_all_buffered(&mut self) -> Vec<Message> {
        let mut messages = Vec::new();
        while let Ok(Some((msg, consumed))) = Message::decode(&self.buffer) {
            self.buffer.drain(..consumed);
            messages.push(msg);
        }
        messages
    }
}

/// Writes length-framed [`Message`]s into the encrypted [`SecureWriter`].
pub struct ProtocolWriter {
    writer: SecureWriter,
}

impl ProtocolWriter {
    pub fn new(writer: SecureWriter) -> Self {
        Self { writer }
    }

    pub async fn send(&mut self, msg: &Message) -> anyhow::Result<()> {
        let data = msg.encode();
        self.writer.write_all(&data).await?;
        Ok(())
    }

    pub async fn flush(&mut self) -> anyhow::Result<()> {
        self.writer.flush().await?;
        Ok(())
    }
}

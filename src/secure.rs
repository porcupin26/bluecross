//! Encrypted, mutually-authenticated transport for BlueCross.
//!
//! The wire is protected with the Noise protocol (`NNpsk0`): both peers prove
//! knowledge of a shared pre-shared key (PSK) and derive forward-secret session
//! keys from ephemeral X25519 keys. Without the correct PSK the handshake
//! produces mismatched keys and every frame fails authentication, so a peer
//! cannot inject input or read keystrokes/clipboard without the secret.
//!
//! On top of the Noise session this module exposes a reliable encrypted
//! byte-stream: [`SecureWriter`] chunks plaintext into <=64 KiB Noise messages,
//! each framed as `[u16 ciphertext-len][ciphertext]`, and [`SecureReader`]
//! reverses it. The application-level message framing in [`crate::protocol`]
//! rides on top of this stream.

use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use snow::{Builder, TransportState};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

const NOISE_PARAMS: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";
const TAG_LEN: usize = 16;
const MAX_NOISE_MSG: usize = 65535;
/// Maximum plaintext bytes per Noise transport message (16-byte AEAD tag).
const MAX_CHUNK: usize = MAX_NOISE_MSG - TAG_LEN;

/// Shared Noise session. `snow`'s `TransportState` drives both directions with
/// independent nonce counters, so the read and write halves can each hold a
/// clone of this `Arc` and lock it only for the duration of one encrypt/decrypt.
type Session = Arc<Mutex<TransportState>>;

/// Derive a 32-byte Noise PSK from the user-provided passphrase.
fn derive_psk(psk: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"bluecross-noise-psk-v1");
    hasher.update(psk.as_bytes());
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

async fn write_hs(w: &mut OwnedWriteHalf, data: &[u8]) -> anyhow::Result<()> {
    w.write_all(&(data.len() as u16).to_be_bytes()).await?;
    w.write_all(data).await?;
    w.flush().await?;
    Ok(())
}

async fn read_hs(r: &mut OwnedReadHalf) -> anyhow::Result<Vec<u8>> {
    let mut len = [0u8; 2];
    r.read_exact(&mut len).await?;
    let n = u16::from_be_bytes(len) as usize;
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Perform the client (initiator) side of the handshake.
pub async fn client_handshake(
    stream: TcpStream,
    psk: &str,
) -> anyhow::Result<(SecureReader, SecureWriter)> {
    let (mut read_half, mut write_half) = stream.into_split();
    let key = derive_psk(psk);
    let mut hs = Builder::new(NOISE_PARAMS.parse()?)
        .psk(0, &key)
        .build_initiator()?;

    let mut buf = vec![0u8; MAX_NOISE_MSG];
    // -> psk, e
    let n = hs.write_message(&[], &mut buf)?;
    write_hs(&mut write_half, &buf[..n]).await?;
    // <- e, ee
    let msg = read_hs(&mut read_half).await?;
    let mut scratch = vec![0u8; MAX_NOISE_MSG];
    hs.read_message(&msg, &mut scratch)?;

    let session: Session = Arc::new(Mutex::new(hs.into_transport_mode()?));
    Ok((
        SecureReader::new(read_half, session.clone()),
        SecureWriter::new(write_half, session),
    ))
}

/// Perform the server (responder) side of the handshake.
pub async fn server_handshake(
    stream: TcpStream,
    psk: &str,
) -> anyhow::Result<(SecureReader, SecureWriter)> {
    let (mut read_half, mut write_half) = stream.into_split();
    let key = derive_psk(psk);
    let mut hs = Builder::new(NOISE_PARAMS.parse()?)
        .psk(0, &key)
        .build_responder()?;

    let mut scratch = vec![0u8; MAX_NOISE_MSG];
    // -> psk, e
    let msg = read_hs(&mut read_half).await?;
    hs.read_message(&msg, &mut scratch)?;
    // <- e, ee
    let mut buf = vec![0u8; MAX_NOISE_MSG];
    let n = hs.write_message(&[], &mut buf)?;
    write_hs(&mut write_half, &buf[..n]).await?;

    let session: Session = Arc::new(Mutex::new(hs.into_transport_mode()?));
    Ok((
        SecureReader::new(read_half, session.clone()),
        SecureWriter::new(write_half, session),
    ))
}

/// Decrypting reader over the Noise session.
pub struct SecureReader {
    reader: BufReader<OwnedReadHalf>,
    session: Session,
}

impl SecureReader {
    fn new(read_half: OwnedReadHalf, session: Session) -> Self {
        Self {
            reader: BufReader::new(read_half),
            session,
        }
    }

    /// Read and decrypt the next Noise frame. Returns `Ok(None)` on clean EOF.
    pub async fn read_chunk(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        let mut len = [0u8; 2];
        match self.reader.read_exact(&mut len).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        let n = u16::from_be_bytes(len) as usize;
        if n == 0 {
            anyhow::bail!("empty encrypted frame");
        }
        let mut ciphertext = vec![0u8; n];
        self.reader.read_exact(&mut ciphertext).await?;
        let mut plaintext = vec![0u8; n];
        let m = {
            let mut session = self.session.lock().unwrap();
            session.read_message(&ciphertext, &mut plaintext)?
        };
        plaintext.truncate(m);
        Ok(Some(plaintext))
    }
}

/// Encrypting writer over the Noise session.
pub struct SecureWriter {
    writer: BufWriter<OwnedWriteHalf>,
    session: Session,
}

impl SecureWriter {
    fn new(write_half: OwnedWriteHalf, session: Session) -> Self {
        Self {
            writer: BufWriter::new(write_half),
            session,
        }
    }

    /// Encrypt and buffer `plaintext` as one or more Noise frames.
    pub async fn write_all(&mut self, plaintext: &[u8]) -> anyhow::Result<()> {
        for chunk in plaintext.chunks(MAX_CHUNK) {
            let mut ciphertext = vec![0u8; chunk.len() + TAG_LEN];
            let n = {
                let mut session = self.session.lock().unwrap();
                session.write_message(chunk, &mut ciphertext)?
            };
            self.writer.write_all(&(n as u16).to_be_bytes()).await?;
            self.writer.write_all(&ciphertext[..n]).await?;
        }
        Ok(())
    }

    pub async fn flush(&mut self) -> anyhow::Result<()> {
        self.writer.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Message, MessageType, ProtocolReader, ProtocolWriter};
    use tokio::net::TcpListener;

    const PSK: &str = "correct-horse-battery-staple-001";

    /// Establish a connected (reader, writer) pair on both ends over loopback.
    async fn pair(
        server_psk: &'static str,
        client_psk: &'static str,
    ) -> anyhow::Result<(
        (ProtocolReader, ProtocolWriter),
        (ProtocolReader, ProtocolWriter),
    )> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            server_handshake(stream, server_psk).await
        });
        let cs = TcpStream::connect(addr).await?;
        let client = client_handshake(cs, client_psk).await;
        let server = server.await.unwrap();
        let (sr, sw) = server?;
        let (cr, cw) = client?;
        Ok((
            (ProtocolReader::new(sr), ProtocolWriter::new(sw)),
            (ProtocolReader::new(cr), ProtocolWriter::new(cw)),
        ))
    }

    #[tokio::test]
    async fn roundtrip_small_and_large() {
        let ((mut sr, mut sw), (mut cr, mut cw)) = pair(PSK, PSK).await.unwrap();

        // Small message client -> server.
        cw.send(&Message::key_event(30, 1)).await.unwrap();
        cw.flush().await.unwrap();
        let got = sr.receive().await.unwrap().unwrap();
        assert_eq!(got.msg_type, MessageType::KeyEvent);

        // Large clipboard payload (>64 KiB) exercises multi-frame chunking.
        let big = "x".repeat(200_000);
        cw.send(&Message::clipboard_data(&big)).await.unwrap();
        cw.flush().await.unwrap();
        let got = sr.receive().await.unwrap().unwrap();
        assert_eq!(got.msg_type, MessageType::ClipboardData);
        assert_eq!(got.payload.get("content").unwrap().as_str().unwrap(), big);

        // Server -> client direction works too.
        sw.send(&Message::switch_to_server()).await.unwrap();
        sw.flush().await.unwrap();
        let got = cr.receive().await.unwrap().unwrap();
        assert_eq!(got.msg_type, MessageType::SwitchToServer);
    }

    #[tokio::test]
    async fn wrong_psk_is_rejected() {
        // A mismatched PSK must not yield a usable channel.
        let result = pair(PSK, "totally-different-preshared-key0").await;
        assert!(result.is_err(), "handshake should fail with mismatched PSK");
    }
}

//! Noise_IK_25519_ChaChaPoly_BLAKE2s handshake over a raw TcpStream.
//!
//! Wire framing: each Noise message is preceded by a 2-byte big-endian length.
//! After the handshake completes both sides hold a `snow::TransportState` that
//! encrypts/decrypts arbitrary payloads with the same 2-byte framing.

use anyhow::{bail, Context};
use snow::{Builder, TransportState};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

const NOISE_PATTERN: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
/// Maximum size of a single Noise message (snow's hard limit is 65535).
const MAX_MSG: usize = 65535;

// ── Framed I/O helpers ────────────────────────────────────────────────────────

pub async fn write_frame(stream: &mut TcpStream, data: &[u8]) -> anyhow::Result<()> {
    let len = data.len();
    if len > MAX_MSG { bail!("frame too large: {len}"); }
    stream.write_all(&(len as u16).to_be_bytes()).await?;
    stream.write_all(data).await?;
    Ok(())
}

pub async fn read_frame(stream: &mut TcpStream) -> anyhow::Result<Vec<u8>> {
    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf).await.context("read frame length")?;
    let len = u16::from_be_bytes(len_buf) as usize;
    if len > MAX_MSG { bail!("incoming frame too large: {len}"); }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await.context("read frame body")?;
    Ok(buf)
}

// ── Handshake: Initiator (the machine running `mesh join`) ────────────────────

/// Perform the Noise_IK initiator handshake.
///
/// `our_static`  — our x25519 private key (32 bytes)
/// `their_pub`   — the responder's x25519 public key (from the join code)
///
/// Returns a ready `TransportState` on success.
pub async fn initiator_handshake(
    stream: &mut TcpStream,
    our_static: &[u8; 32],
    their_pub:  &[u8; 32],
) -> anyhow::Result<TransportState> {
    let mut hs = Builder::new(NOISE_PATTERN.parse()?)
        .local_private_key(our_static)
        .remote_public_key(their_pub)
        .build_initiator()?;

    // → msg1 (initiator → responder, ephemeral + static)
    let mut buf = vec![0u8; MAX_MSG];
    let n = hs.write_message(&[], &mut buf)?;
    write_frame(stream, &buf[..n]).await?;

    // ← msg2 (responder → initiator, ephemeral + static + payload)
    let msg2 = read_frame(stream).await?;
    hs.read_message(&msg2, &mut buf)?;

    // Handshake complete — move to transport mode.
    let ts = hs.into_transport_mode()?;
    Ok(ts)
}

// ── Handshake: Responder (the machine that printed the join code) ─────────────

/// Perform the Noise_IK responder handshake.
///
/// Returns `(initiator_pub_key, TransportState)` on success.
pub async fn responder_handshake(
    stream: &mut TcpStream,
    our_static: &[u8; 32],
) -> anyhow::Result<([u8; 32], TransportState)> {
    let mut hs = Builder::new(NOISE_PATTERN.parse()?)
        .local_private_key(our_static)
        .build_responder()?;

    // ← msg1
    let msg1 = read_frame(stream).await?;
    let mut buf = vec![0u8; MAX_MSG];
    hs.read_message(&msg1, &mut buf)?;

    // → msg2
    let n = hs.write_message(&[], &mut buf)?;
    write_frame(stream, &buf[..n]).await?;

    // Extract the remote's static public key before consuming the handshake state.
    let remote_pub: [u8; 32] = hs.get_remote_static()
        .context("responder: no remote static key after handshake")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("remote static key has wrong length"))?;

    let ts = hs.into_transport_mode()?;
    Ok((remote_pub, ts))
}

// ── Transport-mode framed send / recv ─────────────────────────────────────────

/// Encrypt and send one application message over the transport.
pub async fn send_msg(
    stream: &mut TcpStream,
    ts: &mut TransportState,
    plaintext: &[u8],
) -> anyhow::Result<()> {
    let mut cipher = vec![0u8; plaintext.len() + 16 + 2]; // AEAD tag overhead
    let n = ts.write_message(plaintext, &mut cipher)?;
    write_frame(stream, &cipher[..n]).await
}

/// Receive and decrypt one application message from the transport.
pub async fn recv_msg(
    stream: &mut TcpStream,
    ts: &mut TransportState,
) -> anyhow::Result<Vec<u8>> {
    let frame = read_frame(stream).await?;
    let mut plain = vec![0u8; frame.len()]; // decrypted will be shorter (no tag)
    let n = ts.read_message(&frame, &mut plain)?;
    plain.truncate(n);
    Ok(plain)
}

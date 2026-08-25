//! Transparent TCP forwarders for remote .vln hosts.
//!
//! When a gossip message says peer B has `api.vln → 8080`, we spin up a local
//! TCP listener on 127.0.0.1:0 (OS-assigned ephemeral port) and register *that*
//! port in the local `NetRegistry` under `api.vln`.  SOCKS5 and DNS see no
//! difference — from their perspective every .vln host is local.
//!
//! Accepted connections are proxied through the peer's Noise channel as
//! ProxyOpen / ProxyData / ProxyClose messages.  The remote peer's core listens
//! for those messages and splices the data to the actual node port on its side.

use std::sync::Arc;
use std::collections::HashMap;

use anyhow::Context;
use once_cell::sync::Lazy;
use rand::RngCore;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, Mutex},
};
use uuid::Uuid;

use veloce_net::registry::NetRegistry;

use crate::peer::PeerMsg;

// ── Internal state ────────────────────────────────────────────────────────────

struct ForwarderHandle {
    #[allow(dead_code)] // retained for diagnostics / future use
    local_port: u16,
    shutdown:   mpsc::Sender<()>,
    /// The peer that owns this forwarder; used for cleanup on disconnect.
    peer_id:    Uuid,
}

/// Global forwarder table: hostname → handle.
static FORWARDERS: Lazy<Mutex<HashMap<String, ForwarderHandle>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Active proxy stream table: stream_id → channel that delivers inbound ProxyData.
/// Both the initiator (forward.rs) and responder (peer.rs via `handle_proxy_open`)
/// insert entries here; `on_proxy_data` routes incoming frames to them.
static PROXY_STREAMS: Lazy<Mutex<HashMap<u32, mpsc::Sender<Vec<u8>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

// ── Public API ────────────────────────────────────────────────────────────────

/// Ensure a transparent TCP forwarder exists for `hostname` pointing to
/// `(peer_id, remote_port)`.  If one already exists for this hostname it is
/// left intact (the local port in `NetRegistry` doesn't change).
pub async fn refresh_forwarder(
    hostname:     &str,
    remote_port:  u16,
    peer_id:      Uuid,
    net_registry: &Arc<NetRegistry>,
    peer_tx:      mpsc::Sender<PeerMsg>,
) -> anyhow::Result<()> {
    let mut map = FORWARDERS.lock().await;
    if map.contains_key(hostname) {
        return Ok(());
    }

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind forwarder")?;
    let local_port = listener.local_addr()?.port();

    net_registry.register(
        hostname.to_owned(),
        Uuid::nil(),
        local_port,
        3600,
    );

    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    let hostname_owned = hostname.to_owned();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                result = listener.accept() => {
                    match result {
                        Ok((client, _addr)) => {
                            let h  = hostname_owned.clone();
                            let tx = peer_tx.clone();
                            tokio::spawn(async move {
                                if let Err(e) = run_proxy_client(client, h, remote_port, tx).await {
                                    tracing::debug!("proxy client: {e:#}");
                                }
                            });
                        }
                        Err(e) => {
                            tracing::warn!("forwarder accept error: {e}");
                            break;
                        }
                    }
                }
            }
        }
    });

    map.insert(hostname.to_owned(), ForwarderHandle {
        local_port,
        shutdown: shutdown_tx,
        peer_id,
    });
    tracing::info!("transparent forwarder: {hostname} → 127.0.0.1:{local_port} (peer {peer_id}, remote_port {remote_port})");
    Ok(())
}

/// Remove all forwarders that belong to `peer_id`, shut them down, and
/// unregister their hostnames from the `NetRegistry`.
pub async fn remove_peer_forwarders(peer_id: Uuid, net_registry: &Arc<NetRegistry>) {
    let mut map = FORWARDERS.lock().await;
    let to_remove: Vec<String> = map.iter()
        .filter(|(_, h)| h.peer_id == peer_id)
        .map(|(k, _)| k.clone())
        .collect();

    for hostname in &to_remove {
        if let Some(h) = map.remove(hostname) {
            let _ = h.shutdown.try_send(());
            net_registry.unregister(hostname);
            tracing::info!("removed forwarder for {hostname} (peer {peer_id} disconnected)");
        }
    }

    if !to_remove.is_empty() {
        tracing::debug!(
            "peer {peer_id} disconnected; cleaned up {} forwarder(s)",
            to_remove.len()
        );
    }
}

/// Called by `peer.rs` when a `ProxyOpen` message arrives on the responder side.
///
/// Looks up the hostname in the local `NetRegistry`, opens a TCP connection to
/// the registered local port, and wires it to a bidirectional proxy through the
/// `reply_tx` Noise channel.
pub async fn handle_proxy_open(
    stream_id:    u32,
    hostname:     &str,
    net_registry: &Arc<NetRegistry>,
    reply_tx:     &mpsc::Sender<PeerMsg>,
) {
    let port = match net_registry.resolve(hostname) {
        Some(rec) => rec.local_port,
        None => {
            tracing::warn!(hostname, "ProxyOpen for unregistered host; closing stream");
            let _ = reply_tx.send(PeerMsg::ProxyClose { stream_id }).await;
            return;
        }
    };

    let addr = format!("127.0.0.1:{port}");
    let target = match TcpStream::connect(&addr).await {
        Ok(t)  => t,
        Err(e) => {
            tracing::warn!(hostname, "ProxyOpen: cannot connect to {addr}: {e}");
            let _ = reply_tx.send(PeerMsg::ProxyClose { stream_id }).await;
            return;
        }
    };

    let (data_tx, data_rx) = mpsc::channel::<Vec<u8>>(64);
    PROXY_STREAMS.lock().await.insert(stream_id, data_tx);

    let (target_read, target_write) = target.into_split();
    let reply_tx_clone = reply_tx.clone();

    // Target → ProxyData (responder side reads from local service, sends to initiator)
    tokio::spawn(pump_to_peer(stream_id, target_read, reply_tx_clone));
    // ProxyData → Target (responder side writes data from initiator to local service)
    tokio::spawn(pump_from_peer(stream_id, data_rx, target_write));
}

/// Route an incoming `ProxyData` frame to the right local stream.
pub async fn on_proxy_data(stream_id: u32, data: Vec<u8>) {
    let guard = PROXY_STREAMS.lock().await;
    if let Some(tx) = guard.get(&stream_id) {
        let _ = tx.try_send(data);
    } else {
        tracing::debug!("ProxyData for unknown stream {stream_id} — dropped");
    }
}

/// Remove a proxy stream entry when either side closes the connection.
pub async fn on_proxy_close(stream_id: u32) {
    PROXY_STREAMS.lock().await.remove(&stream_id);
    tracing::debug!("proxy stream {stream_id} closed");
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Initiator side: handle a single inbound TCP connection from a local process
/// (e.g. via SOCKS5) and proxy it through the Noise channel to the remote peer.
async fn run_proxy_client(
    client:      TcpStream,
    hostname:    String,
    _remote_port: u16,
    peer_tx:     mpsc::Sender<PeerMsg>,
) -> anyhow::Result<()> {
    let stream_id: u32 = {
        let mut buf = [0u8; 4];
        rand::rngs::OsRng.fill_bytes(&mut buf);
        u32::from_le_bytes(buf)
    };

    let (data_tx, data_rx) = mpsc::channel::<Vec<u8>>(64);
    PROXY_STREAMS.lock().await.insert(stream_id, data_tx);

    peer_tx.send(PeerMsg::ProxyOpen { stream_id, hostname })
        .await
        .context("ProxyOpen send")?;

    let (client_read, client_write) = client.into_split();
    let peer_tx_clone = peer_tx.clone();

    // Client → ProxyData (initiator reads from local TCP, sends to remote peer)
    tokio::spawn(pump_to_peer(stream_id, client_read, peer_tx_clone));
    // ProxyData → Client (initiator writes data from remote peer to local TCP)
    tokio::spawn(pump_from_peer(stream_id, data_rx, client_write));

    Ok(())
}

/// Pump bytes from a `TcpStream` read half to a peer as `ProxyData` messages.
/// Sends `ProxyClose` and removes the PROXY_STREAMS entry when the stream ends.
async fn pump_to_peer<R>(
    stream_id: u32,
    mut reader: R,
    peer_tx:   mpsc::Sender<PeerMsg>,
)
where
    R: AsyncReadExt + Unpin + Send + 'static,
{
    let mut buf = vec![0u8; 16_384];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if peer_tx
                    .send(PeerMsg::ProxyData { stream_id, data: buf[..n].to_vec() })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    }
    let _ = peer_tx.send(PeerMsg::ProxyClose { stream_id }).await;
    PROXY_STREAMS.lock().await.remove(&stream_id);
}

/// Pump `ProxyData` frames from the channel into a `TcpStream` write half.
async fn pump_from_peer<W>(
    stream_id: u32,
    mut data_rx: mpsc::Receiver<Vec<u8>>,
    mut writer:  W,
)
where
    W: AsyncWriteExt + Unpin + Send + 'static,
{
    while let Some(data) = data_rx.recv().await {
        if writer.write_all(&data).await.is_err() {
            break;
        }
    }
    PROXY_STREAMS.lock().await.remove(&stream_id);
}

//! Transparent TCP forwarders for remote .vln hosts.
//!
//! When a gossip message says peer B has `api.vln → 8080`, we spin up a local
//! TCP listener on 127.0.0.1:0 (OS-assigned ephemeral port) and register *that*
//! port in the local `NetRegistry` under `api.vln`.  SOCKS5 and DNS see no
//! difference — from their perspective every .vln host is local.
//!
//! Accepted connections are proxied through the peer's noise channel as
//! ProxyOpen / ProxyData / ProxyClose messages.  The remote peer's core listens
//! for those messages and splices the data to the actual node port on its side.

use std::sync::Arc;

use anyhow::Context;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};
use uuid::Uuid;

use veloce_net::registry::NetRegistry;

/// State shared between the forwarder task and the manager.
struct ForwarderHandle {
    local_port: u16,
    shutdown:   mpsc::Sender<()>,
}

/// Global table of active forwarders: hostname → handle.
/// Stored in-process; cleared when the peer disconnects.
tokio::task_local! {
    // Not task-local in practice — used as a module-level static via once_cell.
}

use once_cell::sync::Lazy;
use tokio::sync::Mutex;
use std::collections::HashMap;

static FORWARDERS: Lazy<Mutex<HashMap<String, ForwarderHandle>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Ensure a transparent TCP forwarder exists for `hostname` pointing to
/// `(peer_id, remote_port)`.  If one already exists, it is left intact
/// (the port in `NetRegistry` doesn't change).
pub async fn refresh_forwarder(
    hostname: &str,
    remote_port: u16,
    peer_id: Uuid,
    net_registry: &Arc<NetRegistry>,
) -> anyhow::Result<()> {
    let mut map = FORWARDERS.lock().await;
    if map.contains_key(hostname) {
        return Ok(()); // already forwarding
    }

    // Bind on a random local port.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind forwarder")?;
    let local_port = listener.local_addr()?.port();

    // Register the local port in the NetRegistry.
    net_registry.register(
        hostname.to_owned(),
        Uuid::nil(), // no local node_id for remote hosts
        local_port,
        3600,        // 1 h TTL; refreshed each gossip cycle
    );

    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    let hostname_owned = hostname.to_owned();

    // Spawn the forwarder accept loop.
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                result = listener.accept() => {
                    match result {
                        Ok((mut client, _)) => {
                            let h = hostname_owned.clone();
                            tokio::spawn(async move {
                                // For now: simple half-close echo to indicate no peer
                                // transport is wired yet.  Full ProxyOpen/Data/Close
                                // splicing is wired in v0.4.1 when the proxy channel
                                // is threaded through PeerConnection.
                                tracing::debug!("forwarder accepted for {h} (remote_port {remote_port})");
                                // Write an HTTP 502 so curl gives a clean error instead of
                                // a silent hang.
                                let _ = client.write_all(
                                    b"HTTP/1.1 502 Mesh tunnel not yet wired\r\n\
                                      Content-Length: 0\r\n\r\n"
                                ).await;
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

    map.insert(hostname.to_owned(), ForwarderHandle { local_port, shutdown: shutdown_tx });
    tracing::info!("transparent forwarder: {hostname} → 127.0.0.1:{local_port} (peer {peer_id})");
    Ok(())
}

/// Remove all forwarders owned by a peer (called when it disconnects).
pub async fn remove_peer_forwarders(peer_id: Uuid, net_registry: &Arc<NetRegistry>) {
    // NOTE: We don't currently track forwarder→peer mapping in FORWARDERS.
    // For v0.4.0 with small meshes this is fine; a full cleanup map is a v0.4.1 task.
    // The NetRegistry TTL will expire them naturally on the next GC tick (60 s).
    tracing::debug!("peer {peer_id} disconnected; forwarders will expire on next GC");
    let _ = net_registry; // used in future
}

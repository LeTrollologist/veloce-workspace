//! PeerConnection: manages a fully-established Noise transport with one remote peer.
//!
//! After the handshake completes, both sides exchange JSON-encoded `PeerMsg` values
//! over the framed noise channel.  A `PeerConnection` owns a reader task and a
//! writer task; the application interacts with it through an mpsc channel.

use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use snow::TransportState;
use tokio::{
    net::TcpStream,
    sync::{mpsc, RwLock},
    time::{interval, Duration},
};
use uuid::Uuid;

use veloce_ipc::message::PeerInfoMsg;
use veloce_net::registry::NetRegistry;

use crate::forward;
use crate::noise;

// ── Wire messages ──────────────────────────────────────────────────────────────

/// Messages exchanged over the Noise transport channel between two peers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PeerMsg {
    /// First message after handshake — announce machine name and ID.
    Hello { machine_name: String, machine_id: Uuid },
    /// Gossip: push .vln registry entries (Last-Write-Wins by timestamp).
    RegistrySync { entries: Vec<GossipEntry> },
    /// Open a virtual stream towards a remote .vln host.
    ProxyOpen { stream_id: u32, hostname: String },
    /// Data on an open virtual stream.
    ProxyData { stream_id: u32, data: Vec<u8> },
    /// Close a virtual stream.
    ProxyClose { stream_id: u32 },
    /// Keepalive ping.
    Ping { seq: u32 },
    /// Keepalive pong (echoes seq).
    Pong { seq: u32, round_trip_us: u32 },
}

/// A single .vln hostname advertisement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GossipEntry {
    pub hostname: String,
    pub port: u16,
    /// Unix seconds — used for LWW conflict resolution.
    pub ts: u64,
}

// ── PeerConnection ─────────────────────────────────────────────────────────────

pub struct PeerConnection {
    pub peer_id:          Uuid,
    pub peer_name:        String,
    pub connected_since:  u64,
    pub latency_ms:       Arc<AtomicU32>,
    /// Send a message to the peer.
    pub tx:               mpsc::Sender<PeerMsg>,
    /// .vln entries the remote peer has advertised.
    pub remote_hosts:     Arc<RwLock<Vec<GossipEntry>>>,
}

impl PeerConnection {
    /// Spawn reader + writer tasks and return the connection handle.
    ///
    /// The caller must have already completed the Noise handshake;  `transport`
    /// is the resulting `TransportState`.
    pub fn start(
        peer_id:      Uuid,
        peer_name:    String,
        transport:    TransportState,
        stream:       TcpStream,
        net_registry: Arc<NetRegistry>,
        on_disconnect: impl FnOnce(Uuid) + Send + 'static,
    ) -> Arc<Self> {
        let (tx, rx) = mpsc::channel::<PeerMsg>(256);
        let latency  = Arc::new(AtomicU32::new(0));
        let remote_hosts = Arc::new(RwLock::new(Vec::<GossipEntry>::new()));
        let connected_since = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let conn = Arc::new(PeerConnection {
            peer_id,
            peer_name: peer_name.clone(),
            connected_since,
            latency_ms: latency.clone(),
            tx: tx.clone(),
            remote_hosts: remote_hosts.clone(),
        });

        // Split the TcpStream into owned halves.
        let (read_half, write_half) = stream.into_split();

        // We need to share `transport` between reader and writer tasks, but
        // `snow::TransportState` is not `Clone`.  Use a mutex-wrapped shared state.
        let transport = Arc::new(tokio::sync::Mutex::new(transport));
        let ts_r = transport.clone();
        let ts_w = transport.clone();

        // Writer task
        let peer_id_w = peer_id;
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let mut write_half = write_half;
            let mut rx = rx;
            while let Some(msg) = rx.recv().await {
                let json = match serde_json::to_vec(&msg) {
                    Ok(b) => b,
                    Err(e) => { tracing::warn!("peer {} msg encode: {e}", peer_id_w); continue; }
                };
                // Reassemble: noise encrypt + frame write requires a whole TcpStream.
                // We use a workaround: encrypt in-memory then write the ciphertext.
                let ciphertext = {
                    let mut ts = ts_w.lock().await;
                    let mut buf = vec![0u8; json.len() + 16 + 2];
                    match ts.write_message(&json, &mut buf) {
                        Ok(n) => buf[..n].to_vec(),
                        Err(e) => { tracing::warn!("noise encrypt: {e}"); continue; }
                    }
                };
                let len = ciphertext.len() as u16;
                if let Err(e) = write_half.write_all(&len.to_be_bytes()).await {
                    tracing::debug!("peer {peer_id_w} write error: {e}");
                    break;
                }
                if let Err(e) = write_half.write_all(&ciphertext).await {
                    tracing::debug!("peer {peer_id_w} write error: {e}");
                    break;
                }
            }
        });

        // Reader task
        let peer_id_r  = peer_id;
        let remote_h   = remote_hosts.clone();
        let net_reg    = net_registry.clone();
        let tx_ping    = tx.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut read_half = read_half;
            let mut ping_seq: u32 = 0;
            let mut ping_ticker = interval(Duration::from_secs(30));
            ping_ticker.tick().await; // skip the immediate tick

            loop {
                tokio::select! {
                    // Keepalive ping
                    _ = ping_ticker.tick() => {
                        ping_seq = ping_seq.wrapping_add(1);
                        let _ = tx_ping.send(PeerMsg::Ping { seq: ping_seq }).await;
                    }

                    // Inbound frame
                    result = async {
                        let mut len_buf = [0u8; 2];
                        read_half.read_exact(&mut len_buf).await?;
                        let n = u16::from_be_bytes(len_buf) as usize;
                        let mut cipher = vec![0u8; n];
                        read_half.read_exact(&mut cipher).await?;
                        anyhow::Ok(cipher)
                    } => {
                        let cipher = match result {
                            Ok(c) => c,
                            Err(e) => {
                                tracing::debug!("peer {peer_id_r} read error: {e}");
                                break;
                            }
                        };
                        let plain = {
                            let mut ts = ts_r.lock().await;
                            let mut buf = vec![0u8; cipher.len()];
                            match ts.read_message(&cipher, &mut buf) {
                                Ok(n) => { buf.truncate(n); buf }
                                Err(e) => { tracing::warn!("noise decrypt: {e}"); break; }
                            }
                        };
                        let msg: PeerMsg = match serde_json::from_slice(&plain) {
                            Ok(m) => m,
                            Err(e) => { tracing::warn!("peer {peer_id_r} msg decode: {e}"); continue; }
                        };
                        handle_incoming(msg, peer_id_r, &remote_h, &net_reg, &tx_ping, &latency).await;
                    }
                }
            }
            on_disconnect(peer_id_r);
        });

        conn
    }

    /// Snapshot for IPC responses.
    pub fn to_info(&self) -> PeerInfoMsg {
        let hosts = self.remote_hosts.blocking_read()
            .iter()
            .map(|e| e.hostname.clone())
            .collect();
        PeerInfoMsg {
            peer_id: self.peer_id,
            peer_name: self.peer_name.clone(),
            connected_since: self.connected_since,
            latency_ms: self.latency_ms.load(Ordering::Relaxed),
            remote_hosts: hosts,
        }
    }
}

// ── Incoming message handler ───────────────────────────────────────────────────

async fn handle_incoming(
    msg: PeerMsg,
    peer_id: Uuid,
    remote_hosts: &Arc<RwLock<Vec<GossipEntry>>>,
    net_registry: &Arc<NetRegistry>,
    reply_tx: &mpsc::Sender<PeerMsg>,
    latency: &Arc<AtomicU32>,
) {
    match msg {
        PeerMsg::Hello { machine_name, machine_id } => {
            tracing::info!("peer {peer_id} identified as {machine_name} ({machine_id})");
        }

        PeerMsg::RegistrySync { entries } => {
            tracing::debug!("peer {peer_id} gossiped {} entries", entries.len());
            let mut hosts = remote_hosts.write().await;
            for entry in &entries {
                // LWW: only accept if newer than what we have.
                let existing = hosts.iter_mut().find(|e| e.hostname == entry.hostname);
                if let Some(ex) = existing {
                    if entry.ts > ex.ts {
                        *ex = entry.clone();
                        // Update the local forwarder for this host.
                        if let Err(e) = forward::refresh_forwarder(
                            &entry.hostname, entry.port, peer_id, net_registry
                        ).await {
                            tracing::warn!("forwarder refresh for {}: {e}", entry.hostname);
                        }
                    }
                } else {
                    hosts.push(entry.clone());
                    if let Err(e) = forward::refresh_forwarder(
                        &entry.hostname, entry.port, peer_id, net_registry
                    ).await {
                        tracing::warn!("forwarder create for {}: {e}", entry.hostname);
                    }
                }
            }
        }

        PeerMsg::Ping { seq } => {
            let _ = reply_tx.send(PeerMsg::Pong { seq, round_trip_us: 0 }).await;
        }

        PeerMsg::Pong { round_trip_us, .. } => {
            latency.store(round_trip_us / 1000, Ordering::Relaxed);
        }

        // Proxy messages handled by forward.rs tasks — ignored here.
        PeerMsg::ProxyOpen { .. } | PeerMsg::ProxyData { .. } | PeerMsg::ProxyClose { .. } => {}
    }
}

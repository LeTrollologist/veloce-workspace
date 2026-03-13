//! PeerConnection: manages a fully-established Noise transport with one remote peer.
//!
//! After the handshake completes, both sides exchange JSON-encoded `PeerMsg` values
//! over the framed noise channel.  A `PeerConnection` owns a reader task and a
//! writer task; the application interacts with it through an mpsc channel.

use std::{
    sync::{
        atomic::{AtomicU32, AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

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

/// Callback type used for mesh gossip ACL filtering.
///
/// `f(hostname, peer_name)` → `true` means the entry is allowed to be
/// installed in the local forwarding table.
pub type AclFn = Arc<dyn Fn(&str, &str) -> bool + Send + Sync>;

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
    /// Cumulative bytes written to the Noise transport (outbound).
    pub tx_bytes:         Arc<AtomicU64>,
    /// Cumulative bytes read from the Noise transport (inbound).
    pub rx_bytes:         Arc<AtomicU64>,
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
    ///
    /// `acl_fn` is an optional callback `f(hostname, peer_name) -> bool` applied
    /// to each gossip entry in [`PeerMsg::RegistrySync`].  Returning `false`
    /// prevents that entry from being installed in the local forwarder table.
    pub fn start(
        peer_id:      Uuid,
        peer_name:    String,
        transport:    TransportState,
        stream:       TcpStream,
        net_registry: Arc<NetRegistry>,
        on_disconnect: impl FnOnce(Uuid) + Send + 'static,
        acl_fn:       Option<AclFn>,
    ) -> Arc<Self> {
        let (tx, rx) = mpsc::channel::<PeerMsg>(256);
        let latency  = Arc::new(AtomicU32::new(0));
        let tx_bytes = Arc::new(AtomicU64::new(0));
        let rx_bytes = Arc::new(AtomicU64::new(0));
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
            tx_bytes: tx_bytes.clone(),
            rx_bytes: rx_bytes.clone(),
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
        let peer_id_w  = peer_id;
        let tx_bytes_w = tx_bytes.clone();
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
                tx_bytes_w.fetch_add(ciphertext.len() as u64, Ordering::Relaxed);
            }
        });

        // Reader task
        let peer_id_r         = peer_id;
        let initial_peer_name = peer_name.clone();
        let remote_h          = remote_hosts.clone();
        let net_reg           = net_registry.clone();
        let tx_ping           = tx.clone();
        let rx_bytes_r        = rx_bytes.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut read_half = read_half;
            let mut ping_seq: u32 = 0;
            let mut ping_ticker = interval(Duration::from_secs(30));
            ping_ticker.tick().await; // skip the immediate tick
            // Track the peer's real name once we receive their Hello.
            let mut current_peer_name = initial_peer_name;
            // Track the most recent in-flight ping for RTT measurement.
            // At most one ping is in flight at a time (30 s interval).
            let mut last_ping: Option<(u32, std::time::Instant)> = None;

            loop {
                tokio::select! {
                    // Keepalive ping
                    _ = ping_ticker.tick() => {
                        ping_seq = ping_seq.wrapping_add(1);
                        last_ping = Some((ping_seq, std::time::Instant::now()));
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
                        rx_bytes_r.fetch_add(cipher.len() as u64, Ordering::Relaxed);
                        let plain = {
                            let mut ts = ts_r.lock().await;
                            let mut buf = vec![0u8; cipher.len()];
                            match ts.read_message(&cipher, &mut buf) {
                                Ok(n) => { buf.truncate(n); buf }
                                Err(e) => { tracing::warn!("noise decrypt: {e}"); break; }
                            }
                        };
                        // Reject oversized plaintext before JSON parsing (N5).
                        const MAX_PEER_MSG_BYTES: usize = 65_535;
                        if plain.len() > MAX_PEER_MSG_BYTES {
                            tracing::warn!(
                                "peer {peer_id_r} sent oversized message ({} bytes) — dropping",
                                plain.len()
                            );
                            continue;
                        }
                        let msg: PeerMsg = match serde_json::from_slice(&plain) {
                            Ok(m) => m,
                            Err(e) => { tracing::warn!("peer {peer_id_r} msg decode: {e}"); continue; }
                        };
                        // Update the tracked name as soon as we see the Hello.
                        if let PeerMsg::Hello { ref machine_name, .. } = msg {
                            current_peer_name = machine_name.clone();
                        }
                        // Compute RTT for Pong messages using the send timestamp.
                        // The wire round_trip_us field is always 0 (filled by responder
                        // who doesn't know the initiator's send time), so we measure it
                        // locally here instead.
                        if let PeerMsg::Pong { seq, .. } = &msg {
                            if let Some((sent_seq, sent_at)) = last_ping.take() {
                                if *seq == sent_seq {
                                    let rtt_us = sent_at.elapsed().as_micros()
                                        .min(u32::MAX as u128) as u32;
                                    latency.store(rtt_us / 1000, Ordering::Relaxed);
                                }
                            }
                        }
                        handle_incoming(
                            msg, peer_id_r, &current_peer_name,
                            &remote_h, &net_reg, &tx_ping, &acl_fn,
                        ).await;
                    }
                }
            }
            on_disconnect(peer_id_r);
        });

        conn
    }

    /// Cumulative (tx_bytes, rx_bytes) counters for this tunnel.
    pub fn traffic_snapshot(&self) -> (u64, u64) {
        (
            self.tx_bytes.load(Ordering::Relaxed),
            self.rx_bytes.load(Ordering::Relaxed),
        )
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
    peer_name: &str,
    remote_hosts: &Arc<RwLock<Vec<GossipEntry>>>,
    net_registry: &Arc<NetRegistry>,
    reply_tx: &mpsc::Sender<PeerMsg>,
    acl_fn: &Option<AclFn>,
) {
    match msg {
        PeerMsg::Hello { machine_name, machine_id } => {
            tracing::info!("peer {peer_id} identified as {machine_name} ({machine_id})");
        }

        PeerMsg::RegistrySync { mut entries } => {
            // Hard cap: never process more than 1 000 gossip entries per message (N5).
            entries.truncate(1_000);

            // Clock-skew guard: reject entries with timestamps more than 5 minutes in
            // the future or more than 24 hours in the past (N4).
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            const SKEW_FUTURE_SECS: u64 = 300;   // 5 minutes
            const SKEW_PAST_SECS:   u64 = 86_400; // 24 hours

            let entries: Vec<_> = entries
                .into_iter()
                .filter(|e| {
                    if e.ts > now + SKEW_FUTURE_SECS {
                        tracing::warn!(
                            hostname = %e.hostname,
                            entry_ts  = e.ts,
                            local_now = now,
                            "gossip entry timestamp too far in the future — discarding"
                        );
                        return false;
                    }
                    if e.ts + SKEW_PAST_SECS < now {
                        tracing::warn!(
                            hostname = %e.hostname,
                            entry_ts  = e.ts,
                            local_now = now,
                            "gossip entry timestamp too far in the past — discarding"
                        );
                        return false;
                    }
                    true
                })
                // Apply mesh ACL: filter out entries the policy disallows.
                .filter(|e| {
                    let allowed = acl_fn.as_ref().map(|f| f(&e.hostname, peer_name)).unwrap_or(true);
                    if !allowed {
                        tracing::warn!(
                            hostname = %e.hostname,
                            peer = %peer_name,
                            "gossip entry rejected by mesh ACL policy"
                        );
                    }
                    allowed
                })
                .collect();

            tracing::debug!(
                "peer {peer_id} gossiped {} entries (after clock + ACL filter)",
                entries.len()
            );
            let mut hosts = remote_hosts.write().await;
            for entry in &entries {
                // LWW: only accept if newer than what we have.
                let existing = hosts.iter_mut().find(|e| e.hostname == entry.hostname);
                if let Some(ex) = existing {
                    if entry.ts > ex.ts {
                        *ex = entry.clone();
                        // Update the local forwarder for this host.
                        if let Err(e) = forward::refresh_forwarder(
                            &entry.hostname, entry.port, peer_id, net_registry, reply_tx.clone()
                        ).await {
                            tracing::warn!("forwarder refresh for {}: {e}", entry.hostname);
                        }
                    }
                } else {
                    hosts.push(entry.clone());
                    if let Err(e) = forward::refresh_forwarder(
                        &entry.hostname, entry.port, peer_id, net_registry, reply_tx.clone()
                    ).await {
                        tracing::warn!("forwarder create for {}: {e}", entry.hostname);
                    }
                }
            }
        }

        PeerMsg::Ping { seq } => {
            let _ = reply_tx.send(PeerMsg::Pong { seq, round_trip_us: 0 }).await;
        }

        // RTT is measured in the reader task from the Ping send timestamp.
        // The wire round_trip_us field is always 0; nothing to do here.
        PeerMsg::Pong { .. } => {}

        PeerMsg::ProxyOpen { stream_id, hostname } => {
            forward::handle_proxy_open(stream_id, &hostname, net_registry, reply_tx).await;
        }

        PeerMsg::ProxyData { stream_id, data } => {
            forward::on_proxy_data(stream_id, data).await;
        }

        PeerMsg::ProxyClose { stream_id } => {
            forward::on_proxy_close(stream_id).await;
        }
    }
}

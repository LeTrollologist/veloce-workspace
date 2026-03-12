//! VeloceNet peer-to-peer mesh layer.
//!
//! Provides encrypted P2P connectivity between multiple `veloce-core` instances
//! using the Noise_IK_25519_ChaChaPoly_BLAKE2s protocol (same crypto as WireGuard).
//! No kernel driver or admin elevation required.
//!
//! ## Quick start
//! 1. Machine A: `veloce-run mesh identity` → prints a join code.
//! 2. Machine B: `veloce-run mesh join <code>` → connects; both machines can now
//!    resolve each other's `.vln` hostnames transparently.

pub mod forward;
pub mod identity;
pub mod noise;
pub mod peer;

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use tokio::{net::TcpListener, sync::RwLock};
use uuid::Uuid;

use veloce_ipc::message::{MeshInfoMsg, MeshConnectResultMsg, PeerInfoMsg};
use veloce_net::registry::NetRegistry;

use identity::MachineIdentity;
use peer::{GossipEntry, PeerConnection, PeerMsg};

pub const DEFAULT_MESH_PORT: u16 = 7474;

// ── MeshState ─────────────────────────────────────────────────────────────────

/// The full mesh state stored inside `CoreState`.
pub struct MeshState {
    pub identity: MachineIdentity,
    pub peers:    RwLock<HashMap<Uuid, Arc<PeerConnection>>>,
    pub listen_port:  u16,
    net_registry: Arc<NetRegistry>,
}

impl MeshState {
    pub fn new(
        identity: MachineIdentity,
        listen_port: u16,
        net_registry: Arc<NetRegistry>,
    ) -> Arc<Self> {
        Arc::new(Self {
            identity,
            peers: RwLock::new(HashMap::new()),
            listen_port,
            net_registry,
        })
    }

    /// The join code that a remote machine should use with `mesh join`.
    pub fn join_code(&self) -> String {
        let addr = SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), // will be replaced by caller with real IP
            self.listen_port,
        );
        self.identity.join_code(addr)
    }

    /// Snapshot for IPC responses.
    pub fn mesh_info(&self) -> MeshInfoMsg {
        let peers = self.peers.blocking_read()
            .values()
            .map(|p| p.to_info())
            .collect();
        MeshInfoMsg {
            machine_id:  self.identity.machine_id,
            join_code:   self.join_code(),
            listen_port: self.listen_port,
            peers,
        }
    }

    /// Connect to a remote peer given a join code.
    pub async fn connect_to_peer(
        self: &Arc<Self>,
        join_code: &str,
    ) -> anyhow::Result<MeshConnectResultMsg> {
        let (their_pub, addr) = MachineIdentity::decode_join_code(join_code)?;

        let mut stream = tokio::net::TcpStream::connect(addr)
            .await
            .with_context(|| format!("connect to mesh peer at {addr}"))?;

        let ts = noise::initiator_handshake(&mut stream, &self.identity.priv_key, &their_pub)
            .await
            .context("Noise_IK initiator handshake")?;

        // Derive stable peer UUID from their public key.
        let peer_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, &their_pub);

        // Exchange Hello messages to learn the peer's machine name.
        // We do this over the now-encrypted transport.
        // (Serialised as JSON, framed with 2-byte length, same as PeerMsg.)
        // We send our Hello via the peer channel once it's created; the reader
        // task will see their Hello and log it.

        let self_clone = Arc::clone(self);
        let conn = PeerConnection::start(
            peer_id,
            format!("peer-{}", &peer_id.simple().to_string()[..8]),
            ts,
            stream,
            Arc::clone(&self.net_registry),
            move |gone_id| {
                let s = self_clone.clone();
                tokio::spawn(async move {
                    s.peers.write().await.remove(&gone_id);
                    tracing::info!("peer {gone_id} disconnected");
                });
            },
        );

        // Send our Hello.
        let _ = conn.tx.send(PeerMsg::Hello {
            machine_name: self.identity.machine_name.clone(),
            machine_id:   self.identity.machine_id,
        }).await;

        // Send our current registry entries as initial gossip.
        let entries = self.local_gossip_entries();
        if !entries.is_empty() {
            let _ = conn.tx.send(PeerMsg::RegistrySync { entries }).await;
        }

        let result = MeshConnectResultMsg {
            peer_id,
            peer_name: conn.peer_name.clone(),
        };

        self.peers.write().await.insert(peer_id, conn);
        Ok(result)
    }

    /// Disconnect from a peer.
    pub async fn disconnect(&self, peer_id: Uuid) -> anyhow::Result<()> {
        let conn = self.peers.write().await.remove(&peer_id)
            .context("peer not found")?;
        forward::remove_peer_forwarders(peer_id, &self.net_registry).await;
        tracing::info!("disconnected from peer {peer_id} ({})", conn.peer_name);
        Ok(())
    }

    pub fn peer_list(&self) -> Vec<PeerInfoMsg> {
        self.peers.blocking_read().values().map(|p| p.to_info()).collect()
    }

    /// Build the gossip payload from our local NetRegistry.
    /// Skips entries registered by the mesh layer itself (port < 1024 would be
    /// suspicious; we identify ours by nil node_id).
    fn local_gossip_entries(&self) -> Vec<GossipEntry> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.net_registry
            .list()
            .into_iter()
            .filter(|(_, rec)| rec.node_id != Uuid::nil()) // skip remote forwarders
            .map(|(hostname, rec)| GossipEntry { hostname, port: rec.local_port, ts })
            .collect()
    }
}

// ── TCP mesh server ───────────────────────────────────────────────────────────

/// Accept inbound peer connections in a background task.
pub async fn run_mesh_server(
    state: Arc<MeshState>,
    listen_port: u16,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
        listen_port,
    ))
    .await
    .with_context(|| format!("bind mesh server on port {listen_port}"))?;

    tracing::info!("VeloceNet mesh server listening on 0.0.0.0:{listen_port}");

    loop {
        let (mut stream, peer_addr) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => { tracing::warn!("mesh accept error: {e}"); continue; }
        };
        tracing::debug!("inbound mesh connection from {peer_addr}");

        let state = Arc::clone(&state);
        tokio::spawn(async move {
            match noise::responder_handshake(&mut stream, &state.identity.priv_key).await {
                Ok((their_pub, ts)) => {
                    let peer_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, &their_pub);
                    let state_clone = Arc::clone(&state);
                    let conn = PeerConnection::start(
                        peer_id,
                        format!("peer-{}", &peer_id.simple().to_string()[..8]),
                        ts,
                        stream,
                        Arc::clone(&state.net_registry),
                        move |gone_id| {
                            let s = state_clone.clone();
                            tokio::spawn(async move {
                                s.peers.write().await.remove(&gone_id);
                            });
                        },
                    );
                    let _ = conn.tx.send(PeerMsg::Hello {
                        machine_name: state.identity.machine_name.clone(),
                        machine_id:   state.identity.machine_id,
                    }).await;
                    state.peers.write().await.insert(peer_id, conn);
                    tracing::info!("accepted peer {peer_id} from {peer_addr}");
                }
                Err(e) => {
                    tracing::warn!("mesh handshake from {peer_addr} failed: {e}");
                }
            }
        });
    }
}

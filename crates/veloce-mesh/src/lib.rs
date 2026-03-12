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
pub mod stun;

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
use peer::{AclFn, GossipEntry, PeerConnection, PeerMsg};

pub const DEFAULT_MESH_PORT: u16 = 7474;

// ── MeshState ─────────────────────────────────────────────────────────────────

/// The full mesh state stored inside `CoreState`.
pub struct MeshState {
    pub identity:    MachineIdentity,
    pub peers:       RwLock<HashMap<Uuid, Arc<PeerConnection>>>,
    pub listen_port: u16,
    net_registry:    Arc<NetRegistry>,
    /// Cache of the current join code (starts as VM1/LAN, upgraded to VM2 once
    /// STUN resolves the WAN IP in a background task).
    join_code_cache: Arc<RwLock<String>>,
    /// Optional mesh ACL callback injected by `veloce-core`.  Called as
    /// `f(hostname, peer_name) -> bool` for each gossip entry; returning
    /// `false` prevents the entry from being installed locally.
    pub mesh_acl_fn: Option<AclFn>,
}

impl MeshState {
    pub fn new(
        identity:    MachineIdentity,
        listen_port: u16,
        net_registry: Arc<NetRegistry>,
        mesh_acl_fn: Option<AclFn>,
    ) -> Arc<Self> {
        // Build the initial VM1 join code using the best local LAN address.
        let lan_ip  = get_local_ip().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let lan_addr = SocketAddr::new(lan_ip, listen_port);
        let initial_code = identity.join_code(lan_addr);
        let join_code_cache = Arc::new(RwLock::new(initial_code));

        // Capture values needed by the background STUN task.
        let cache_clone = Arc::clone(&join_code_cache);
        let pub_key     = identity.pub_key;

        // Spawn background STUN discovery: upgrades the cache to a VM2 code
        // that includes both the LAN address and the WAN address.
        tokio::spawn(async move {
            match stun::discover_external_ip().await {
                Some(wan_ip) if wan_ip != lan_ip => {
                    let wan_addr  = SocketAddr::new(wan_ip, listen_port);
                    let vm2_code  = build_vm2_code(&pub_key, &[lan_addr, wan_addr]);
                    *cache_clone.write().await = vm2_code;
                    tracing::info!(
                        "STUN WAN IP discovered: {wan_ip} — join code upgraded to VM2"
                    );
                }
                Some(wan_ip) => {
                    tracing::debug!("STUN: WAN IP {wan_ip} same as LAN; keeping VM1 code");
                }
                None => {
                    tracing::debug!("STUN: no WAN IP discovered; keeping VM1 code (LAN only)");
                }
            }
        });

        Arc::new(Self {
            identity,
            peers: RwLock::new(HashMap::new()),
            listen_port,
            net_registry,
            join_code_cache,
            mesh_acl_fn,
        })
    }

    /// The join code that a remote machine should use with `mesh join`.
    ///
    /// Returns a VM1 code initially; upgraded to VM2 (multi-address) once
    /// the background STUN task completes.
    pub fn join_code(&self) -> String {
        self.join_code_cache.blocking_read().clone()
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

    /// Connect to a remote peer given a join code (VM1 or VM2).
    ///
    /// For VM2 codes that include multiple addresses (e.g. LAN + WAN), all
    /// addresses are tried concurrently with a staggered start so the fastest
    /// reachable one is used.
    pub async fn connect_to_peer(
        self: &Arc<Self>,
        join_code: &str,
    ) -> anyhow::Result<MeshConnectResultMsg> {
        let (their_pub, addrs) = MachineIdentity::decode_join_code_addrs(join_code)?;

        let mut stream = connect_first(&addrs)
            .await
            .context("could not reach any address in the join code")?;

        let ts = noise::initiator_handshake(&mut stream, &self.identity.priv_key, &their_pub)
            .await
            .context("Noise_IK initiator handshake")?;

        // Derive stable peer UUID from their public key.
        let peer_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, &their_pub);

        let self_clone = Arc::clone(self);
        let acl = self.mesh_acl_fn.clone();
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
            acl,
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
                    let acl = state.mesh_acl_fn.clone();
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
                        acl,
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

// ── Private helpers ───────────────────────────────────────────────────────────

/// Detect the machine's primary outbound LAN IP address.
///
/// Binds a UDP socket towards 8.8.8.8:80 (no actual packet is sent) and
/// reads the chosen local address back from the kernel.
fn get_local_ip() -> Option<IpAddr> {
    use std::net::UdpSocket;
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    Some(sock.local_addr().ok()?.ip())
}

/// Race TCP connects to all provided addresses, returning the first one that
/// succeeds.  Addresses are tried with a 250 ms stagger so that the preferred
/// (first) address gets a head-start but doesn't block indefinitely if it is
/// unreachable.
async fn connect_first(addrs: &[SocketAddr]) -> anyhow::Result<tokio::net::TcpStream> {
    use tokio::task::JoinSet;
    use tokio::time::{sleep, Duration};

    anyhow::ensure!(!addrs.is_empty(), "connect_first: empty address list");

    let mut set: JoinSet<anyhow::Result<tokio::net::TcpStream>> = JoinSet::new();
    for (i, &addr) in addrs.iter().enumerate() {
        let delay = Duration::from_millis(i as u64 * 250);
        set.spawn(async move {
            if !delay.is_zero() {
                sleep(delay).await;
            }
            tokio::net::TcpStream::connect(addr)
                .await
                .with_context(|| format!("TCP connect to {addr}"))
        });
    }

    // Return the first successful stream; cancel the rest.
    let mut last_err = anyhow::anyhow!("all addresses failed");
    while let Some(result) = set.join_next().await {
        match result {
            Ok(Ok(stream)) => {
                set.abort_all();
                return Ok(stream);
            }
            Ok(Err(e)) => { last_err = e; }
            Err(join_err) => { last_err = anyhow::anyhow!("task panic: {join_err}"); }
        }
    }
    Err(last_err)
}

/// Build a VM2 multi-address join code given a raw x25519 public key and
/// a list of addresses.
///
/// This is a free function so it can be called from the background STUN task
/// before `MeshState` is fully constructed.
fn build_vm2_code(pub_key: &[u8; 32], addrs: &[SocketAddr]) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let n = addrs.len().min(255) as u8;
    let mut payload: Vec<u8> = Vec::with_capacity(32 + 1 + addrs.len() * 19 + 8);
    payload.extend_from_slice(pub_key);
    payload.push(n);
    for addr in addrs.iter().take(n as usize) {
        match addr.ip() {
            IpAddr::V4(v4) => {
                payload.push(4u8);
                payload.extend_from_slice(&v4.octets());
            }
            IpAddr::V6(v6) => {
                payload.push(6u8);
                payload.extend_from_slice(&v6.octets());
            }
        }
        payload.extend_from_slice(&addr.port().to_le_bytes());
    }
    payload.extend_from_slice(&ts.to_le_bytes());
    format!("VM2:{}", URL_SAFE_NO_PAD.encode(&payload))
}

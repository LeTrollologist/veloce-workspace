/// TCP port-forward table for `ports:` publish mappings (v1.1 — Veloce Compose).
///
/// Each rule binds a listener on `0.0.0.0:{host_port}` and forwards every
/// accepted TCP connection to `127.0.0.1:{target_port}`.  Forwarding tasks
/// are self-cleaning — they stop when the `_shutdown_tx` oneshot is dropped
/// (i.e. when the `ForwardEntry` is removed from the table).
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use anyhow::{Context, Result};
use veloce_ipc::message::PortForwardEntry;

// ── Internal entry ─────────────────────────────────────────────────────────

struct ForwardEntry {
    info: PortForwardEntry,
    /// Dropping this sender signals the listener task to stop.
    _shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

// ── Public table ───────────────────────────────────────────────────────────

pub struct PortForwardTable {
    forwards: RwLock<HashMap<String, ForwardEntry>>,
}

impl PortForwardTable {
    pub fn new() -> Self {
        Self { forwards: RwLock::new(HashMap::new()) }
    }

    /// Bind `0.0.0.0:{host_port}` and start forwarding to
    /// `127.0.0.1:{target_port}`.  Returns `Err` if a rule with the same
    /// name already exists or if the port cannot be bound.
    pub async fn add(
        self: &Arc<Self>,
        name:        String,
        host_port:   u16,
        target_port: u16,
        node_id:     Option<uuid::Uuid>,
    ) -> Result<PortForwardEntry> {
        {
            let guard = self.forwards.read();
            if guard.contains_key(&name) {
                anyhow::bail!("port forward rule '{}' already exists", name);
            }
        }

        let listener = TcpListener::bind(format!("0.0.0.0:{host_port}"))
            .await
            .with_context(|| format!("bind 0.0.0.0:{host_port}"))?;

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let info = PortForwardEntry { name: name.clone(), host_port, target_port, node_id };
        let info_clone = info.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    accept = listener.accept() => {
                        match accept {
                            Ok((inbound, peer)) => {
                                tracing::debug!(
                                    "port-forward {host_port}→{target_port}: \
                                     accepted from {peer}"
                                );
                                tokio::spawn(forward_conn(inbound, target_port));
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "port-forward {host_port}: accept error: {e}"
                                );
                                break;
                            }
                        }
                    }
                    _ = &mut shutdown_rx => {
                        tracing::debug!(
                            "port-forward {host_port}→{target_port}: shut down"
                        );
                        break;
                    }
                }
            }
        });

        let entry = ForwardEntry { info: info_clone.clone(), _shutdown_tx: shutdown_tx };
        self.forwards.write().insert(name, entry);
        Ok(info_clone)
    }

    /// Remove a rule by name.  Returns `true` if the rule existed.
    /// The listener task stops as soon as the shutdown sender is dropped.
    pub fn remove(&self, name: &str) -> bool {
        self.forwards.write().remove(name).is_some()
    }

    /// Return a snapshot of all active rules.
    pub fn list(&self) -> Vec<PortForwardEntry> {
        self.forwards.read().values().map(|e| e.info.clone()).collect()
    }
}

// ── Per-connection forwarder ───────────────────────────────────────────────

async fn forward_conn(inbound: TcpStream, target_port: u16) {
    let target_addr = format!("127.0.0.1:{target_port}");
    let outbound = match TcpStream::connect(&target_addr).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("port-forward: connect to {target_addr} failed: {e}");
            return;
        }
    };

    let (mut ri, mut wi) = inbound.into_split();
    let (mut ro, mut wo) = outbound.into_split();

    let a = tokio::spawn(async move {
        let _ = tokio::io::copy(&mut ri, &mut wo).await;
    });
    let b = tokio::spawn(async move {
        let _ = tokio::io::copy(&mut ro, &mut wi).await;
    });
    let _ = tokio::join!(a, b);
}

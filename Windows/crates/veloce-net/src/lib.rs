/*!
# veloce-net — VeloceNetwork Userspace Layer

Two servers run in parallel:

1. **DNS resolver** (`0.0.0.0:5354` or `VLN_DNS_PORT`)
   Handles UDP DNS queries for `*.vln` and `*.veloce` TLDs.
   All other queries are forwarded to the system resolver (passthrough).

2. **SOCKS5 proxy** (`127.0.0.1:1055` or `VLN_SOCKS_PORT`)
   TCP SOCKS5 endpoint.  Connections targeting `*.vln` hosts are routed
   to the local port registered with `NetRegistry`.  Everything else
   connects normally through the system stack.

Sideloaded apps configure `VELOCE_DNS=127.0.0.1:5354`
and `VELOCE_SOCKS=127.0.0.1:1055` — no drivers, no kernel modules.
*/

pub mod dns;
pub mod socks5;
pub mod registry;
pub mod error;
pub mod port_forward;

pub use registry::NetRegistry;
pub use error::NetError;
pub use port_forward::PortForwardTable;

use anyhow::Result;
use std::sync::Arc;

pub const DEFAULT_DNS_PORT:   u16 = 5354;
pub const DEFAULT_SOCKS_PORT: u16 = 1055;

/// Start DNS, SOCKS5, and the TTL GC loop.  Runs until any task errors.
pub async fn start(registry: Arc<NetRegistry>) -> Result<()> {
    let dns_registry   = registry.clone();
    let socks_registry = registry.clone();
    let gc_registry    = registry.clone();

    let dns_port   = port_from_env("VLN_DNS_PORT",   DEFAULT_DNS_PORT);
    let socks_port = port_from_env("VLN_SOCKS_PORT", DEFAULT_SOCKS_PORT);
    // VLN_DNS_BIND: interface the DNS server binds to.
    // Default: 127.0.0.1 (localhost only).  Set to 0.0.0.0 for LAN-wide resolution.
    let dns_bind = std::env::var("VLN_DNS_BIND")
        .unwrap_or_else(|_| "127.0.0.1".to_string());
    let dns_bind_log = dns_bind.clone(); // keep a copy for the log line below

    let dns_task = tokio::spawn(async move {
        dns::serve(dns_registry, &dns_bind, dns_port).await
    });
    let socks_task = tokio::spawn(async move {
        socks5::serve(socks_registry, socks_port).await
    });
    // Evict expired TTL entries every 60 seconds
    let gc_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            gc_registry.gc();
            tracing::debug!("VeloceNet: TTL GC pass complete");
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    });

    tracing::info!("VeloceNet DNS    → {dns_bind_log}:{dns_port}");
    tracing::info!("VeloceNet SOCKS5 → 127.0.0.1:{socks_port}");
    tracing::info!("VeloceNet GC     → every 60s");

    tokio::select! {
        r = dns_task   => { r??; }
        r = socks_task => { r??; }
        r = gc_task    => { r??; }
    }

    Ok(())
}

fn port_from_env(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/*!
NetRegistry — thread-safe hostname → local_port mapping with TTL support.

Used by both the DNS resolver and SOCKS5 proxy to route *.vln traffic
to registered local ports without a full IPC round-trip.
*/

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use uuid::Uuid;

// ── RECORD ────────────────────────────────────────────────────────────────────

/// A single registered hostname entry.
#[derive(Debug, Clone)]
pub struct NetRecord {
    /// The node that owns this hostname.
    pub node_id:       Uuid,
    /// TCP port on localhost that the node is listening on.
    pub local_port:    u16,
    /// TTL in seconds.  0 = permanent (until explicitly unregistered).
    pub ttl_secs:      u64,
    /// Wall-clock time when the entry was registered.
    pub registered_at: DateTime<Utc>,
    /// Cumulative bytes proxied through SOCKS5 for this hostname.
    /// Shared via Arc so clones returned by `resolve()` point to the same counter.
    pub bytes_proxied: Arc<AtomicU64>,
}

// ── REGISTRY ──────────────────────────────────────────────────────────────────

/// Thread-safe in-memory hostname → `NetRecord` table.
pub struct NetRegistry {
    entries: RwLock<HashMap<String, NetRecord>>,
}

impl NetRegistry {
    pub fn new() -> Self {
        Self { entries: RwLock::new(HashMap::new()) }
    }

    /// Register or update a *.vln hostname.
    ///
    /// `ttl_secs == 0` means the entry is permanent until explicitly removed.
    pub fn register(
        &self,
        hostname:   String,
        node_id:    Uuid,
        local_port: u16,
        ttl_secs:   u64,
    ) {
        self.entries.write().insert(hostname, NetRecord {
            node_id,
            local_port,
            ttl_secs,
            registered_at: Utc::now(),
            bytes_proxied: Arc::new(AtomicU64::new(0)),
        });
    }

    /// Remove a registered hostname.
    pub fn unregister(&self, hostname: &str) {
        self.entries.write().remove(hostname);
    }

    /// Look up a hostname.  Returns `None` if not registered or TTL expired.
    pub fn resolve(&self, hostname: &str) -> Option<NetRecord> {
        let guard = self.entries.read();
        let rec = guard.get(hostname)?;
        if is_expired(rec) { return None; }
        Some(rec.clone())
    }

    /// Returns `true` if the hostname belongs to a VeloceNet private TLD.
    ///
    /// Does not check whether the hostname is actually registered — only
    /// whether it *looks* like a VLN name.  Used by the DNS and SOCKS5
    /// servers to decide routing without a full resolve round-trip.
    pub fn is_vln(&self, hostname: &str) -> bool {
        hostname.ends_with(".vln") || hostname.ends_with(".veloce")
    }

    /// Evict all entries whose TTL has elapsed.
    /// Called periodically by the GC loop in `veloce_net::start`.
    pub fn gc(&self) {
        self.entries.write().retain(|_host, rec| !is_expired(rec));
    }

    /// Snapshot of all current entries (including possibly-expired ones).
    /// Callers should filter as needed.
    pub fn list(&self) -> Vec<(String, NetRecord)> {
        self.entries
            .read()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Snapshot of per-hostname cumulative byte counters for traffic reporting.
    pub fn traffic_snapshot(&self) -> Vec<veloce_ipc::message::HostTrafficMsg> {
        self.entries
            .read()
            .iter()
            .map(|(hostname, rec)| veloce_ipc::message::HostTrafficMsg {
                hostname:      hostname.clone(),
                bytes_proxied: rec.bytes_proxied.load(Ordering::Relaxed),
            })
            .collect()
    }
}

impl Default for NetRegistry {
    fn default() -> Self { Self::new() }
}

// ── HELPERS ───────────────────────────────────────────────────────────────────

fn is_expired(rec: &NetRecord) -> bool {
    if rec.ttl_secs == 0 { return false; }
    let elapsed = (Utc::now() - rec.registered_at).num_seconds();
    elapsed < 0 || elapsed as u64 >= rec.ttl_secs
}

/*!
# P2P Replicated Key-Value Store (v3.5)

Provides decentralized, causal CRDT-based key-value replication
over the encrypted Noise_IK mesh overlay.
*/

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KvEntry {
    pub key: String,
    pub value: String,
    pub version: u64,
    pub updated_at: u64,
    pub origin: Uuid,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LeaseLock {
    pub key: String,
    pub holder: String,
    pub fence_token: u64,
    pub acquired_at: u64,
    pub expires_at: u64,
}

pub struct MeshKvStore {
    local_peer_id: Uuid,
    entries: RwLock<HashMap<String, KvEntry>>,
    locks: RwLock<HashMap<String, LeaseLock>>,
    next_fence: std::sync::atomic::AtomicU64,
}

impl MeshKvStore {
    pub fn new(local_peer_id: Uuid) -> Arc<Self> {
        Arc::new(Self {
            local_peer_id,
            entries: RwLock::new(HashMap::new()),
            locks: RwLock::new(HashMap::new()),
            next_fence: std::sync::atomic::AtomicU64::new(1),
        })
    }

    /// Set a key-value pair locally and produce a replication update.
    pub fn set(&self, key: &str, value: &str) -> KvEntry {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut map = self.entries.write();
        let version = map.get(key).map(|e| e.version + 1).unwrap_or(1);

        let entry = KvEntry {
            key: key.to_owned(),
            value: value.to_owned(),
            version,
            updated_at: now,
            origin: self.local_peer_id,
            deleted: false,
        };

        map.insert(key.to_owned(), entry.clone());
        entry
    }

    /// Atomic Compare-And-Swap (CAS) for strong consistency operations.
    /// Returns `(success, current_value, version)`.
    pub fn cas(&self, key: &str, expected_value: Option<&str>, new_value: &str) -> (bool, Option<String>, u64) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut map = self.entries.write();
        let current = map.get(key).and_then(|e| if e.deleted { None } else { Some(e.value.clone()) });
        let current_version = map.get(key).map(|e| e.version).unwrap_or(0);

        let matches = match (expected_value, &current) {
            (None, None) => true,
            (Some(exp), Some(cur)) => exp == cur.as_str(),
            _ => false,
        };

        if matches {
            let version = current_version + 1;
            let entry = KvEntry {
                key: key.to_owned(),
                value: new_value.to_owned(),
                version,
                updated_at: now,
                origin: self.local_peer_id,
                deleted: false,
            };
            map.insert(key.to_owned(), entry);
            (true, Some(new_value.to_owned()), version)
        } else {
            (false, current, current_version)
        }
    }

    /// Acquire or renew a distributed lease lock on a key with fencing token.
    /// Returns `(acquired, fence_token, expires_at)`.
    pub fn acquire_lock(&self, key: &str, holder: &str, ttl_secs: u64) -> (bool, u64, u64) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut map = self.locks.write();
        let effective_ttl = ttl_secs.max(1);

        if let Some(lock) = map.get_mut(key) {
            if lock.expires_at > now && lock.holder != holder {
                // Lock is actively held by someone else
                return (false, 0, lock.expires_at);
            }
            // Either expired or held by same holder -> renew / acquire
            let fence_token = if lock.holder == holder && lock.expires_at > now {
                lock.fence_token
            } else {
                self.next_fence.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            };
            lock.holder = holder.to_owned();
            lock.fence_token = fence_token;
            lock.acquired_at = now;
            lock.expires_at = now + effective_ttl;
            (true, fence_token, lock.expires_at)
        } else {
            let fence_token = self.next_fence.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let expires_at = now + effective_ttl;
            let lock = LeaseLock {
                key: key.to_owned(),
                holder: holder.to_owned(),
                fence_token,
                acquired_at: now,
                expires_at,
            };
            map.insert(key.to_owned(), lock);
            (true, fence_token, expires_at)
        }
    }

    /// Release an existing distributed lease lock using holder identity and fencing token.
    pub fn release_lock(&self, key: &str, holder: &str, fence_token: u64) -> bool {
        let mut map = self.locks.write();
        if let Some(lock) = map.get(key) {
            if lock.holder == holder && (lock.fence_token == fence_token || fence_token == 0) {
                map.remove(key);
                return true;
            }
        }
        false
    }

    /// Get current lock status if present.
    pub fn get_lock(&self, key: &str) -> Option<LeaseLock> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let map = self.locks.read();
        map.get(key).and_then(|l| {
            if l.expires_at > now {
                Some(l.clone())
            } else {
                None
            }
        })
    }

    /// Retrieve a value by key if present and not deleted.
    pub fn get(&self, key: &str) -> Option<String> {
        let map = self.entries.read();
        map.get(key).and_then(|e| {
            if e.deleted { None } else { Some(e.value.clone()) }
        })
    }

    /// Mark a key as deleted (tombstone) and produce a replication update.
    pub fn delete(&self, key: &str) -> Option<KvEntry> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut map = self.entries.write();
        if let Some(existing) = map.get_mut(key) {
            if existing.deleted {
                return None;
            }
            existing.version += 1;
            existing.updated_at = now;
            existing.origin = self.local_peer_id;
            existing.deleted = true;
            Some(existing.clone())
        } else {
            None
        }
    }

    /// Return all active (non-deleted) key-value entries.
    pub fn list(&self) -> Vec<KvEntry> {
        let map = self.entries.read();
        let mut list: Vec<KvEntry> = map.values()
            .filter(|e| !e.deleted)
            .cloned()
            .collect();
        list.sort_by(|a, b| a.key.cmp(&b.key));
        list
    }

    /// Merge an incoming remote entry using Last-Write-Wins (LWW) resolution.
    /// Returns `true` if the entry updated local state.
    pub fn merge_entry(&self, entry: KvEntry) -> bool {
        let mut map = self.entries.write();
        if let Some(existing) = map.get(&entry.key) {
            let is_newer = entry.version > existing.version
                || (entry.version == existing.version && entry.updated_at > existing.updated_at)
                || (entry.version == existing.version && entry.updated_at == existing.updated_at && entry.origin > existing.origin);

            if is_newer {
                map.insert(entry.key.clone(), entry);
                true
            } else {
                false
            }
        } else {
            map.insert(entry.key.clone(), entry);
            true
        }
    }

    /// Get all entries (including tombstones) for initial peer state synchronization.
    pub fn snapshot(&self) -> Vec<KvEntry> {
        self.entries.read().values().cloned().collect()
    }

    /// Merge a full snapshot from a newly connected peer.
    pub fn merge_snapshot(&self, snapshot: Vec<KvEntry>) -> usize {
        let mut updated = 0;
        for entry in snapshot {
            if self.merge_entry(entry) {
                updated += 1;
            }
        }
        updated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mesh_kv_crud_and_lww_merge() {
        let node_a = Uuid::new_v4();
        let node_b = Uuid::new_v4();

        let store_a = MeshKvStore::new(node_a);
        let store_b = MeshKvStore::new(node_b);

        // Set on node A
        let entry_a = store_a.set("cluster.config", "enabled=true");
        assert_eq!(store_a.get("cluster.config"), Some("enabled=true".into()));

        // Merge to node B
        assert!(store_b.merge_entry(entry_a.clone()));
        assert_eq!(store_b.get("cluster.config"), Some("enabled=true".into()));

        // Update on node B
        let entry_b = store_b.set("cluster.config", "enabled=false");
        assert_eq!(entry_b.version, 2);

        // Merge back to node A
        assert!(store_a.merge_entry(entry_b));
        assert_eq!(store_a.get("cluster.config"), Some("enabled=false".into()));

        // Stale update from node A should be rejected
        assert!(!store_b.merge_entry(entry_a));

        // Deletion
        let del_entry = store_a.delete("cluster.config").unwrap();
        assert_eq!(store_a.get("cluster.config"), None);
        assert!(store_b.merge_entry(del_entry));
        assert_eq!(store_b.get("cluster.config"), None);
    }

    #[test]
    fn test_mesh_kv_cas_and_leases() {
        let node_a = Uuid::new_v4();
        let store = MeshKvStore::new(node_a);

        // CAS create from None
        let (ok, val, ver) = store.cas("leader.lock", None, "worker-1");
        assert!(ok);
        assert_eq!(val, Some("worker-1".into()));
        assert_eq!(ver, 1);

        // CAS with wrong expected fails
        let (ok, val, _) = store.cas("leader.lock", Some("worker-2"), "worker-3");
        assert!(!ok);
        assert_eq!(val, Some("worker-1".into()));

        // CAS with correct expected succeeds
        let (ok, val, ver) = store.cas("leader.lock", Some("worker-1"), "worker-2");
        assert!(ok);
        assert_eq!(val, Some("worker-2".into()));
        assert_eq!(ver, 2);

        // Distributed Lease Lock
        let (acquired, fence1, exp1) = store.acquire_lock("master.lease", "node-1", 10);
        assert!(acquired);
        assert!(fence1 > 0);
        assert!(exp1 > 0);

        // Other node cannot acquire active lease
        let (acquired2, fence2, _) = store.acquire_lock("master.lease", "node-2", 10);
        assert!(!acquired2);
        assert_eq!(fence2, 0);

        // Same holder can renew
        let (renewed, fence_renew, _) = store.acquire_lock("master.lease", "node-1", 20);
        assert!(renewed);
        assert_eq!(fence_renew, fence1);

        // Release lock
        assert!(store.release_lock("master.lease", "node-1", fence1));
        assert_eq!(store.get_lock("master.lease"), None);
    }
}

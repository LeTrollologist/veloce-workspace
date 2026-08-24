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

pub struct MeshKvStore {
    local_peer_id: Uuid,
    entries: RwLock<HashMap<String, KvEntry>>,
}

impl MeshKvStore {
    pub fn new(local_peer_id: Uuid) -> Arc<Self> {
        Arc::new(Self {
            local_peer_id,
            entries: RwLock::new(HashMap::new()),
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
}

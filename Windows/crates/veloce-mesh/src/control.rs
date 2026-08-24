/*!
# veloce-mesh Control Plane — Multi-Node Consensus & Scheduling (v3.0)

Provides cluster coordination, leader state tracking, and distributed replica
assignment across connected mesh peer nodes.
*/

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;
use uuid::Uuid;

/// Node's role in the multi-node mesh cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterRole {
    Follower,
    Candidate,
    Leader,
}

/// Cluster coordinator managing leadership state and distributed workload assignments.
pub struct ClusterCoordinator {
    local_id:     Uuid,
    current_term: AtomicU64,
    leader_id:    RwLock<Option<Uuid>>,
    role:         RwLock<ClusterRole>,
}

impl ClusterCoordinator {
    pub fn new(local_id: Uuid) -> Self {
        Self {
            local_id,
            current_term: AtomicU64::new(0),
            leader_id:    RwLock::new(None),
            role:         RwLock::new(ClusterRole::Follower),
        }
    }

    /// Return true if the local node is currently the cluster leader.
    pub async fn is_leader(&self) -> bool {
        *self.role.read().await == ClusterRole::Leader
    }

    /// Return the currently recognized cluster leader UUID, if any.
    pub async fn current_leader(&self) -> Option<Uuid> {
        *self.leader_id.read().await
    }

    /// Return the current cluster role.
    pub async fn role(&self) -> ClusterRole {
        *self.role.read().await
    }

    /// Return the current election term.
    pub fn term(&self) -> u64 {
        self.current_term.load(Ordering::SeqCst)
    }

    /// Step up as leader (or candidate) and increment the term.
    pub async fn promote_to_leader(&self) -> u64 {
        let new_term = self.current_term.fetch_add(1, Ordering::SeqCst) + 1;
        *self.role.write().await = ClusterRole::Leader;
        *self.leader_id.write().await = Some(self.local_id);
        tracing::info!(term = new_term, "Node assumed cluster leadership");
        new_term
    }

    /// Process a heartbeat or leader announcement from a peer node.
    pub async fn handle_heartbeat(&self, leader_id: Uuid, term: u64) -> bool {
        let current = self.current_term.load(Ordering::SeqCst);
        if term >= current {
            self.current_term.store(term, Ordering::SeqCst);
            *self.leader_id.write().await = Some(leader_id);
            if leader_id != self.local_id {
                *self.role.write().await = ClusterRole::Follower;
            }
            true
        } else {
            false
        }
    }

    /// Compute distributed replica allocations for a service across all active cluster nodes.
    ///
    /// Returns a list of `(node_id, replica_count)` pairs.
    pub fn assign_replicas(
        &self,
        _service_name: &str,
        desired_count: usize,
        peer_ids: &[Uuid],
    ) -> Vec<(Uuid, usize)> {
        if desired_count == 0 {
            return Vec::new();
        }

        let mut cluster_nodes = Vec::with_capacity(peer_ids.len() + 1);
        cluster_nodes.push(self.local_id);
        cluster_nodes.extend_from_slice(peer_ids);
        cluster_nodes.sort(); // deterministic ordering

        let node_count = cluster_nodes.len();
        let base_count = desired_count / node_count;
        let remainder = desired_count % node_count;

        let mut allocations = Vec::with_capacity(node_count);
        for (i, node) in cluster_nodes.into_iter().enumerate() {
            let count = base_count + if i < remainder { 1 } else { 0 };
            if count > 0 {
                allocations.push((node, count));
            }
        }

        allocations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_coordinator_leadership_and_heartbeat() {
        let node_a = Uuid::new_v4();
        let node_b = Uuid::new_v4();

        let coord_a = ClusterCoordinator::new(node_a);
        let coord_b = ClusterCoordinator::new(node_b);

        assert_eq!(coord_a.role().await, ClusterRole::Follower);
        assert!(!coord_a.is_leader().await);

        // Node A assumes leadership
        let term_1 = coord_a.promote_to_leader().await;
        assert_eq!(term_1, 1);
        assert!(coord_a.is_leader().await);
        assert_eq!(coord_a.current_leader().await, Some(node_a));

        // Node B receives heartbeat from Node A
        assert!(coord_b.handle_heartbeat(node_a, term_1).await);
        assert_eq!(coord_b.role().await, ClusterRole::Follower);
        assert_eq!(coord_b.current_leader().await, Some(node_a));
        assert_eq!(coord_b.term(), 1);

        // Stale heartbeat rejection
        assert!(!coord_b.handle_heartbeat(node_a, 0).await);
    }

    #[test]
    fn test_replica_assignment() {
        let node_a = Uuid::from_u128(1);
        let node_b = Uuid::from_u128(2);
        let node_c = Uuid::from_u128(3);

        let coord = ClusterCoordinator::new(node_a);

        // 5 replicas across 3 nodes -> 2 on node 1, 2 on node 2, 1 on node 3
        let allocs = coord.assign_replicas("web-svc", 5, &[node_b, node_c]);
        assert_eq!(allocs.len(), 3);
        let total: usize = allocs.iter().map(|(_, c)| *c).sum();
        assert_eq!(total, 5);

        // 0 replicas -> empty
        let empty = coord.assign_replicas("web-svc", 0, &[node_b]);
        assert!(empty.is_empty());
    }
}

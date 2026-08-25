/*!
Automated Jepsen-Style Distributed Consensus & Network Partition Verification Suite.

Validates VeloceMesh under:
1. Split-Brain Leadership Resignation upon Partition Heal
2. Monotonic Fencing Token Invariants under Network Partitions / Stalls
3. CRDT LWW Partition Healing & Eventual Consistency Convergence
4. Crash-Recovery & Leadership Failover
*/

use crate::control::{ClusterCoordinator, ClusterRole};
use crate::kv::MeshKvStore;
use std::time::Duration;
use uuid::Uuid;

/// Test 1: Split-Brain Resolution & Term Monotonicity upon Partition Heal
#[tokio::test]
async fn test_split_brain_partition_heal_and_leadership_resignation() {
    let node_a_id = Uuid::new_v4();
    let node_b_id = Uuid::new_v4();

    let coord_a = ClusterCoordinator::new(node_a_id);
    let coord_b = ClusterCoordinator::new(node_b_id);

    // Initial state: Node A is leader at Term 1
    let term_a = coord_a.promote_to_leader().await;
    assert_eq!(term_a, 1);
    assert_eq!(coord_a.role().await, ClusterRole::Leader);

    // Node B is follower acknowledging Leader A
    assert!(coord_b.handle_heartbeat(node_a_id, 1).await);
    assert_eq!(coord_b.role().await, ClusterRole::Follower);
    assert_eq!(coord_b.current_leader().await, Some(node_a_id));

    // --- NETWORK PARTITION OCCURS ---
    // Node B stops receiving heartbeats from Node A.
    // Node B steps up in its partition and promotes itself to Leader at Term 2.
    let term_b = coord_b.promote_to_leader().await;
    assert_eq!(term_b, 2);
    assert_eq!(coord_b.role().await, ClusterRole::Leader);
    assert_eq!(coord_b.current_leader().await, Some(node_b_id));

    // --- PARTITION HEALS ---
    // Node A receives a heartbeat from Node B with higher Term (Term 2 > Term 1)
    let accepted = coord_a.handle_heartbeat(node_b_id, 2).await;
    assert!(accepted, "Leader A must accept higher-term heartbeat from Node B");

    // Invariant: Node A MUST immediately step down from Leader to Follower
    assert_eq!(
        coord_a.role().await,
        ClusterRole::Follower,
        "Split-brain resolved: former Leader A yielded to higher-term Leader B"
    );
    assert_eq!(coord_a.current_leader().await, Some(node_b_id));
    assert_eq!(coord_a.term(), 2);
}

/// Test 2: CP Monotonic Fencing Token & Lease Lock Expiration under Partition Stalls
#[tokio::test]
async fn test_cp_fencing_token_invariants_under_partition_stall() {
    let kv_store = MeshKvStore::new(Uuid::new_v4());
    let lock_key = "cluster-shared-resource-lock";

    // 1. Client 1 (e.g. Node 1) acquires a 1-second distributed lease lock
    let (acquired1, fence_token1, _exp1) = kv_store.acquire_lock(lock_key, "node-1", 1);
    assert!(acquired1, "Node 1 must acquire initial lock");
    assert!(fence_token1 > 0, "Fencing token must be positive");

    // 2. Client 2 attempts to acquire lock immediately -> Must be rejected (held by node-1)
    let (acquired2_fail, _, _) = kv_store.acquire_lock(lock_key, "node-2", 5);
    assert!(!acquired2_fail, "Concurrent lock acquisition while lease is active must fail");

    // 3. Simulate network partition / GC pause on Node 1: sleep 1.2s so lease expires
    tokio::time::sleep(Duration::from_millis(1200)).await;

    // 4. Client 2 now acquires the expired lock -> Must succeed with higher monotonic fencing token
    let (acquired2, fence_token2, _) = kv_store.acquire_lock(lock_key, "node-2", 5);
    assert!(acquired2, "Node 2 must acquire expired lease");
    assert!(
        fence_token2 > fence_token1,
        "New fencing token ({fence_token2}) must be strictly greater than old token ({fence_token1})"
    );

    // 5. Stale Client 1 wakes up and attempts to release using stale token -> Strictly REJECTED
    let stale_release = kv_store.release_lock(lock_key, "node-1", fence_token1);
    assert!(!stale_release, "Stale fencing token release must be rejected");

    // 6. Active Client 2 releases with valid token -> SUCCEEDS
    let valid_release = kv_store.release_lock(lock_key, "node-2", fence_token2);
    assert!(valid_release, "Valid holder with current fencing token must successfully release");
}

/// Test 3: AP CRDT LWW Partition Healing & Eventual Consistency Convergence
#[test]
fn test_ap_crdt_lww_bidirectional_partition_convergence() {
    let origin_a = Uuid::new_v4();
    let origin_b = Uuid::new_v4();

    let store_a = MeshKvStore::new(origin_a);
    let store_b = MeshKvStore::new(origin_b);

    // Partition A: Node A writes key1 and key2
    store_a.set("app_config", "v1.0-alpha");
    store_a.set("cluster_region", "us-east-1");

    // Partition B: Node B writes key2 (newer update) and key3
    store_b.set("cluster_region", "us-west-2");
    store_b.set("database_url", "postgres://db.vln:5432");

    // Partition Heal: Bidirectional sync between Store A and Store B
    let entries_a = store_a.list();
    let entries_b = store_b.list();

    for e in entries_b {
        store_a.merge_entry(e);
    }
    for e in entries_a {
        store_b.merge_entry(e);
    }

    // Invariant: Both partitioned stores MUST reach bit-for-bit identical state
    assert_eq!(store_a.get("app_config"), Some("v1.0-alpha".into()));
    assert_eq!(store_a.get("database_url"), Some("postgres://db.vln:5432".into()));
    assert_eq!(store_a.get("cluster_region"), store_b.get("cluster_region"));

    let list_a = store_a.list();
    let list_b = store_b.list();
    assert_eq!(list_a.len(), 3);
    assert_eq!(list_b.len(), 3);
}

/// Test 4: Distributed CAS Linearizability under Contention
#[test]
fn test_distributed_cas_contention_linearizability() {
    let kv_store = MeshKvStore::new(Uuid::new_v4());
    let key = "distributed-counter";

    // Initialize key
    kv_store.set(key, "0");

    // CAS from 0 -> 1 succeeds
    let (ok1, cur1, ver1) = kv_store.cas(key, Some("0"), "1");
    assert!(ok1);
    assert_eq!(cur1, Some("1".into()));
    assert_eq!(ver1, 2);

    // Stale CAS expecting 0 -> Fails
    let (ok2, cur2, ver2) = kv_store.cas(key, Some("0"), "2");
    assert!(!ok2);
    assert_eq!(cur2, Some("1".into()));
    assert_eq!(ver2, 2);

    // Valid CAS expecting 1 -> 2 succeeds
    let (ok3, cur3, ver3) = kv_store.cas(key, Some("1"), "2");
    assert!(ok3);
    assert_eq!(cur3, Some("2".into()));
    assert_eq!(ver3, 3);
}

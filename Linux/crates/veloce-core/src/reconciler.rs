/// Desired-state reconciler for Veloce Compose (v1.3).
///
/// The reconciler holds a `DesiredStateSpec` set via `ApplyDesiredState` IPC
/// and runs a background loop every 5 seconds that brings actual running
/// nodes into alignment with the spec.
///
/// Supports two update strategies:
/// - `Recreate`      — kill all replicas, then spawn fresh ones.
/// - `RollingUpdate` — replace one replica at a time, waiting for `Healthy`
///                     before moving on.
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use uuid::Uuid;

use veloce_ipc::message::{
    DependsOnCondition, DesiredStateSpec, HealthStatus, ServiceSpec,
};

use crate::state::CoreState;

// ── Public type ────────────────────────────────────────────────────────────

pub struct Reconciler {
    state:   OnceLock<Arc<CoreState>>,
    desired: RwLock<Option<DesiredStateSpec>>,
}

impl Reconciler {
    pub fn new(state: Arc<CoreState>) -> Arc<Self> {
        let r = Arc::new(Self { state: OnceLock::new(), desired: RwLock::new(None) });
        let _ = r.state.set(state);
        r
    }

    /// Create a Reconciler without a `CoreState` reference.
    /// Call `wire_state` before `run` to connect the reconciler to the shared state.
    pub fn new_detached() -> Self {
        Self { state: OnceLock::new(), desired: RwLock::new(None) }
    }

    /// Wire the CoreState after construction (used to break the circular-ref cycle).
    pub fn wire_state(&self, state: Arc<CoreState>) {
        let _ = self.state.set(state);
    }

    fn state(&self) -> &Arc<CoreState> {
        self.state.get().expect("Reconciler::wire_state must be called before run()")
    }

    /// Replace the current desired state.  The next reconcile pass will act
    /// on the new spec.
    pub fn apply(&self, spec: DesiredStateSpec) {
        tracing::info!("reconciler: applying desired state '{}'", spec.name);
        *self.desired.write() = Some(spec);
    }

    /// Clear the desired state — the reconciler will stop managing nodes.
    pub fn clear(&self) {
        *self.desired.write() = None;
    }

    /// Background loop.  Wakes every 5 seconds and calls `reconcile_once`.
    pub async fn run(self: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            self.reconcile_once().await;
        }
    }

    // ── Private ───────────────────────────────────────────────────────────

    async fn reconcile_once(&self) {
        let spec = match self.desired.read().clone() {
            Some(s) => s,
            None    => return,
        };

        // Build a map of service_name → live node UUIDs
        let live_by_service = self.live_nodes_by_service();

        // Topological sort so dependencies start first
        let order = match topo_sort(&spec.services) {
            Ok(o)  => o,
            Err(e) => {
                tracing::warn!("reconciler: topo_sort failed: {e}");
                return;
            }
        };

        for svc_name in &order {
            let svc = match spec.services.iter().find(|s| &s.service_name == svc_name) {
                Some(s) => s,
                None    => continue,
            };

            // Check depends_on conditions
            if !self.deps_satisfied(svc, &live_by_service) {
                tracing::debug!(
                    "reconciler: '{}' waiting on dependencies — skipping",
                    svc_name
                );
                continue;
            }

            let live: Vec<Uuid> = live_by_service
                .get(svc_name.as_str())
                .cloned()
                .unwrap_or_default();
            let running = live.len() as u32;
            let desired = svc.replicas;

            if running < desired {
                // Scale up — spawn missing replicas
                let missing = (desired - running) as usize;
                tracing::info!(
                    "reconciler: '{}' has {running}/{desired} replicas — \
                     spawning {missing}",
                    svc_name
                );
                for _ in 0..missing {
                    self.spawn_replica(svc).await;
                }
            } else if running > desired {
                // Scale down
                let excess = (running - desired) as usize;
                tracing::info!(
                    "reconciler: '{}' has {running}/{desired} replicas — \
                     removing {excess}",
                    svc_name
                );
                let to_kill: Vec<Uuid> = live.into_iter().take(excess).collect();
                for id in to_kill {
                    self.kill_node(id).await;
                }
            } else {
                tracing::debug!(
                    "reconciler: '{}' is at desired replica count ({desired})",
                    svc_name
                );
            }
        }
    }

    /// Spawn one replica for a service.
    async fn spawn_replica(&self, svc: &ServiceSpec) {
        let mut spawn_msg = svc.spawn_msg.clone();
        // Let service_name / replica_index be set by job.rs via CoreState labels
        spawn_msg.service_name   = Some(svc.service_name.clone());

        // Compute next replica index
        let idx = {
            let live = self.live_nodes_by_service();
            live.get(svc.service_name.as_str())
                .map(|v| v.len() as u32)
                .unwrap_or(0)
        };
        spawn_msg.replica_index = Some(idx);

        if let Err(e) = crate::job::spawn_node_via_state(self.state(), spawn_msg).await {
            tracing::warn!(
                "reconciler: failed to spawn replica for '{}': {e:?}",
                svc.service_name
            );
        }
    }

    /// Kill a node by UUID (best-effort).
    async fn kill_node(&self, node_id: Uuid) {
        if let Some(handle) = self.state().node_table().remove(node_id) {
            let _ = crate::job::terminate_node(&handle, 0);
            tracing::info!("reconciler: killed node {node_id}");
        }
    }

    /// Return a map of service_name → [node_uuid, …] for currently live nodes.
    fn live_nodes_by_service(&self) -> HashMap<String, Vec<Uuid>> {
        let mut map: HashMap<String, Vec<Uuid>> = HashMap::new();
        for summary in self.state().node_table().list_live() {
            if let Some(svc_name) = summary.service_name {
                map.entry(svc_name).or_default().push(summary.node_id);
            }
        }
        map
    }

    /// Check whether all `depends_on` conditions for `svc` are satisfied.
    fn deps_satisfied(
        &self,
        svc: &ServiceSpec,
        live: &HashMap<String, Vec<Uuid>>,
    ) -> bool {
        for dep in &svc.depends_on {
            let dep_nodes = live.get(dep.service_name.as_str());
            match dep.condition {
                DependsOnCondition::ServiceStarted => {
                    if dep_nodes.map(|v| v.is_empty()).unwrap_or(true) {
                        return false;
                    }
                }
                DependsOnCondition::ServiceHealthy => {
                    let healthy = dep_nodes
                        .map(|ids| {
                            ids.iter().any(|id| {
                                self.state().node_table().get_health(*id)
                                    == HealthStatus::Healthy
                            })
                        })
                        .unwrap_or(false);
                    if !healthy {
                        return false;
                    }
                }
            }
        }
        true
    }
}

// ── Topological sort (Kahn's algorithm) ───────────────────────────────────

fn topo_sort(services: &[ServiceSpec]) -> anyhow::Result<Vec<String>> {
    use std::collections::VecDeque;

    let names: Vec<&str> = services.iter().map(|s| s.service_name.as_str()).collect();

    // In-degree map and adjacency list
    let mut in_degree: HashMap<&str, usize> = names.iter().map(|n| (*n, 0)).collect();
    let mut adj: HashMap<&str, Vec<&str>>   = names.iter().map(|n| (*n, vec![])).collect();

    for svc in services {
        for dep in &svc.depends_on {
            anyhow::ensure!(
                in_degree.contains_key(dep.service_name.as_str()),
                "unknown dependency '{}' referenced by '{}'",
                dep.service_name, svc.service_name
            );
            *in_degree.entry(svc.service_name.as_str()).or_insert(0) += 1;
            adj.entry(dep.service_name.as_str())
                .or_default()
                .push(svc.service_name.as_str());
        }
    }

    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter_map(|(name, &deg)| if deg == 0 { Some(*name) } else { None })
        .collect();

    let mut result: Vec<String> = Vec::with_capacity(services.len());

    while let Some(node) = queue.pop_front() {
        result.push(node.to_owned());
        if let Some(neighbors) = adj.get(node) {
            for &nb in neighbors {
                let deg = in_degree.get_mut(nb).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    queue.push_back(nb);
                }
            }
        }
    }

    anyhow::ensure!(
        result.len() == services.len(),
        "circular dependency detected in desired-state service graph"
    );

    Ok(result)
}

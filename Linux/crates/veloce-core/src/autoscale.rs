//! Horizontal Process Autoscaler (HPA) for VeloceNetwork (v3.1).
//!
//! Evaluates service replica CPU% and memory utilization against configured
//! targets and computes desired replica counts with cooldown hysteresis.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, info};
use veloce_ipc::message::{AutoscaleInfoMsg, AutoscalePolicyMsg};

#[derive(Debug, Clone, PartialEq)]
pub struct AutoscalePolicy {
    pub service_name: String,
    pub min_replicas: u32,
    pub max_replicas: u32,
    pub target_cpu_percent: Option<u32>,
    pub target_memory_mb: Option<u64>,
    pub scale_up_cooldown_secs: u32,
    pub scale_down_cooldown_secs: u32,
    pub last_scale_instant: Option<Instant>,
    pub last_scale_timestamp_secs: u64,
}

impl AutoscalePolicy {
    pub fn from_msg(msg: AutoscalePolicyMsg) -> Self {
        Self {
            service_name: msg.service_name,
            min_replicas: msg.min_replicas.max(1),
            max_replicas: msg.max_replicas.max(msg.min_replicas.max(1)),
            target_cpu_percent: msg.target_cpu_percent,
            target_memory_mb: msg.target_memory_mb,
            scale_up_cooldown_secs: if msg.scale_up_cooldown_secs == 0 { 30 } else { msg.scale_up_cooldown_secs },
            scale_down_cooldown_secs: if msg.scale_down_cooldown_secs == 0 { 60 } else { msg.scale_down_cooldown_secs },
            last_scale_instant: None,
            last_scale_timestamp_secs: 0,
        }
    }

    pub fn to_msg(&self) -> AutoscalePolicyMsg {
        AutoscalePolicyMsg {
            service_name: self.service_name.clone(),
            min_replicas: self.min_replicas,
            max_replicas: self.max_replicas,
            target_cpu_percent: self.target_cpu_percent,
            target_memory_mb: self.target_memory_mb,
            scale_up_cooldown_secs: self.scale_up_cooldown_secs,
            scale_down_cooldown_secs: self.scale_down_cooldown_secs,
        }
    }
}

pub struct AutoscaleEngine {
    policies: RwLock<HashMap<String, AutoscalePolicy>>,
}

impl AutoscaleEngine {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            policies: RwLock::new(HashMap::new()),
        })
    }

    pub fn set_policy(&self, policy: AutoscalePolicy) {
        info!(
            service = %policy.service_name,
            min = policy.min_replicas,
            max = policy.max_replicas,
            "hpa: policy configured"
        );
        self.policies.write().insert(policy.service_name.clone(), policy);
    }

    #[allow(dead_code)]
    pub fn get_policy(&self, service: &str) -> Option<AutoscalePolicy> {
        self.policies.read().get(service).cloned()
    }

    pub fn remove_policy(&self, service: &str) -> bool {
        let removed = self.policies.write().remove(service).is_some();
        if removed {
            info!(service = %service, "hpa: policy removed");
        }
        removed
    }

    pub fn list_policies(&self) -> Vec<AutoscalePolicy> {
        self.policies.read().values().cloned().collect()
    }

    /// Evaluates current metrics and computes whether replicas should scale.
    /// Returns `Some(target_replicas)` if scaling should occur, or `None` if no change / in cooldown.
    #[allow(dead_code)]
    pub fn evaluate(
        &self,
        service: &str,
        current_replicas: u32,
        avg_cpu_percent: f32,
        avg_memory_mb: u64,
    ) -> Option<u32> {
        let mut policies = self.policies.write();
        let policy = policies.get_mut(service)?;

        if current_replicas == 0 {
            return Some(policy.min_replicas);
        }

        let mut desired_from_cpu = None;
        let mut desired_from_mem = None;

        if let Some(target_cpu) = policy.target_cpu_percent {
            if target_cpu > 0 && avg_cpu_percent >= 0.0 {
                let ratio = avg_cpu_percent / (target_cpu as f32);
                let computed = ((current_replicas as f32) * ratio).ceil() as u32;
                desired_from_cpu = Some(computed.clamp(policy.min_replicas, policy.max_replicas));
            }
        }

        if let Some(target_mem) = policy.target_memory_mb {
            if target_mem > 0 && avg_memory_mb > 0 {
                let ratio = (avg_memory_mb as f64) / (target_mem as f64);
                let computed = ((current_replicas as f64) * ratio).ceil() as u32;
                desired_from_mem = Some(computed.clamp(policy.min_replicas, policy.max_replicas));
            }
        }

        let target_replicas = match (desired_from_cpu, desired_from_mem) {
            (Some(c), Some(m)) => c.max(m),
            (Some(c), None) => c,
            (None, Some(m)) => m,
            (None, None) => current_replicas,
        }.clamp(policy.min_replicas, policy.max_replicas);

        if target_replicas == current_replicas {
            return None;
        }

        let now = Instant::now();
        let is_scale_up = target_replicas > current_replicas;
        let cooldown_dur = if is_scale_up {
            Duration::from_secs(policy.scale_up_cooldown_secs as u64)
        } else {
            Duration::from_secs(policy.scale_down_cooldown_secs as u64)
        };

        if let Some(last_scale) = policy.last_scale_instant {
            if now.duration_since(last_scale) < cooldown_dur {
                debug!(
                    service = %service,
                    current = current_replicas,
                    target = target_replicas,
                    "hpa: scaling postponed due to cooldown"
                );
                return None;
            }
        }

        policy.last_scale_instant = Some(now);
        policy.last_scale_timestamp_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        info!(
            service = %service,
            from = current_replicas,
            to = target_replicas,
            cpu = avg_cpu_percent,
            mem_mb = avg_memory_mb,
            "hpa: scaling service replicas"
        );

        Some(target_replicas)
    }

    pub fn get_info(
        &self,
        service: &str,
        current_replicas: u32,
        current_cpu: f32,
        current_mem_mb: u64,
    ) -> Option<AutoscaleInfoMsg> {
        let policies = self.policies.read();
        let policy = policies.get(service)?;

        Some(AutoscaleInfoMsg {
            service_name: policy.service_name.clone(),
            min_replicas: policy.min_replicas,
            max_replicas: policy.max_replicas,
            target_cpu_percent: policy.target_cpu_percent,
            target_memory_mb: policy.target_memory_mb,
            current_replicas,
            current_cpu_percent: current_cpu,
            current_memory_mb: current_mem_mb,
            last_scale_time_secs: policy.last_scale_timestamp_secs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autoscale_calculation_scale_up() {
        let engine = AutoscaleEngine::new();
        let policy = AutoscalePolicy {
            service_name: "web".into(),
            min_replicas: 1,
            max_replicas: 10,
            target_cpu_percent: Some(50),
            target_memory_mb: None,
            scale_up_cooldown_secs: 0,
            scale_down_cooldown_secs: 0,
            last_scale_instant: None,
            last_scale_timestamp_secs: 0,
        };
        engine.set_policy(policy);

        // 1 replica running at 100% CPU with target 50% -> should scale to 2
        let res = engine.evaluate("web", 1, 100.0, 100);
        assert_eq!(res, Some(2));
    }

    #[test]
    fn test_autoscale_calculation_scale_down() {
        let engine = AutoscaleEngine::new();
        let policy = AutoscalePolicy {
            service_name: "api".into(),
            min_replicas: 2,
            max_replicas: 8,
            target_cpu_percent: Some(80),
            target_memory_mb: None,
            scale_up_cooldown_secs: 0,
            scale_down_cooldown_secs: 0,
            last_scale_instant: None,
            last_scale_timestamp_secs: 0,
        };
        engine.set_policy(policy);

        // 6 replicas running at 20% CPU with target 80% -> should scale down to 2 (min_replicas clamp)
        let res = engine.evaluate("api", 6, 20.0, 100);
        assert_eq!(res, Some(2));
    }

    #[test]
    fn test_autoscale_cooldown() {
        let engine = AutoscaleEngine::new();
        let policy = AutoscalePolicy {
            service_name: "worker".into(),
            min_replicas: 1,
            max_replicas: 5,
            target_cpu_percent: Some(50),
            target_memory_mb: None,
            scale_up_cooldown_secs: 10,
            scale_down_cooldown_secs: 10,
            last_scale_instant: None,
            last_scale_timestamp_secs: 0,
        };
        engine.set_policy(policy);

        // First scale succeeds
        assert_eq!(engine.evaluate("worker", 1, 100.0, 50), Some(2));

        // Immediate subsequent scale is blocked by cooldown
        assert_eq!(engine.evaluate("worker", 2, 100.0, 50), None);
    }
}

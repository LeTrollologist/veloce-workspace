/*!
Cloud Bridge & Kubernetes Telepresence Engine (v4.0).

Allows unprivileged local developers to resolve in-cluster Kubernetes DNS (*.svc.cluster.local),
tunnel TCP connections to cloud services, and shadow/intercept live staging traffic over the Noise mesh.
*/

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Result;
use parking_lot::RwLock;
use veloce_ipc::message::{BridgeConfigMsg, BridgeInterceptRuleMsg, BridgeSessionInfoMsg};

#[derive(Debug, Clone)]
pub struct BridgeSession {
    pub session_id: String,
    pub peer: String,
    pub namespace: String,
    pub target: Option<String>,
    pub dns_suffixes: Vec<String>,
    pub intercept_rules: HashMap<String, BridgeInterceptRuleMsg>,
    pub connected_at: u64,
}

pub struct BridgeEngine {
    sessions: Arc<RwLock<HashMap<String, BridgeSession>>>,
}

impl BridgeEngine {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Connect to a remote Kubernetes bridge peer and register in-cluster DNS suffixes.
    pub fn connect_bridge(&self, config: BridgeConfigMsg) -> BridgeSessionInfoMsg {
        let session_id = format!("br-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut dns_suffixes = config.dns_suffixes;
        if dns_suffixes.is_empty() {
            dns_suffixes.push("svc.cluster.local".into());
            dns_suffixes.push("cluster.local".into());
            dns_suffixes.push("k8s.vln".into());
        }

        let session = BridgeSession {
            session_id: session_id.clone(),
            peer: config.peer.clone(),
            namespace: config.namespace.clone(),
            target: config.target.clone(),
            dns_suffixes: dns_suffixes.clone(),
            intercept_rules: HashMap::new(),
            connected_at: now,
        };

        self.sessions.write().insert(session_id.clone(), session);

        BridgeSessionInfoMsg {
            session_id,
            peer: config.peer,
            namespace: config.namespace,
            target: config.target,
            dns_suffixes,
            active_intercepts: Vec::new(),
            connected_at: now,
        }
    }

    /// Register a traffic interception rule for a remote service.
    pub fn add_intercept_rule(&self, mut rule: BridgeInterceptRuleMsg) -> Result<(String, String)> {
        let mut sessions = self.sessions.write();
        let session = sessions.get_mut(&rule.session_id)
            .ok_or_else(|| anyhow::anyhow!("bridge session '{}' not found", rule.session_id))?;

        let rule_id = if rule.rule_id.is_empty() {
            format!("ic-{}", &uuid::Uuid::new_v4().to_string()[..8])
        } else {
            rule.rule_id.clone()
        };

        rule.rule_id = rule_id.clone();
        session.intercept_rules.insert(rule_id.clone(), rule);
        Ok((session.session_id.clone(), rule_id))
    }

    /// Remove a traffic interception rule.
    pub fn remove_intercept_rule(&self, session_id: &str, rule_id: &str) -> bool {
        let mut sessions = self.sessions.write();
        if let Some(session) = sessions.get_mut(session_id) {
            session.intercept_rules.remove(rule_id).is_some()
        } else {
            false
        }
    }

    /// Disconnect an active bridge session.
    pub fn disconnect_bridge(&self, session_id: &str) -> bool {
        self.sessions.write().remove(session_id).is_some()
    }

    /// List all active bridge sessions and their intercept rules.
    pub fn list_bridges(&self) -> Vec<BridgeSessionInfoMsg> {
        let sessions = self.sessions.read();
        sessions.values().map(|s| BridgeSessionInfoMsg {
            session_id: s.session_id.clone(),
            peer: s.peer.clone(),
            namespace: s.namespace.clone(),
            target: s.target.clone(),
            dns_suffixes: s.dns_suffixes.clone(),
            active_intercepts: s.intercept_rules.values().cloned().collect(),
            connected_at: s.connected_at,
        }).collect()
    }

    /// Check if a domain name matches any active bridge's Kubernetes DNS suffix.
    pub fn matches_dns_suffix(&self, hostname: &str) -> Option<String> {
        let sessions = self.sessions.read();
        for session in sessions.values() {
            for suffix in &session.dns_suffixes {
                if hostname.ends_with(suffix) || hostname.contains(".k8s.") {
                    return Some(session.peer.clone());
                }
            }
        }
        None
    }

    /// Match an incoming request against active interception rules.
    /// Returns the local port to forward the traffic to, if matched.
    pub fn match_intercept(
        &self,
        service_name: &str,
        remote_port: u16,
        headers: &HashMap<String, String>,
    ) -> Option<u16> {
        let sessions = self.sessions.read();
        for session in sessions.values() {
            for rule in session.intercept_rules.values() {
                if rule.service_name == service_name && rule.remote_port == remote_port {
                    if let Some(ref filter) = rule.header_filter {
                        if let Some((filter_k, filter_v)) = filter.split_once(':') {
                            let k = filter_k.trim().to_lowercase();
                            let v = filter_v.trim();
                            if let Some(actual_v) = headers.get(&k) {
                                if actual_v == v {
                                    return Some(rule.local_port);
                                }
                            }
                        } else {
                            // Key existence check (e.g. "X-Veloce-Intercept")
                            let k = filter.trim().to_lowercase();
                            if headers.contains_key(&k) {
                                return Some(rule.local_port);
                            }
                        }
                    } else {
                        // Unconditional port interception
                        return Some(rule.local_port);
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bridge_session_and_dns_matching() {
        let engine = BridgeEngine::new();
        let config = BridgeConfigMsg {
            peer: "k8s-agent-pod-7f".into(),
            namespace: "staging".into(),
            target: Some("payment-service".into()),
            dns_suffixes: vec!["svc.cluster.local".into(), "staging.svc".into()],
        };

        let session_info = engine.connect_bridge(config);
        assert_eq!(session_info.namespace, "staging");
        assert_eq!(engine.list_bridges().len(), 1);

        // DNS suffix matching
        assert_eq!(
            engine.matches_dns_suffix("postgres.staging.svc.cluster.local"),
            Some("k8s-agent-pod-7f".into())
        );
        assert_eq!(
            engine.matches_dns_suffix("redis.staging.svc"),
            Some("k8s-agent-pod-7f".into())
        );
        assert_eq!(engine.matches_dns_suffix("google.com"), None);

        // Disconnect
        assert!(engine.disconnect_bridge(&session_info.session_id));
        assert_eq!(engine.list_bridges().len(), 0);
        assert_eq!(engine.matches_dns_suffix("postgres.staging.svc.cluster.local"), None);
    }

    #[test]
    fn test_bridge_traffic_interception() {
        let engine = BridgeEngine::new();
        let config = BridgeConfigMsg {
            peer: "k8s-node".into(),
            namespace: "staging".into(),
            target: None,
            dns_suffixes: vec![],
        };
        let session_info = engine.connect_bridge(config);

        let rule = BridgeInterceptRuleMsg {
            session_id: session_info.session_id.clone(),
            rule_id: "".into(),
            service_name: "payment-api".into(),
            remote_port: 8080,
            local_port: 3000,
            header_filter: Some("X-Veloce-Intercept: alice".into()),
        };
        let (_, rule_id) = engine.add_intercept_rule(rule).expect("add rule");

        let mut matching_headers = HashMap::new();
        matching_headers.insert("x-veloce-intercept".into(), "alice".into());

        let mut non_matching_headers = HashMap::new();
        non_matching_headers.insert("x-veloce-intercept".into(), "bob".into());

        // Header match routes to local 3000
        assert_eq!(
            engine.match_intercept("payment-api", 8080, &matching_headers),
            Some(3000)
        );

        // Mismatched value bypasses
        assert_eq!(
            engine.match_intercept("payment-api", 8080, &non_matching_headers),
            None
        );

        // Remove rule
        assert!(engine.remove_intercept_rule(&session_info.session_id, &rule_id));
        assert_eq!(
            engine.match_intercept("payment-api", 8080, &matching_headers),
            None
        );
    }
}

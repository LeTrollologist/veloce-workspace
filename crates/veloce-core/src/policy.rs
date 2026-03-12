//! VeloceCore policy engine.
//!
//! Loads optional `veloce-policy.toml` from the data directory and evaluates:
//! 1. **Capability RBAC** — per-app allow/deny rules for IPC capabilities.
//! 2. **Mesh ACL** — per-hostname (and optionally per-peer) gossip filter rules.
//!
//! If the file is absent all operations are allowed (permissive default).
//!
//! TOML schema:
//! ```toml
//! # default_effect = "allow"   # or "deny" — applies when no rule matches
//!
//! # [[rules]]
//! # app   = "untrusted-agent"
//! # deny  = ["SpawnNodes", "KillNodes"]
//!
//! # [[mesh_acl]]
//! # hostname  = "secret.vln"
//! # effect    = "deny"
//!
//! # [[mesh_acl]]
//! # hostname  = "*.internal"
//! # from_peer = "DESKTOP-UNTRUSTED"
//! # effect    = "deny"
//! ```

use std::{path::PathBuf, sync::Arc};

use parking_lot::RwLock;
use serde::Deserialize;

use veloce_ipc::message::{MeshAclMsg, PolicyRuleMsg, PolicyRulesMsg};

// ── Config types (TOML-deserialized) ─────────────────────────────────────────

fn default_allow() -> String { "allow".to_string() }

#[derive(Deserialize, Default, Clone)]
pub struct PolicyConfig {
    /// Effect to apply when no rule matches: `"allow"` (default) or `"deny"`.
    #[serde(default = "default_allow")]
    pub default_effect: String,
    #[serde(default)]
    pub rules:    Vec<PolicyRule>,
    #[serde(default)]
    pub mesh_acl: Vec<MeshAcl>,
}

/// Per-app capability RBAC rule.
#[derive(Deserialize, Clone)]
pub struct PolicyRule {
    /// App name to match.  `"*"` matches any app.
    pub app:   String,
    /// Capabilities explicitly allowed (whitelist).
    pub allow: Option<Vec<String>>,
    /// Capabilities explicitly denied (blacklist).
    pub deny:  Option<Vec<String>>,
}

/// Mesh gossip ACL entry.
#[derive(Deserialize, Clone)]
pub struct MeshAcl {
    /// Hostname pattern: exact or `"*.suffix"`.
    pub hostname:  String,
    /// If set, only applies when the gossip came from this peer name.
    pub from_peer: Option<String>,
    /// `"allow"` or `"deny"`.
    pub effect:    String,
}

// ── PolicyEngine ──────────────────────────────────────────────────────────────

/// Thread-safe, hot-reloadable policy engine.
pub struct PolicyEngine {
    path:   PathBuf,
    config: RwLock<PolicyConfig>,
}

impl PolicyEngine {
    /// Load policy from `path`, or use permissive defaults if the file is absent
    /// or cannot be parsed.
    pub fn load_or_default(path: PathBuf) -> Arc<Self> {
        let config = load_config(&path).unwrap_or_else(|e| {
            if path.exists() {
                tracing::warn!("failed to parse policy file {}: {e} — using defaults", path.display());
            }
            PolicyConfig::default()
        });
        Arc::new(Self { path, config: RwLock::new(config) })
    }

    /// Re-read the policy file from disk.  If the file has been deleted, reverts
    /// to the permissive default.
    pub fn reload(&self) -> anyhow::Result<()> {
        let config = load_config(&self.path).unwrap_or_else(|_| PolicyConfig::default());
        *self.config.write() = config;
        tracing::info!("policy reloaded from {}", self.path.display());
        Ok(())
    }

    /// Check whether `app` is allowed to use capability `cap`.
    ///
    /// Iterates rules in order; first match wins.  If no rule matches, the
    /// `default_effect` decides.
    pub fn check_capability(&self, app: &str, cap: &str) -> bool {
        let cfg = self.config.read();
        for rule in &cfg.rules {
            if !glob_match(&rule.app, app) { continue; }
            if let Some(allow) = &rule.allow {
                return allow.iter().any(|c| c.eq_ignore_ascii_case(cap));
            }
            if let Some(deny) = &rule.deny {
                return !deny.iter().any(|c| c.eq_ignore_ascii_case(cap));
            }
            // Rule matched but has neither allow nor deny — treat as allow
        }
        cfg.default_effect.eq_ignore_ascii_case("allow")
    }

    /// Check whether a gossip entry (`hostname`, from `peer`) is allowed through
    /// the mesh ACL.
    pub fn check_mesh_acl(&self, hostname: &str, peer: &str) -> bool {
        let cfg = self.config.read();
        for acl in &cfg.mesh_acl {
            if !glob_match(&acl.hostname, hostname) { continue; }
            if let Some(fp) = &acl.from_peer {
                if !glob_match(fp, peer) { continue; }
            }
            return acl.effect.eq_ignore_ascii_case("allow");
        }
        cfg.default_effect.eq_ignore_ascii_case("allow")
    }

    /// Snapshot the current policy as an IPC-serialisable message.
    pub fn to_msg(&self) -> PolicyRulesMsg {
        let cfg = self.config.read();
        PolicyRulesMsg {
            default_effect: cfg.default_effect.clone(),
            rules: cfg.rules.iter().map(|r| PolicyRuleMsg {
                app:   r.app.clone(),
                allow: r.allow.clone(),
                deny:  r.deny.clone(),
            }).collect(),
            mesh_acls: cfg.mesh_acl.iter().map(|a| MeshAclMsg {
                hostname:  a.hostname.clone(),
                from_peer: a.from_peer.clone(),
                effect:    a.effect.clone(),
            }).collect(),
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn load_config(path: &PathBuf) -> anyhow::Result<PolicyConfig> {
    let content = std::fs::read_to_string(path)?;
    let cfg: PolicyConfig = toml::from_str(&content)?;
    Ok(cfg)
}

/// Simple glob matching supporting `"*"` (match any) and `"*.suffix"`.
fn glob_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" { return true; }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        // "*.example.com" matches "foo.example.com" and "example.com"
        return value.ends_with(&format!(".{suffix}")) || value == suffix;
    }
    pattern == value
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_from_toml(s: &str) -> Arc<PolicyEngine> {
        let cfg: PolicyConfig = toml::from_str(s).unwrap();
        Arc::new(PolicyEngine {
            path:   PathBuf::from("/dev/null"),
            config: RwLock::new(cfg),
        })
    }

    #[test]
    fn default_permissive() {
        let e = PolicyEngine::load_or_default(PathBuf::from("nonexistent-policy-xyz.toml"));
        assert!(e.check_capability("any-app", "SpawnNodes"));
        assert!(e.check_mesh_acl("foo.vln", "peer1"));
    }

    #[test]
    fn deny_rule_blocks_cap() {
        let e = engine_from_toml(r#"
            [[rules]]
            app  = "untrusted"
            deny = ["SpawnNodes", "KillNodes"]
        "#);
        assert!(!e.check_capability("untrusted", "SpawnNodes"));
        assert!(!e.check_capability("untrusted", "KillNodes"));
        assert!( e.check_capability("untrusted", "RegistryRead"));
        assert!( e.check_capability("trusted",   "SpawnNodes"));
    }

    #[test]
    fn allow_rule_whitelist() {
        let e = engine_from_toml(r#"
            [[rules]]
            app   = "readonly-app"
            allow = ["RegistryRead", "NetResolve"]
        "#);
        assert!( e.check_capability("readonly-app", "RegistryRead"));
        assert!(!e.check_capability("readonly-app", "SpawnNodes"));
    }

    #[test]
    fn wildcard_app_rule() {
        let e = engine_from_toml(r#"
            default_effect = "deny"
            [[rules]]
            app   = "*"
            allow = ["RegistryRead"]
        "#);
        assert!( e.check_capability("any-app", "RegistryRead"));
        assert!(!e.check_capability("any-app", "SpawnNodes"));
    }

    #[test]
    fn mesh_acl_deny_hostname() {
        let e = engine_from_toml(r#"
            [[mesh_acl]]
            hostname = "secret.vln"
            effect   = "deny"
        "#);
        assert!(!e.check_mesh_acl("secret.vln", "peer1"));
        assert!( e.check_mesh_acl("public.vln", "peer1"));
    }

    #[test]
    fn mesh_acl_glob_suffix() {
        let e = engine_from_toml(r#"
            [[mesh_acl]]
            hostname = "*.internal"
            effect   = "deny"
        "#);
        assert!(!e.check_mesh_acl("db.internal",   "p"));
        assert!(!e.check_mesh_acl("api.internal",  "p"));
        assert!( e.check_mesh_acl("public.vln",    "p"));
    }

    #[test]
    fn mesh_acl_from_peer_filter() {
        let e = engine_from_toml(r#"
            [[mesh_acl]]
            hostname  = "*.internal"
            from_peer = "UNTRUSTED"
            effect    = "deny"
        "#);
        // Denied for the named peer
        assert!(!e.check_mesh_acl("db.internal", "UNTRUSTED"));
        // Allowed for a different peer
        assert!( e.check_mesh_acl("db.internal", "TRUSTED"));
    }
}

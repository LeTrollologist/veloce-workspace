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
fn default_stun_servers() -> Vec<String> {
    vec![
        "stun.l.google.com:19302".to_string(),
        "stun.cloudflare.com:3478".to_string(),
    ]
}
fn default_gossip_interval() -> u64 { 60 }

/// Controls how the mesh layer handles WAN reachability.
#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MeshMode {
    /// Try STUN for WAN IP discovery; fall back to LAN-only with a warning
    /// if all STUN servers fail (default).
    #[default]
    Auto,
    /// Skip STUN entirely — only advertise LAN addresses in join codes.
    LanOnly,
    /// Require a reachable WAN IP via STUN; log a warning if unavailable but
    /// still operate in LAN-only mode rather than refusing to start.
    Wan,
}

#[derive(Deserialize, Clone)]
pub struct PolicyConfig {
    /// Effect to apply when no rule matches: `"allow"` (default) or `"deny"`.
    #[serde(default = "default_allow")]
    pub default_effect: String,
    #[serde(default)]
    pub rules:    Vec<PolicyRule>,
    #[serde(default)]
    pub mesh_acl: Vec<MeshAcl>,

    /// STUN servers to use for WAN IP discovery.
    ///
    /// ```toml
    /// stun_servers = ["stun.l.google.com:19302", "stun.example.corp:3478"]
    /// ```
    ///
    /// Falls back to the built-in list if not set.
    #[serde(default = "default_stun_servers")]
    pub stun_servers: Vec<String>,

    /// Controls WAN/LAN mesh mode.
    ///
    /// ```toml
    /// mesh_mode = "auto"     # default: try STUN, fall back to LAN
    /// mesh_mode = "lan-only" # skip STUN entirely
    /// mesh_mode = "wan"      # require WAN IP (warn if unavailable)
    /// ```
    #[serde(default)]
    pub mesh_mode: MeshMode,

    /// How often (in seconds) to re-broadcast the full local hostname list to
    /// each connected peer.  Default: 60 s.  Set to 0 to disable periodic
    /// re-sync (not recommended — stale peers will miss late registrations).
    ///
    /// ```toml
    /// gossip_interval_secs = 60
    /// ```
    #[serde(default = "default_gossip_interval")]
    pub gossip_interval_secs: u64,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            default_effect:       default_allow(),
            rules:                vec![],
            mesh_acl:             vec![],
            stun_servers:         default_stun_servers(),
            mesh_mode:            MeshMode::default(),
            gossip_interval_secs: default_gossip_interval(),
        }
    }
}

/// Per-app capability RBAC rule.
#[derive(Deserialize, Clone)]
pub struct PolicyRule {
    /// Verified executable path pattern — matched against the kernel-resolved
    /// Win32 image path of the connecting process.  Takes precedence over `app`
    /// when present.  Supports `"*"` (any exe), exact basename (`"veloce-run.exe"`),
    /// or full path globs (`"*.exe"`).
    #[serde(default)]
    pub exe:   Option<String>,
    /// Legacy app-name pattern matched against the client-declared name (unverified).
    /// Ignored when `exe` is present.  Kept for backward compatibility.
    #[serde(default)]
    pub app:   Option<String>,
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

    /// Read-lock the current `PolicyConfig`.
    ///
    /// The returned guard holds the `RwLock` for its lifetime; drop it promptly
    /// to avoid blocking writers.
    pub fn config(&self) -> parking_lot::RwLockReadGuard<'_, PolicyConfig> {
        self.config.read()
    }

    /// Re-read the policy file from disk.
    ///
    /// On parse failure the **previous** configuration is kept intact — the
    /// system never regresses to the permissive default mid-flight because of
    /// a typo in a live file.  The error is logged at ERROR level so operators
    /// see the failure immediately.
    pub fn reload(&self) -> anyhow::Result<()> {
        match load_config(&self.path) {
            Ok(config) => {
                *self.config.write() = config;
                tracing::info!("policy reloaded from {}", self.path.display());
            }
            Err(e) => {
                tracing::error!(
                    path = %self.path.display(),
                    error = %e,
                    "policy reload failed — keeping previous configuration"
                );
            }
        }
        Ok(())
    }

    /// Check whether `exe_path` is allowed to use the named capability.
    ///
    /// Convenience wrapper around [`compute_max_caps`] for single-capability
    /// checks — primarily used in tests.
    pub fn check_capability(&self, exe_path: &str, cap_name: &str) -> bool {
        self.compute_max_caps(exe_path)
            .iter()
            .any(|c| format!("{c:?}").eq_ignore_ascii_case(cap_name))
    }

    /// Compute the full set of capabilities this executable is permitted to hold.
    ///
    /// Called once at handshake time; the result is intersected with the
    /// client's declared request so clients can still ask for a subset.
    /// `exe_path` must be the kernel-verified Win32 image path.
    pub fn compute_max_caps(&self, exe_path: &str) -> Vec<veloce_ipc::message::Capability> {
        use veloce_ipc::message::Capability::*;
        const ALL: &[veloce_ipc::message::Capability] =
            &[SpawnNodes, KillNodes, RegistryRead, RegistryWrite, NetRegister, NetResolve,
              MeshManage, PolicyAdmin, SecretsRead, SecretsWrite, NetPortForward, DesiredStateManage,
              HubManage, MeshKvManage];

        let cfg = self.config.read();
        for rule in &cfg.rules {
            let (pattern, verified) = match (&rule.exe, &rule.app) {
                (Some(e), _) => (e.as_str(), true),
                (None, Some(a)) => (a.as_str(), false),
                (None, None) => continue,
            };
            let matched = if verified {
                exe_glob_match(pattern, exe_path)
            } else {
                glob_match(pattern, exe_path)
            };
            if !matched { continue; }

            if let Some(allow) = &rule.allow {
                return ALL.iter()
                    .filter(|c| allow.iter().any(|a| a.eq_ignore_ascii_case(&format!("{c:?}"))))
                    .cloned()
                    .collect();
            }
            if let Some(deny) = &rule.deny {
                return ALL.iter()
                    .filter(|c| !deny.iter().any(|d| d.eq_ignore_ascii_case(&format!("{c:?}"))))
                    .cloned()
                    .collect();
            }
            // Rule matched with neither allow nor deny → full grant.
            return ALL.to_vec();
        }
        // No rule matched — apply default effect.
        if cfg.default_effect.eq_ignore_ascii_case("allow") {
            ALL.to_vec()
        } else {
            vec![]
        }
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
                exe:   r.exe.clone(),
                app:   r.app.clone().unwrap_or_default(),
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

/// Match a policy `exe` pattern against a full Win32 image path.
///
/// Tries the full path first, then falls back to the basename component so
/// that `"veloce-run.exe"` matches `"C:\...\veloce-run.exe"`.
fn exe_glob_match(pattern: &str, exe_path: &str) -> bool {
    if glob_match(pattern, exe_path) { return true; }
    let name = exe_path.rsplit(['\\', '/']).next().unwrap_or(exe_path);
    glob_match(pattern, name)
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
    fn exe_rule_blocks_spawn_by_basename() {
        let e = engine_from_toml(r#"
            default_effect = "deny"
            [[rules]]
            exe   = "untrusted-agent.exe"
            allow = ["RegistryRead"]
        "#);
        // Full path — basename matches
        assert!( e.check_capability(r"C:\Apps\untrusted-agent.exe", "RegistryRead"));
        assert!(!e.check_capability(r"C:\Apps\untrusted-agent.exe", "SpawnNodes"));
        // Different exe — no rule matches, default = deny
        assert!(!e.check_capability(r"C:\Apps\trusted-tool.exe", "SpawnNodes"));
    }

    #[test]
    fn compute_max_caps_deny_rule() {
        use veloce_ipc::message::Capability::*;
        let e = engine_from_toml(r#"
            [[rules]]
            exe  = "restricted.exe"
            deny = ["SpawnNodes", "KillNodes"]
        "#);
        let caps = e.compute_max_caps(r"C:\bin\restricted.exe");
        assert!(!caps.contains(&SpawnNodes));
        assert!(!caps.contains(&KillNodes));
        assert!( caps.contains(&RegistryRead));
        assert!( caps.contains(&RegistryWrite));
    }

    #[test]
    fn compute_max_caps_default_deny() {
        let e = engine_from_toml(r#"
            default_effect = "deny"
        "#);
        let caps = e.compute_max_caps(r"C:\bin\any.exe");
        assert!(caps.is_empty(), "default-deny with no rules should grant nothing");
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

/*!
Veloce Compose YAML schema and processing (v1.1).

Parses `veloce-compose.yml` and converts each service into a `SpawnNodeMsg`
suitable for passing to `VeloceClient::spawn_node_with`.
*/

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use veloce_ipc::message::{
    DependsOnCondition, DependsOnSpec, DesiredStateSpec, HealthCheck, HealthProbe,
    NodeLimits, RestartPolicy, SecretRef, ServiceSpec, SpawnNodeMsg, UpdateStrategy,
    VolumeMountSource, VolumeMountSpec,
};

// ── YAML schema types ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ComposeFile {
    #[serde(default)]
    #[allow(dead_code)]
    pub version: String,
    pub services: HashMap<String, ComposeService>,
    #[serde(default)]
    pub volumes: HashMap<String, serde_yaml::Value>,
    #[serde(default)]
    pub secrets: HashMap<String, ComposeSecretSource>,
}

#[derive(Debug, Deserialize)]
pub struct ComposeService {
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub environment: Vec<String>,
    #[serde(default)]
    pub ports: Vec<String>,
    #[serde(default)]
    pub volumes: Vec<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
    #[serde(default)]
    pub depends_on: DependsOnField,
    #[serde(default)]
    pub healthcheck: Option<ComposeHealthCheck>,
    #[serde(default)]
    pub restart: Option<String>,
    #[serde(default)]
    pub deploy: Option<ComposeDeploy>,
    #[serde(default)]
    pub cpu: Option<u8>,
    #[serde(default)]
    pub mem: Option<u64>,
    #[serde(default)]
    pub autoscaling: Option<ComposeAutoscaling>,
    #[serde(default)]
    pub cron: Option<ComposeCron>,
    #[serde(default)]
    pub ingress: Option<ComposeIngress>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComposeIngress {
    pub host: String,
    #[serde(default = "default_ingress_path")]
    pub path: String,
    #[serde(default)]
    pub strip_prefix: bool,
    #[serde(default)]
    pub tls: bool,
    pub cert: Option<String>,
    pub key: Option<String>,
}

fn default_ingress_path() -> String { "/".into() }

#[derive(Debug, Clone, Deserialize)]
pub struct ComposeAutoscaling {
    #[serde(default = "default_min_replicas")]
    pub min_replicas: u32,
    #[serde(default = "default_max_replicas")]
    pub max_replicas: u32,
    pub target_cpu: Option<u32>,
    pub target_memory_mb: Option<u64>,
}

fn default_min_replicas() -> u32 { 1 }
fn default_max_replicas() -> u32 { 5 }

#[derive(Debug, Clone, Deserialize)]
pub struct ComposeCron {
    pub schedule: String,
    #[serde(default = "default_concurrency")]
    pub concurrency: String,
}

fn default_concurrency() -> String { "Allow".into() }

/// `depends_on` accepts either a list of strings or a map.
#[derive(Debug, Deserialize, Default)]
#[serde(untagged)]
pub enum DependsOnField {
    List(Vec<String>),
    Map(HashMap<String, DependsOnConditionYaml>),
    #[default]
    None,
}

#[derive(Debug, Deserialize)]
pub struct DependsOnConditionYaml {
    pub condition: String,
}

#[derive(Debug, Deserialize)]
pub struct ComposeHealthCheck {
    pub test: ComposeHealthTest,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default = "default_retries")]
    pub retries: u32,
}

fn default_interval() -> u64 { 10 }
fn default_timeout()  -> u64 { 5 }
fn default_retries()  -> u32 { 3 }

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ComposeHealthTest {
    Http { http: HttpProbeYaml },
    Tcp  { tcp:  TcpProbeYaml  },
    Exec { exec: ExecProbeYaml },
}

#[derive(Debug, Deserialize)]
pub struct HttpProbeYaml { pub port: u16, #[serde(default = "default_path")] pub path: String }
#[derive(Debug, Deserialize)]
pub struct TcpProbeYaml  { pub port: u16 }
#[derive(Debug, Deserialize)]
pub struct ExecProbeYaml { pub command: String, #[serde(default)] pub args: Vec<String> }

fn default_path() -> String { "/".into() }

#[derive(Debug, Deserialize)]
pub struct ComposeDeploy {
    #[serde(default = "default_replicas")]
    pub replicas: u32,
    #[serde(default)]
    pub update_config: Option<ComposeUpdateConfig>,
}
fn default_replicas() -> u32 { 1 }

#[derive(Debug, Deserialize)]
pub struct ComposeUpdateConfig {
    #[serde(default = "default_order")]
    pub order: String,
    #[serde(default = "default_max_unavailable")]
    pub max_unavailable: u32,
}
fn default_order() -> String { "stop-first".into() }
fn default_max_unavailable() -> u32 { 1 }

#[derive(Debug, Deserialize, Default)]
pub struct ComposeSecretSource {
    /// Host file path — its contents will be pre-loaded into the vault.
    #[serde(default)]
    pub file: Option<String>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Load and parse a `veloce-compose.yml` file.
pub fn load(path: &Path) -> Result<ComposeFile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read compose file {:?}", path))?;
    serde_yaml::from_str(&text)
        .with_context(|| format!("parse compose file {:?}", path))
}

/// Convert the compose file into a `DesiredStateSpec` for `apply_desired_state`.
pub fn to_desired_state(file: &ComposeFile, name: &str) -> Result<DesiredStateSpec> {
    let mut services = Vec::new();
    for (svc_name, svc) in &file.services {
        let spawn_msg = to_spawn_msg(svc_name, svc)?;
        let replicas  = svc.deploy.as_ref().map(|d| d.replicas).unwrap_or(1);
        let update_strategy = svc.deploy.as_ref()
            .and_then(|d| d.update_config.as_ref())
            .map(|uc| {
                if uc.order == "start-first" {
                    UpdateStrategy::RollingUpdate { max_unavailable: uc.max_unavailable }
                } else {
                    UpdateStrategy::Recreate
                }
            })
            .unwrap_or(UpdateStrategy::Recreate);

        services.push(ServiceSpec {
            service_name:    svc_name.clone(),
            spawn_msg,
            replicas,
            update_strategy,
            depends_on: parse_depends_on(&svc.depends_on)?,
        });
    }
    Ok(DesiredStateSpec { name: name.into(), services })
}

/// Build a `SpawnNodeMsg` from a single compose service definition.
pub fn to_spawn_msg(service_name: &str, svc: &ComposeService) -> Result<SpawnNodeMsg> {
    // Parse KEY=VALUE environment entries.
    let env: Vec<(String, String)> = svc.environment.iter().filter_map(|e| {
        let mut parts = e.splitn(2, '=');
        Some((parts.next()?.to_owned(), parts.next().unwrap_or("").to_owned()))
    }).collect();

    let limits = if svc.cpu.is_some() || svc.mem.is_some() {
        Some(NodeLimits {
            cpu_pct:           svc.cpu.map(|c| c as u32),
            mem_mb:            svc.mem,
            max_lifetime_secs: None,
        })
    } else {
        None
    };

    let restart_policy = svc.restart.as_deref().map(parse_restart_policy);

    // Parse volume mount specs: "name:SUFFIX" or "/host/path:SUFFIX"
    let volume_mounts: Vec<VolumeMountSpec> = svc.volumes.iter().map(|v| {
        let parts: Vec<&str> = v.splitn(2, ':').collect();
        let source_str = parts[0];
        let env_suffix = parts.get(1).copied().unwrap_or(source_str).to_owned();
        let source = if source_str.starts_with('/') || source_str.contains('\\')
                     || source_str.contains(':') {
            VolumeMountSource::BindPath(source_str.to_owned())
        } else {
            VolumeMountSource::Named(source_str.to_owned())
        };
        VolumeMountSpec { source, env_suffix }
    }).collect();

    // Parse secret refs: "secret_name" → env var NAME=<decrypted>
    let secret_refs: Vec<SecretRef> = svc.secrets.iter().map(|s| SecretRef {
        secret_name: s.clone(),
        env_var:     s.to_uppercase(),
    }).collect();

    let health_check = svc.healthcheck.as_ref().map(|hc| HealthCheck {
        probe:             match &hc.test {
            ComposeHealthTest::Http { http } => HealthProbe::Http { port: http.port, path: http.path.clone() },
            ComposeHealthTest::Tcp  { tcp  } => HealthProbe::Tcp  { port: tcp.port },
            ComposeHealthTest::Exec { exec } => HealthProbe::Exec { command: exec.command.clone(), args: exec.args.clone() },
        },
        interval_secs:     hc.interval,
        timeout_secs:      hc.timeout,
        success_threshold: 1,
        failure_threshold: hc.retries,
    });

    Ok(SpawnNodeMsg {
        app_name:         service_name.into(),
        executable:       svc.executable.clone(),
        args:             svc.args.clone(),
        env,
        limits,
        auto_kill:        false, // managed by reconciler
        restart_policy,
        use_appcontainer: false,
        health_check,
        volume_mounts,
        secret_refs,
        service_name:     Some(service_name.into()),
        replica_index:    None, // set by reconciler
        isolation_level:  None,
    })
}

/// Parse `depends_on` YAML (list or map) into `Vec<DependsOnSpec>`.
pub fn parse_depends_on(field: &DependsOnField) -> Result<Vec<DependsOnSpec>> {
    Ok(match field {
        DependsOnField::None => vec![],
        DependsOnField::List(names) => names.iter().map(|n| DependsOnSpec {
            service_name: n.clone(),
            condition:    DependsOnCondition::ServiceStarted,
        }).collect(),
        DependsOnField::Map(map) => map.iter().map(|(name, cv)| DependsOnSpec {
            service_name: name.clone(),
            condition:    if cv.condition == "service_healthy" {
                DependsOnCondition::ServiceHealthy
            } else {
                DependsOnCondition::ServiceStarted
            },
        }).collect(),
    })
}

/// Parse `"HOST:NODE"` port mapping string.
pub fn parse_port_mapping(s: &str) -> Result<(u16, u16)> {
    let parts: Vec<&str> = s.splitn(2, ':').collect();
    if parts.len() != 2 {
        bail!("invalid port mapping '{}' — expected HOST:NODE", s);
    }
    let host = parts[0].parse::<u16>()
        .with_context(|| format!("invalid host port in '{s}'"))?;
    let node = parts[1].parse::<u16>()
        .with_context(|| format!("invalid node port in '{s}'"))?;
    Ok((host, node))
}

/// Perform a topological sort of services by their `depends_on` relationships.
/// Returns service names in dependency-first order.
pub fn topo_sort(services: &HashMap<String, ComposeService>) -> Result<Vec<String>> {
    use std::collections::VecDeque;

    let names: Vec<&str> = services.keys().map(|s| s.as_str()).collect();
    let mut in_degree: HashMap<&str, usize> = names.iter().map(|n| (*n, 0)).collect();
    let mut adj: HashMap<&str, Vec<&str>>   = names.iter().map(|n| (*n, vec![])).collect();

    for (name, svc) in services {
        let deps = match &svc.depends_on {
            DependsOnField::List(v) => v.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            DependsOnField::Map(m)  => m.keys().map(|s| s.as_str()).collect::<Vec<_>>(),
            DependsOnField::None    => vec![],
        };
        for dep in deps {
            anyhow::ensure!(
                in_degree.contains_key(dep),
                "unknown dependency '{}' referenced by '{}'", dep, name
            );
            *in_degree.entry(name.as_str()).or_insert(0) += 1;
            adj.entry(dep).or_default().push(name.as_str());
        }
    }

    let mut queue: VecDeque<&str> = in_degree.iter()
        .filter_map(|(n, &d)| if d == 0 { Some(*n) } else { None })
        .collect();
    let mut result: Vec<String> = Vec::with_capacity(services.len());

    while let Some(node) = queue.pop_front() {
        result.push(node.to_owned());
        if let Some(neighbors) = adj.get(node) {
            for &nb in neighbors {
                let deg = in_degree.get_mut(nb).unwrap();
                *deg -= 1;
                if *deg == 0 { queue.push_back(nb); }
            }
        }
    }

    anyhow::ensure!(
        result.len() == services.len(),
        "circular dependency detected in compose file"
    );
    Ok(result)
}

// ── Restart policy parsing ────────────────────────────────────────────────────

fn parse_restart_policy(s: &str) -> RestartPolicy {
    if s == "always" || s == "unless-stopped" {
        RestartPolicy { max_restarts: u32::MAX, base_delay_secs: 1, max_delay_secs: 60 }
    } else if let Some(rest) = s.strip_prefix("on-failure:") {
        let n = rest.parse().unwrap_or(3);
        RestartPolicy { max_restarts: n, base_delay_secs: 1, max_delay_secs: 30 }
    } else if s == "on-failure" {
        RestartPolicy { max_restarts: 3, base_delay_secs: 1, max_delay_secs: 30 }
    } else {
        RestartPolicy { max_restarts: 0, base_delay_secs: 1, max_delay_secs: 30 }
    }
}

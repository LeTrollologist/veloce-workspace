/*!
veloce-run — launch any executable into the VeloceNetwork mesh.

Usage:
    veloce-run [OPTIONS] <EXECUTABLE> [ARGS]...
    veloce-run mesh identity [--ttl MINS] [--one-time]
    veloce-run mesh join <CODE>
    veloce-run mesh peers
    veloce-run mesh status
    veloce-run mesh diagnose
    veloce-run mesh ping <PEER_ID>
    veloce-run mesh leave <PEER_ID>
    veloce-run nrpt enable
    veloce-run nrpt disable
    veloce-run nrpt status
    veloce-run policy show
    veloce-run policy reload
    veloce-run version

Examples:
    # Wrap ping and stream its output to the terminal
    veloce-run --watch -- ping -t 127.0.0.1

    # Start a Node.js server, register a .vln hostname, detach
    veloce-run --name api --hostname api.vln --port 3000 --detach -- node server.js

    # Print this machine's join code (15-minute TTL by default)
    veloce-run mesh identity

    # One-time join code (expires after first use)
    veloce-run mesh identity --one-time

    # Connect to a peer (run on Machine B, paste code from Machine A)
    veloce-run mesh join "VM3:BBB..."

    # Show mesh connectivity summary with traffic stats
    veloce-run mesh status

    # Run a self-diagnosis (identity, STUN, peers, gossip counts)
    veloce-run mesh diagnose

    # Show last-known RTT to a specific peer
    veloce-run mesh ping <peer-uuid>

    # Enable system-wide .vln DNS routing via NRPT (requires admin)
    veloce-run nrpt enable

    # Show active policy rules
    veloce-run policy show

    # Hot-reload policy from veloce-policy.toml
    veloce-run policy reload

    # Print version
    veloce-run version
*/

mod compose;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use veloce_ipc::message::{
    AutoscalePolicyMsg, Capability, CronJobMsg, IngressPathRule, IngressRule, LogStream,
    NetPortForwardMsg, NodeLimits, RestartPolicy, SpawnNodeMsg,
};
use veloce_sdk::VeloceClient;

// ── Top-level CLI ─────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name    = "veloce-run",
    about   = "Launch any executable into the VeloceNetwork mesh, or manage mesh peers",
    version,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    // ── Run mode (default when no subcommand is given) ────────────────────────

    /// Executable to launch (path or name on PATH)
    #[arg(required_unless_present = "command")]
    executable: Option<String>,

    /// Arguments forwarded to the executable
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,

    /// Extra environment variables (KEY=VALUE); may be repeated
    #[arg(short = 'e', long = "env", value_name = "KEY=VALUE")]
    extra_env: Vec<String>,

    /// App name shown in the Dashboard [default: exe basename]
    #[arg(short = 'n', long)]
    name: Option<String>,

    /// Register a .vln hostname (e.g. myapp.vln)
    #[arg(short = 'H', long, value_name = "HOST")]
    hostname: Option<String>,

    /// Local port used for the .vln hostname registration
    #[arg(short = 'p', long, requires = "hostname")]
    port: Option<u16>,

    /// CPU limit in percent (1–100); omit for no limit
    #[arg(long, value_name = "PCT")]
    cpu: Option<u8>,

    /// Memory limit in megabytes; omit for no limit
    #[arg(long, value_name = "MB")]
    mem: Option<u64>,

    /// Maximum number of auto-restarts on crash (0 = disabled)
    #[arg(short = 'r', long, default_value = "0", value_name = "N")]
    restarts: u32,

    /// Stream stdout/stderr to the terminal (default unless --detach is set)
    #[arg(short = 'w', long)]
    watch: bool,

    /// Print the node ID and exit immediately after spawning
    #[arg(short = 'd', long, conflicts_with = "watch")]
    detach: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Mesh peer management commands
    Mesh {
        #[command(subcommand)]
        action: MeshAction,
    },
    /// Policy engine management commands
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },
    /// Manage system-wide .vln DNS routing via NRPT (requires Administrator)
    Nrpt {
        #[command(subcommand)]
        action: NrptAction,
    },
    /// Start services from a veloce-compose.yml file
    Up {
        /// Path to the compose file [default: veloce-compose.yml]
        #[arg(short, long, value_name = "FILE")]
        file: Option<PathBuf>,
        /// Detach after spawning (don't stream logs)
        #[arg(short, long)]
        detach: bool,
    },
    /// Stop services started by a compose file
    Down {
        /// Path to the compose file [default: veloce-compose.yml]
        #[arg(short, long, value_name = "FILE")]
        file: Option<PathBuf>,
    },
    /// Show status of all running nodes (including compose services)
    Ps,
    /// Manage the secrets vault
    Secret {
        #[command(subcommand)]
        action: SecretAction,
    },
    /// Manage Layer-7 HTTP Ingress routing rules (v2.1)
    Ingress {
        #[command(subcommand)]
        action: IngressAction,
    },
    /// Manage Horizontal Process Autoscaler (HPA) policies (v3.1)
    Autoscale {
        #[command(subcommand)]
        action: AutoscaleAction,
    },
    /// Manage scheduled tasks and CronJobs (v3.1)
    Cron {
        #[command(subcommand)]
        action: CronAction,
    },
    /// Open the embedded Web Status Portal in the default web browser (v3.3)
    Portal {
        /// Port the portal is listening on [default: 9090]
        #[arg(short, long, default_value = "9090")]
        port: u16,
    },
    /// Fetch and display Prometheus metrics from VeloceCore (v3.3)
    Metrics {
        /// Port the metrics endpoint is listening on [default: 9090]
        #[arg(short, long, default_value = "9090")]
        port: u16,
    },
    /// Discover, publish, and deploy applications from Veloce Hub (v3.4)
    Hub {
        #[command(subcommand)]
        action: HubAction,
    },
    /// Print version information
    Version,
}

#[derive(Subcommand, Debug)]
enum IngressAction {
    /// Add or update an HTTP ingress route
    Add {
        /// Hostname to match (e.g. app.vln, api.vln)
        #[arg(short = 'H', long, value_name = "HOST")]
        host: String,
        /// Path prefix to route (e.g. /api)
        #[arg(short = 'p', long, default_value = "/", value_name = "PREFIX")]
        path: String,
        /// Target backend port (e.g. 3000)
        #[arg(short = 't', long, value_name = "PORT")]
        target_port: u16,
        /// Strip prefix before forwarding to backend
        #[arg(long)]
        strip_prefix: bool,
        /// Enable TLS termination on HTTPS port 8443
        #[arg(long)]
        tls: bool,
        /// Path to custom TLS certificate in PEM format
        #[arg(long, value_name = "CERT_FILE")]
        cert: Option<PathBuf>,
        /// Path to custom TLS private key in PEM format
        #[arg(long, value_name = "KEY_FILE")]
        key: Option<PathBuf>,
    },
    /// Remove an HTTP ingress route by hostname
    Rm {
        /// Hostname to remove
        host: String,
    },
    /// List all active HTTP ingress routes
    List,
}

#[derive(Subcommand, Debug)]
enum AutoscaleAction {
    /// Attach or update an HPA autoscaling policy on a service
    Set {
        /// Service name to autoscale
        service: String,
        /// Minimum replicas
        #[arg(long, default_value = "1")]
        min: u32,
        /// Maximum replicas
        #[arg(long, default_value = "5")]
        max: u32,
        /// Target CPU percentage (1-100)
        #[arg(long)]
        cpu: Option<u32>,
        /// Target memory usage in megabytes
        #[arg(long)]
        mem: Option<u64>,
        /// Scale-up cooldown period in seconds
        #[arg(long, default_value = "30")]
        scale_up_cooldown: u32,
        /// Scale-down cooldown period in seconds
        #[arg(long, default_value = "60")]
        scale_down_cooldown: u32,
    },
    /// Get HPA status and live metrics for a service
    Get {
        /// Service name
        service: String,
    },
    /// Remove HPA autoscaling policy from a service
    Rm {
        /// Service name
        service: String,
    },
}

#[derive(Subcommand, Debug)]
enum CronAction {
    /// Create a scheduled task (Cron job)
    Create {
        /// Task name
        name: String,
        /// Schedule expression (e.g. "*/5 * * * *", "@hourly", "@daily", "@every 30s")
        #[arg(short = 's', long)]
        schedule: String,
        /// Concurrency policy ("Allow", "Forbid", "Replace")
        #[arg(long, default_value = "Allow")]
        concurrency: String,
        /// Executable command to run
        executable: String,
        /// Arguments for the command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// List all scheduled tasks
    List,
    /// Trigger a scheduled task immediately
    Run {
        /// Task name
        name: String,
    },
    /// Delete a scheduled task
    Rm {
        /// Task name
        name: String,
    },
}

#[derive(Subcommand, Debug)]
enum HubAction {
    /// List all applications available in the Veloce Hub catalog
    List,
    /// Search applications in the Hub catalog by keyword
    Search {
        /// Keyword to match against name, category, or description
        query: String,
    },
    /// Publish or register an application in the Hub catalog
    Publish {
        /// Application name
        name: String,
        /// Application category (e.g. Web, API, Database, Tools)
        #[arg(short = 'c', long, default_value = "Custom")]
        category: String,
        /// Description of the application
        #[arg(short = 'd', long, default_value = "")]
        description: String,
        /// Author / organization
        #[arg(short = 'a', long, default_value = "Community")]
        author: String,
        /// Executable command
        executable: String,
        /// Arguments for the command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
        /// Optional .vln hostname to register
        #[arg(short = 'H', long)]
        hostname: Option<String>,
        /// Optional port to expose
        #[arg(short = 'p', long)]
        port: Option<u16>,
        /// CPU limit in percent (1-100)
        #[arg(long)]
        cpu: Option<u8>,
        /// Memory limit in megabytes
        #[arg(long)]
        mem: Option<u64>,
        /// Enable TLS on HTTPS port 8443
        #[arg(long)]
        tls: bool,
    },
    /// Deploy an application directly from the Hub catalog into the mesh
    Deploy {
        /// Application name in the Hub catalog
        name: String,
    },
    /// Remove an application from the Hub catalog
    Rm {
        /// Application name to remove
        name: String,
    },
}

#[derive(Subcommand, Debug)]
enum MeshAction {
    /// Print this machine's join code (share it with a peer to connect)
    Identity {
        /// Join code TTL in minutes; 0 = no expiry [default: 15]
        #[arg(long, default_value = "15", value_name = "MINS")]
        ttl: u16,
        /// Produce a single-use code that expires after one successful connection
        #[arg(long, conflicts_with_all = ["ttl"])]
        one_time: bool,
    },
    /// Connect to a remote peer using its join code
    Join {
        /// Join code printed by `veloce-run mesh identity` on the remote machine
        code: String,
    },
    /// List all connected peers and their .vln hosts (compact table)
    Peers,
    /// Show rich mesh status: peers, RTT, traffic, join-code format
    Status,
    /// Run a self-diagnosis: identity, STUN, peer health, gossip stats
    Diagnose,
    /// Show the last-measured RTT latency to a specific peer
    Ping {
        /// Peer UUID from `veloce-run mesh peers`
        peer_id: uuid::Uuid,
    },
    /// Disconnect from a peer
    Leave {
        /// Peer UUID from `veloce-run mesh peers`
        peer_id: uuid::Uuid,
    },
    /// Manage the P2P replicated key-value mesh database (v3.5)
    Kv {
        #[command(subcommand)]
        action: MeshKvAction,
    },
}

#[derive(Subcommand, Debug)]
enum MeshKvAction {
    /// Set a key-value pair in the mesh database
    Set {
        /// Key name
        key: String,
        /// Value string
        value: String,
    },
    /// Get a key from the mesh database
    Get {
        /// Key name
        key: String,
    },
    /// List all key-value entries across the mesh
    List,
    /// Delete a key from the mesh database
    Rm {
        /// Key name to remove
        key: String,
    },
}

#[derive(Subcommand, Debug)]
enum PolicyAction {
    /// Print the currently active policy rules
    Show,
    /// Reload `veloce-policy.toml` from disk and print the new rules
    Reload,
}

#[derive(Subcommand, Debug)]
enum NrptAction {
    /// Install the NRPT rule: routes *.vln queries to VeloceNet DNS (127.0.0.1:5354)
    Enable,
    /// Remove the NRPT rule
    Disable,
    /// Show whether the NRPT rule is currently installed
    Status,
}

#[derive(Subcommand, Debug)]
enum SecretAction {
    /// Encrypt and store a secret value
    Set {
        /// Secret name (alphanumeric, _ and - only)
        name: String,
        /// Plaintext value to store
        #[arg(long, value_name = "VALUE")]
        value: String,
    },
    /// Delete a secret from the vault
    Rm {
        /// Secret name to remove
        name: String,
    },
    /// List all stored secret names (values are never shown)
    List,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn basename(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

async fn connect_client(name: &str, caps: Vec<Capability>) -> Result<VeloceClient> {
    eprintln!("veloce-run: connecting to VeloceCore…");
    let client = VeloceClient::connect(name, env!("CARGO_PKG_VERSION"), caps)
        .await
        .context("failed to connect to VeloceCore — is the service running?")?;
    eprintln!("veloce-run: connected (client_id={})", client.client_id);
    Ok(client)
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Mesh    { action }) => run_mesh(action).await,
        Some(Commands::Policy  { action }) => run_policy(action).await,
        Some(Commands::Nrpt    { action }) => {
            #[cfg(windows)]
            { run_nrpt(action) }
            #[cfg(not(windows))]
            {
                // On Linux, DNS config is managed by veloce-core directly via
                // /etc/resolv.conf or systemd-resolved; there is no NRPT.
                let _ = action;
                eprintln!("Use `veloce-core dns enable/disable` to manage .vln DNS routing on Linux.");
                Ok(())
            }
        }
        Some(Commands::Up   { file, detach }) => run_up(file, detach).await,
        Some(Commands::Down { file })         => run_down(file).await,
        Some(Commands::Ps)                    => run_ps().await,
        Some(Commands::Secret { action })     => run_secret(action).await,
        Some(Commands::Ingress { action })    => run_ingress(action).await,
        Some(Commands::Autoscale { action })  => run_autoscale(action).await,
        Some(Commands::Cron { action })       => run_cron(action).await,
        Some(Commands::Portal { port })       => run_portal(port).await,
        Some(Commands::Metrics { port })      => run_metrics(port).await,
        Some(Commands::Hub { action })        => run_hub(action).await,
        Some(Commands::Version)               => { run_version(); Ok(()) }
        None => {
            let executable = cli.executable.expect("clap ensures executable is set when no subcommand");
            run_spawn(
                executable, cli.args, cli.extra_env, cli.name, cli.hostname,
                cli.port, cli.cpu, cli.mem, cli.restarts, cli.detach,
            ).await
        }
    }
}

// ── Mesh subcommands ──────────────────────────────────────────────────────────

async fn run_mesh(action: MeshAction) -> Result<()> {
    // MeshManage is required for join/leave; MeshKvManage for mesh database operations.
    let mut client = connect_client("veloce-run-mesh", vec![Capability::MeshManage, Capability::MeshKvManage]).await?;
    match action {
        MeshAction::Identity { ttl, one_time } => {
            let info = client.mesh_info().await?;
            let join_code = if ttl != 15 || one_time {
                client.mesh_get_join_code_v3(ttl, one_time).await?
            } else {
                info.join_code.clone()
            };
            println!("{join_code}");
            eprintln!("machine_id:    {}", info.machine_id);
            eprintln!("listen_port:   {}", info.listen_port);
            if join_code.starts_with("VM2:") || join_code.starts_with("VM3:") {
                eprintln!("mode:          WAN-ready (STUN discovered external IP)");
            } else {
                eprintln!("mode:          LAN-only (STUN unreachable; WAN requires port forward :{}))",
                    info.listen_port);
            }
            if one_time {
                eprintln!("access:        single-use (one-time nonce)");
            }
            if ttl != 0 && join_code.starts_with("VM3:") {
                eprintln!("TTL:           {ttl}min");
            }
        }

        MeshAction::Join { code } => {
            let result = client.mesh_connect(&code).await?;
            println!("✓ connected to {} (peer_id={})", result.peer_name, result.peer_id);
        }

        MeshAction::Peers => {
            let peers = client.mesh_peers().await?;
            if peers.is_empty() {
                println!("No connected peers.");
                return Ok(());
            }
            println!("{:<36}  {:<20}  {:>8}  {}", "PEER ID", "NAME", "LATENCY", "REMOTE HOSTS");
            println!("{}", "-".repeat(90));
            for p in &peers {
                let hosts = if p.remote_hosts.is_empty() {
                    "(none)".to_owned()
                } else {
                    p.remote_hosts.join(", ")
                };
                let latency = if p.latency_ms == 0 {
                    "  pending".to_owned()
                } else {
                    format!("{:>5}ms", p.latency_ms)
                };
                println!("{:<36}  {:<20}  {}  {}",
                    p.peer_id, p.peer_name, latency, hosts);
            }
        }

        MeshAction::Status => {
            let info    = client.mesh_info().await?;
            let peers   = client.mesh_peers().await?;
            let traffic = client.query_traffic().await?;

            println!("═══════════════════════════  VeloceNet Mesh Status  ═══════════════════════════");
            println!("  Machine ID : {}", info.machine_id);
            println!("  Listen     : port {}", info.listen_port);
            let mode = if info.join_code.starts_with("VM2:") || info.join_code.starts_with("VM3:") {
                "WAN-ready"
            } else {
                "LAN-only"
            };
            println!("  Join mode  : {mode}");
            println!("  Peers      : {}", peers.len());
            println!();

            if peers.is_empty() {
                println!("  (no connected peers)");
            } else {
                println!("  {:<36}  {:<18}  {:>6}  {:>10}  {:>10}  HOSTS",
                    "PEER ID", "NAME", "RTT", "TX BYTES", "RX BYTES");
                println!("  {}", "-".repeat(100));
                for p in &peers {
                    let tunnel = traffic.tunnels.iter().find(|t| t.peer_id == p.peer_id);
                    let (tx, rx) = tunnel.map(|t| (t.tx_bytes, t.rx_bytes)).unwrap_or((0, 0));
                    let rtt = if p.latency_ms == 0 { " pending".to_owned() }
                              else { format!("{:>3}ms", p.latency_ms) };
                    let hosts_str = if p.remote_hosts.is_empty() {
                        "(none)".to_owned()
                    } else {
                        p.remote_hosts.join(", ")
                    };
                    println!("  {:<36}  {:<18}  {}  {:>10}  {:>10}  {}",
                        p.peer_id, p.peer_name, rtt, tx, rx, hosts_str);
                }
            }
            println!("════════════════════════════════════════════════════════════════════════════════");
        }

        MeshAction::Diagnose => {
            let info  = client.mesh_info().await?;
            let peers = client.mesh_peers().await?;

            println!("─── VeloceNet Mesh Diagnostic ───────────────────────────────────────────────");
            println!();
            println!("  [Identity]");
            println!("    machine_id   : {}", info.machine_id);
            println!("    listen_port  : {}", info.listen_port);
            println!("    join_code    : {} (format: {})",
                &info.join_code[..info.join_code.find(':').map(|i| i + 1).unwrap_or(4)],
                if info.join_code.starts_with("VM3:") { "VM3 — TTL+one-time capable" }
                else if info.join_code.starts_with("VM2:") { "VM2 — multi-address (WAN+LAN)" }
                else { "VM1 — single address (LAN only)" });
            println!();

            println!("  [STUN / WAN reachability]");
            if info.join_code.starts_with("VM2:") || info.join_code.starts_with("VM3:") {
                println!("    ✓  WAN IP discovered — mesh is reachable across the internet");
            } else {
                println!("    ⚠  No WAN IP — STUN may have failed or mesh_mode = \"lan-only\"");
                println!("       Peers on different networks must port-forward :{} → this host",
                    info.listen_port);
            }
            println!();

            println!("  [Peers] ({} connected)", peers.len());
            for p in &peers {
                let age_s = {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    now.saturating_sub(p.connected_since)
                };
                let rtt = if p.latency_ms == 0 { "pending".to_owned() }
                          else { format!("{}ms", p.latency_ms) };
                println!("    {} ({})  RTT={}  connected {}s ago  {} remote hosts",
                    p.peer_name, p.peer_id, rtt, age_s, p.remote_hosts.len());
                if !p.remote_hosts.is_empty() {
                    for h in &p.remote_hosts {
                        println!("        • {h}");
                    }
                }
            }
            if peers.is_empty() {
                println!("    (none)  — use `veloce-run mesh identity` to get a join code");
                println!("             then run `veloce-run mesh join <code>` on a remote machine");
            }
            println!();
            println!("─────────────────────────────────────────────────────────────────────────────");
        }

        MeshAction::Ping { peer_id } => {
            match client.mesh_ping_peer(peer_id).await? {
                Some(ms) => println!("RTT to {peer_id}: {ms}ms  (last keepalive measurement)"),
                None     => {
                    eprintln!("No latency sample for {peer_id}.");
                    eprintln!("Either the peer is not connected or no keepalive has completed yet");
                    eprintln!("(keepalives run every 30 seconds).");
                    std::process::exit(1);
                }
            }
        }

        MeshAction::Leave { peer_id } => {
            client.mesh_disconnect(peer_id).await?;
            println!("✓ disconnected from {peer_id}");
        }

        MeshAction::Kv { action } => {
            match action {
                MeshKvAction::Set { key, value } => {
                    client.mesh_kv_set(&key, &value).await?;
                    println!("✓ Mesh KV: set '{key}' = '{value}'");
                }
                MeshKvAction::Get { key } => {
                    match client.mesh_kv_get(&key).await? {
                        Some(val) => println!("{val}"),
                        None => {
                            eprintln!("Key '{key}' not found in Mesh database");
                            std::process::exit(1);
                        }
                    }
                }
                MeshKvAction::List => {
                    let list = client.mesh_kv_list().await?;
                    if list.is_empty() {
                        println!("No entries in Mesh database.");
                    } else {
                        println!("{:<24} {:<32} {:<8} {:<36}", "KEY", "VALUE", "VERSION", "ORIGIN PEER");
                        println!("{}", "─".repeat(105));
                        for e in list {
                            println!("{:<24} {:<32} {:<8} {:<36}", e.key, e.value, e.version, e.origin);
                        }
                    }
                }
                MeshKvAction::Rm { key } => {
                    client.mesh_kv_delete(&key).await?;
                    println!("✓ Mesh KV: deleted '{key}'");
                }
            }
        }
    }
    Ok(())
}

// ── Policy subcommands ────────────────────────────────────────────────────────

async fn run_policy(action: PolicyAction) -> Result<()> {
    // PolicyAdmin is required for `reload`; request it for `show` too so a single
    // connect covers both without needing to know the action upfront.
    let mut client = connect_client("veloce-run-policy", vec![Capability::PolicyAdmin]).await?;
    let rules = match action {
        PolicyAction::Show   => client.policy_get_rules().await?,
        PolicyAction::Reload => {
            let r = client.policy_reload().await?;
            eprintln!("✓ policy reloaded");
            r
        }
    };

    println!("default_effect: {}", rules.default_effect);

    if rules.rules.is_empty() {
        println!("\nCapability rules: (none — all capabilities allowed by default)");
    } else {
        println!("\nCapability rules ({}):", rules.rules.len());
        println!("  {:<30}  {:<10}  {}", "APP", "MODE", "CAPABILITIES");
        println!("  {}", "-".repeat(70));
        for r in &rules.rules {
            let (mode, caps) = if let Some(a) = &r.allow {
                ("allow", a.join(", "))
            } else if let Some(d) = &r.deny {
                ("deny", d.join(", "))
            } else {
                ("(none)", String::new())
            };
            println!("  {:<30}  {:<10}  {}", r.app, mode, caps);
        }
    }

    if rules.mesh_acls.is_empty() {
        println!("\nMesh ACL rules: (none — all gossip allowed by default)");
    } else {
        println!("\nMesh ACL rules ({}):", rules.mesh_acls.len());
        println!("  {:<30}  {:<20}  {}", "HOSTNAME", "FROM PEER", "EFFECT");
        println!("  {}", "-".repeat(65));
        for a in &rules.mesh_acls {
            let peer = a.from_peer.as_deref().unwrap_or("*");
            println!("  {:<30}  {:<20}  {}", a.hostname, peer, a.effect);
        }
    }

    Ok(())
}

// ── NRPT subcommand ───────────────────────────────────────────────────────────

#[cfg(windows)]
fn run_nrpt(action: NrptAction) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use winreg::{enums::*, RegKey};

        const BASE: &str =
            r"SYSTEM\CurrentControlSet\Services\Dnscache\Parameters\DnsPolicyConfig";
        const RULE: &str = "VeloceNetwork-VLN";
        const DNS:  &str = "127.0.0.1:5354";

        match action {
            NrptAction::Enable => {
                let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
                let base = hklm
                    .open_subkey_with_flags(BASE, KEY_CREATE_SUB_KEY)
                    .map_err(|e| anyhow::anyhow!(
                        "Cannot open DnsPolicyConfig key: {e}\n\
                         Run this command as Administrator."
                    ))?;
                let (rule, _) = base.create_subkey(RULE)?;
                rule.set_value("Version",           &2u32)?;
                rule.set_value("ConfigOptions",     &8u32)?;
                rule.set_value("Name",              &vec![".vln".to_owned()])?;
                rule.set_value("GenericDNSServers", &DNS)?;
                rule.set_value("Comment",           &"VeloceNetwork .vln private namespace")?;

                // Best-effort Dnscache restart for immediate effect
                let _ = std::process::Command::new("net").args(["stop", "Dnscache"]).output();
                let _ = std::process::Command::new("net").args(["start", "Dnscache"]).output();

                println!("NRPT rule installed.  *.vln → {DNS}");
                println!("System-wide .vln DNS is now active (no VELOCE_DNS needed).");
            }
            NrptAction::Disable => {
                let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
                if let Ok(base) = hklm.open_subkey_with_flags(BASE, KEY_WRITE) {
                    let _ = base.delete_subkey_all(RULE);
                }
                let _ = std::process::Command::new("net").args(["stop", "Dnscache"]).output();
                let _ = std::process::Command::new("net").args(["start", "Dnscache"]).output();
                println!("NRPT rule removed.  Set VELOCE_DNS=127.0.0.1:5354 for per-process routing.");
            }
            NrptAction::Status => {
                let installed = RegKey::predef(HKEY_LOCAL_MACHINE)
                    .open_subkey(format!("{BASE}\\{RULE}"))
                    .is_ok();
                if installed {
                    let addr: String = RegKey::predef(HKEY_LOCAL_MACHINE)
                        .open_subkey(format!("{BASE}\\{RULE}"))
                        .and_then(|k| k.get_value("GenericDNSServers"))
                        .unwrap_or_else(|_| "?".into());
                    println!("NRPT:    installed");
                    println!("DNS:     {addr}");
                    println!("Effect:  *.vln queries route to VeloceNet DNS system-wide");
                } else {
                    println!("NRPT:    not installed");
                    println!("Hint:    run `veloce-run nrpt enable` as Administrator to activate");
                    println!("         or set VELOCE_DNS=127.0.0.1:5354 for per-process routing");
                }
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = action;
        // On Linux, .vln DNS routing is managed by veloce-core via
        // systemd-resolved or /etc/resolv.conf — not via Windows NRPT.
        // The VELOCE_SOCKET environment variable points to the Unix socket
        // (equivalent of VELOCE_PIPE on Windows).
        eprintln!("NRPT is a Windows-only feature.");
        eprintln!("On Linux, use `veloce-core dns enable/disable` to manage .vln DNS routing.");
        Ok(())
    }
}

// ── Compose up/down/ps ────────────────────────────────────────────────────────

async fn run_up(file: Option<PathBuf>, detach: bool) -> Result<()> {
    let path = file.unwrap_or_else(|| PathBuf::from("veloce-compose.yml"));
    let compose_file = compose::load(&path)?;

    let caps = vec![
        Capability::SpawnNodes, Capability::KillNodes,
        Capability::RegistryRead, Capability::RegistryWrite,
        Capability::NetPortForward, Capability::DesiredStateManage,
        Capability::SecretsRead, Capability::SecretsWrite,
    ];
    let mut client = connect_client("veloce-compose", caps).await?;

    // Pre-load file-backed secrets declared in the compose file.
    for (secret_name, src) in &compose_file.secrets {
        if let Some(ref file_path) = src.file {
            let plaintext = std::fs::read_to_string(file_path)
                .with_context(|| format!("read secret file '{}' for '{}'", file_path, secret_name))?;
            client.secret_set(secret_name, plaintext.trim()).await
                .with_context(|| format!("store secret '{secret_name}'"))?;
            eprintln!("veloce-compose: ✓ secret '{secret_name}' loaded from {file_path}");
        }
    }

    // Pre-create named volumes declared in the compose file.
    for volume_name in compose_file.volumes.keys() {
        let v = client.volume_register(volume_name).await
            .with_context(|| format!("register volume '{volume_name}'"))?;
        eprintln!("veloce-compose: ✓ volume '{}' → {}", volume_name, v.host_path);
    }

    // Build desired state and apply it.
    let compose_name = path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "compose".into());
    let spec = compose::to_desired_state(&compose_file, &compose_name)?;
    eprintln!("veloce-compose: applying desired state '{}' ({} services)…",
        spec.name, spec.services.len());
    client.apply_desired_state(spec).await?;
    eprintln!("veloce-compose: ✓ desired state applied — reconciler is converging");

    // Register port forwards, autoscaling, cron schedules, and ingress rules.
    let order = compose::topo_sort(&compose_file.services)?;
    for svc_name in &order {
        let svc = &compose_file.services[svc_name];

        // 1. Port forwards
        for port_str in &svc.ports {
            let (host_port, target_port) = compose::parse_port_mapping(port_str)
                .with_context(|| format!("service '{}': port '{}'", svc_name, port_str))?;
            let forward_name = format!("{svc_name}-{host_port}");
            client.add_port_forward(NetPortForwardMsg {
                name:        forward_name.clone(),
                host_port,
                target_port,
                node_id:     None,
            }).await.with_context(|| format!("add port forward {host_port}:{target_port}"))?;
            eprintln!("veloce-compose: ✓ port forward {host_port} → {target_port}  ({forward_name})");
        }

        // 2. Autoscaling
        if let Some(ref auto) = svc.autoscaling {
            client.autoscale_set(AutoscalePolicyMsg {
                service_name: svc_name.clone(),
                min_replicas: auto.min_replicas,
                max_replicas: auto.max_replicas,
                target_cpu_percent: auto.target_cpu,
                target_memory_mb: auto.target_memory_mb,
                scale_up_cooldown_secs: 30,
                scale_down_cooldown_secs: 60,
            }).await.with_context(|| format!("service '{}': configure autoscaling", svc_name))?;
            eprintln!("veloce-compose: ✓ autoscaling configured for '{svc_name}' (min: {}, max: {})", auto.min_replicas, auto.max_replicas);
        }

        // 3. Cron schedule
        if let Some(ref cron) = svc.cron {
            let job_name = format!("{compose_name}-{svc_name}");
            client.cron_create(CronJobMsg {
                name: job_name.clone(),
                schedule: cron.schedule.clone(),
                executable: svc.executable.clone(),
                args: svc.args.clone(),
                concurrency_policy: cron.concurrency.clone(),
                enabled: true,
                last_run_timestamp_secs: None,
                last_run_status: None,
                next_run_timestamp_secs: None,
            }).await.with_context(|| format!("service '{}': configure cron schedule", svc_name))?;
            eprintln!("veloce-compose: ✓ cron scheduled for '{svc_name}' ({})", cron.schedule);
        }

        // 4. Ingress route
        if let Some(ref ing) = svc.ingress {
            let target_port = svc.ports.first()
                .and_then(|p| compose::parse_port_mapping(p).ok().map(|(_, node_port)| node_port))
                .unwrap_or(8080);
            client.ingress_add(IngressRule {
                host: ing.host.clone(),
                paths: vec![IngressPathRule {
                    path_prefix: ing.path.clone(),
                    target_port,
                    strip_prefix: ing.strip_prefix,
                }],
                default_port: Some(target_port),
                tls_enabled: ing.tls,
                tls_cert_pem: ing.cert.clone(),
                tls_key_pem: ing.key.clone(),
            }).await.with_context(|| format!("service '{}': add ingress route for '{}'", svc_name, ing.host))?;
            eprintln!("veloce-compose: ✓ ingress route {} → port {} ({})", ing.host, target_port, ing.path);
        }
    }

    if !detach {
        eprintln!("veloce-compose: services are starting (run `veloce-run ps` to check status)");
    }
    Ok(())
}

async fn run_down(file: Option<PathBuf>) -> Result<()> {
    let path = file.unwrap_or_else(|| PathBuf::from("veloce-compose.yml"));
    let compose_file = compose::load(&path)?;

    let caps = vec![
        Capability::KillNodes, Capability::DesiredStateManage, Capability::NetPortForward,
        Capability::NetRegister,
    ];
    let mut client = connect_client("veloce-compose", caps).await?;

    // Clear the desired state so the reconciler stops managing these services.
    let compose_name = path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "compose".into());

    // Apply an empty desired state spec to clear reconciler.
    client.apply_desired_state(veloce_ipc::message::DesiredStateSpec {
        name:     compose_name.clone(),
        services: vec![],
    }).await?;

    // Kill all running nodes for services in the compose file.
    let statuses = client.query_node_status().await?;
    let service_names: std::collections::HashSet<&str> = compose_file.services.keys()
        .map(|s| s.as_str()).collect();

    let mut killed = 0usize;
    for s in &statuses {
        if s.service_name.as_deref().map(|n| service_names.contains(n)).unwrap_or(false) {
            match client.kill_node(s.node_id).await {
                Ok(_)  => { killed += 1; }
                Err(e) => eprintln!("veloce-compose: warn: kill {}: {e}", s.node_id),
            }
        }
    }
    eprintln!("veloce-compose: ✓ {killed} node(s) stopped");

    // Remove port forwards, autoscaling policies, cron jobs, and ingress rules.
    let forwards = client.list_port_forwards().await?;
    for f in &forwards {
        // Only remove forwards whose names start with a service name from this file.
        let belongs = service_names.iter().any(|n| f.name.starts_with(*n));
        if belongs {
            client.remove_port_forward(&f.name).await.ok();
        }
    }

    for svc_name in &service_names {
        let _ = client.autoscale_remove(svc_name).await;
        let _ = client.cron_remove(&format!("{compose_name}-{svc_name}")).await;
        if let Some(svc) = compose_file.services.get(*svc_name) {
            if let Some(ref ing) = svc.ingress {
                let _ = client.ingress_remove(&ing.host).await;
            }
        }
    }

    eprintln!("veloce-compose: ✓ down complete");
    Ok(())
}

async fn run_ps() -> Result<()> {
    let mut client = connect_client("veloce-run-ps", vec![]).await?;
    let statuses = client.query_node_status().await?;

    if statuses.is_empty() {
        println!("No running nodes.");
        return Ok(());
    }

    println!("{:<36}  {:<20}  {:<12}  {:<12}  {:>7}  {}",
        "NODE ID", "APP / SERVICE", "HEALTH", "REPLICA", "PID", "SPAWNED");
    println!("{}", "-".repeat(110));
    for s in &statuses {
        let svc = match (&s.service_name, s.replica_index) {
            (Some(n), Some(i)) => format!("{n}[{i}]"),
            (Some(n), None)    => n.clone(),
            (None,    _)       => s.app_name.clone(),
        };
        let health = format!("{:?}", s.health);
        let replica = s.replica_index.map(|i| i.to_string()).unwrap_or_default();
        println!("{:<36}  {:<20}  {:<12}  {:<12}  {:>7}  {}",
            s.node_id, svc, health, replica, s.pid,
            s.spawned_at.format("%H:%M:%S"));
    }
    Ok(())
}

// ── Secrets subcommand ────────────────────────────────────────────────────────

async fn run_secret(action: SecretAction) -> Result<()> {
    let caps = vec![Capability::SecretsRead, Capability::SecretsWrite];
    let mut client = connect_client("veloce-run-secret", caps).await?;

    match action {
        SecretAction::Set { name, value } => {
            client.secret_set(&name, &value).await?;
            eprintln!("✓ secret '{name}' stored");
        }
        SecretAction::Rm { name } => {
            client.secret_delete(&name).await?;
            eprintln!("✓ secret '{name}' deleted");
        }
        SecretAction::List => {
            let names = client.secret_list().await?;
            if names.is_empty() {
                println!("No secrets stored.");
            } else {
                for n in &names { println!("{n}"); }
            }
        }
    }
    Ok(())
}

// ── Version subcommand ────────────────────────────────────────────────────────

fn run_version() {
    println!("VeloceNetwork v{}", env!("CARGO_PKG_VERSION"));
    println!("Components: veloce-core · veloce-net · veloce-mesh · veloce-sdk");
    println!();
    println!("Dashboard:  veloce-dashboard --version");
    println!("Docs:       https://github.com/LeTrollologist/veloce-workspace");
}

// ── Spawn mode ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn run_spawn(
    executable: String,
    args: Vec<String>,
    extra_env: Vec<String>,
    name: Option<String>,
    hostname: Option<String>,
    port: Option<u16>,
    cpu: Option<u8>,
    mem: Option<u64>,
    restarts: u32,
    detach: bool,
) -> Result<()> {
    let app_name = name.unwrap_or_else(|| basename(&executable));

    let mut caps = vec![
        Capability::SpawnNodes,
        Capability::KillNodes,
        Capability::RegistryRead,
    ];
    if hostname.is_some() { caps.push(Capability::NetRegister); }

    let mut client = connect_client(&app_name, caps).await?;

    let limits = if cpu.is_some() || mem.is_some() {
        Some(NodeLimits {
            cpu_pct:           cpu.map(|c| c as u32),
            mem_mb:            mem,
            max_lifetime_secs: None,
        })
    } else {
        None
    };

    let restart_policy = if restarts > 0 {
        Some(RestartPolicy {
            max_restarts:    restarts,
            base_delay_secs: 1,
            max_delay_secs:  30,
        })
    } else {
        None
    };

    // Parse --env KEY=VALUE pairs
    let env: Vec<(String, String)> = extra_env.iter().filter_map(|e| {
        let mut parts = e.splitn(2, '=');
        Some((parts.next()?.to_owned(), parts.next().unwrap_or("").to_owned()))
    }).collect();

    let msg = SpawnNodeMsg {
        app_name:         app_name.clone(),
        executable:       executable.clone(),
        args:             args.clone(),
        env,
        limits,
        auto_kill:        !detach,
        restart_policy,
        use_appcontainer: false,
        health_check:     None,
        volume_mounts:    vec![],
        secret_refs:      vec![],
        service_name:     None,
        replica_index:    None,
    };

    let spawned = client.spawn_node_with(msg).await.context("spawn_node failed")?;
    eprintln!("veloce-run: ✓ spawned  node_id={}  pid={}", spawned.node_id, spawned.pid);

    if let (Some(host), Some(p)) = (&hostname, port) {
        client.register_host(host, spawned.node_id, p, 3600)
            .await
            .context("register_host failed")?;
        eprintln!("veloce-run: ✓ {host} → 127.0.0.1:{p}");
    }

    if detach {
        println!("{}", spawned.node_id);
        return Ok(());
    }

    let mut logs = client
        .subscribe_node_logs(spawned.node_id)
        .await
        .context("subscribe_node_logs failed")?;

    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
    let stop_tx = std::sync::Mutex::new(Some(stop_tx));
    ctrlc::set_handler(move || {
        if let Some(tx) = stop_tx.lock().unwrap().take() { let _ = tx.send(()); }
    })
    .context("failed to install Ctrl-C handler")?;

    eprintln!("veloce-run: streaming logs (Ctrl-C to stop and kill node)…");

    loop {
        tokio::select! {
            chunk = logs.next() => match chunk {
                Some(c) => {
                    let text = String::from_utf8_lossy(&c.data);
                    match c.stream {
                        LogStream::Stdout => print!("{text}"),
                        LogStream::Stderr => eprint!("{text}"),
                    }
                }
                None => { eprintln!("\nveloce-run: node exited."); break; }
            },
            _ = &mut stop_rx => {
                eprintln!("\nveloce-run: stopping…");
                let _ = client.kill_node(spawned.node_id).await;
                eprintln!("veloce-run: node killed.");
                break;
            }
        }
    }
    Ok(())
}

async fn run_ingress(action: IngressAction) -> Result<()> {
    let mut client = connect_client("veloce-run-ingress", vec![Capability::NetRegister]).await?;
    match action {
        IngressAction::Add { host, path, target_port, strip_prefix, tls, cert, key } => {
            let (cert_pem, key_pem) = match (cert, key) {
                (Some(c), Some(k)) => {
                    let c_str = std::fs::read_to_string(&c).with_context(|| format!("read cert file {:?}", c))?;
                    let k_str = std::fs::read_to_string(&k).with_context(|| format!("read key file {:?}", k))?;
                    (Some(c_str), Some(k_str))
                }
                (Some(_), None) | (None, Some(_)) => {
                    anyhow::bail!("both --cert and --key must be specified together");
                }
                (None, None) => (None, None),
            };

            let is_tls = tls || cert_pem.is_some();

            let rule = veloce_ipc::message::IngressRule {
                host: host.clone(),
                paths: vec![veloce_ipc::message::IngressPathRule {
                    path_prefix: path.clone(),
                    target_port,
                    strip_prefix,
                }],
                default_port: Some(target_port),
                tls_enabled: is_tls,
                tls_cert_pem: cert_pem,
                tls_key_pem: key_pem,
            };
            let confirmed = client.ingress_add(rule).await?;
            let proto = if is_tls { "https" } else { "http" };
            let port = if is_tls { 8443 } else { 8080 };
            println!("✓ ingress route added: {proto}://{confirmed}:{port}{path} → 127.0.0.1:{target_port}");
        }
        IngressAction::Rm { host } => {
            client.ingress_remove(&host).await?;
            println!("✓ ingress route removed for {host}");
        }
        IngressAction::List => {
            let rules = client.ingress_list().await?;
            if rules.is_empty() {
                println!("No active ingress routes. Use `veloce-run ingress add` to create one.");
            } else {
                println!("{:<25} {:<15} {:<12} {:<8} {}", "HOST", "PATH PREFIX", "TARGET PORT", "TLS", "STRIP PREFIX");
                println!("{}", "─".repeat(75));
                for r in rules {
                    let tls_str = if r.tls_enabled { "yes" } else { "no" };
                    for p in r.paths {
                        println!("{:<25} {:<15} {:<12} {:<8} {}", r.host, p.path_prefix, p.target_port, tls_str, p.strip_prefix);
                    }
                }
            }
        }
    }
    Ok(())
}

async fn run_autoscale(action: AutoscaleAction) -> Result<()> {
    let mut client = connect_client("veloce-run-autoscale", vec![Capability::DesiredStateManage]).await?;
    match action {
        AutoscaleAction::Set { service, min, max, cpu, mem, scale_up_cooldown, scale_down_cooldown } => {
            let policy = veloce_ipc::message::AutoscalePolicyMsg {
                service_name: service.clone(),
                min_replicas: min,
                max_replicas: max,
                target_cpu_percent: cpu,
                target_memory_mb: mem,
                scale_up_cooldown_secs: scale_up_cooldown,
                scale_down_cooldown_secs: scale_down_cooldown,
            };
            let info = client.autoscale_set(policy).await?;
            println!("✓ HPA policy configured for service '{service}' (min: {min}, max: {max})");
            if let Some(inf) = info {
                println!("  current replicas: {}, target cpu: {:?}%, target mem: {:?}MB", inf.current_replicas, inf.target_cpu_percent, inf.target_memory_mb);
            }
        }
        AutoscaleAction::Get { service } => {
            let info = client.autoscale_get(&service).await?;
            if let Some(inf) = info {
                println!("─── HPA Status: {} ────────────────────────────", inf.service_name);
                println!("  Min Replicas : {}", inf.min_replicas);
                println!("  Max Replicas : {}", inf.max_replicas);
                println!("  Current      : {} replicas", inf.current_replicas);
                println!("  Target CPU   : {:?}", inf.target_cpu_percent);
                println!("  Target Mem   : {:?}", inf.target_memory_mb);
                println!("  Current CPU  : {:.1}%", inf.current_cpu_percent);
                println!("  Current Mem  : {} MB", inf.current_memory_mb);
            } else {
                println!("No HPA policy found for service '{service}'.");
            }
        }
        AutoscaleAction::Rm { service } => {
            client.autoscale_remove(&service).await?;
            println!("✓ HPA policy removed for service '{service}'");
        }
    }
    Ok(())
}

async fn run_cron(action: CronAction) -> Result<()> {
    let mut client = connect_client("veloce-run-cron", vec![Capability::SpawnNodes]).await?;
    match action {
        CronAction::Create { name, schedule, concurrency, executable, args } => {
            let job = veloce_ipc::message::CronJobMsg {
                name: name.clone(),
                schedule: schedule.clone(),
                executable,
                args,
                concurrency_policy: concurrency,
                enabled: true,
                last_run_timestamp_secs: None,
                last_run_status: None,
                next_run_timestamp_secs: None,
            };
            client.cron_create(job).await?;
            println!("✓ Scheduled task '{name}' created with schedule '{schedule}'");
        }
        CronAction::List => {
            let jobs = client.cron_list().await?;
            if jobs.is_empty() {
                println!("No scheduled tasks found. Use `veloce-run cron create` to register one.");
            } else {
                println!("{:<20} {:<15} {:<12} {:<12} {}", "NAME", "SCHEDULE", "POLICY", "LAST STATUS", "COMMAND");
                println!("{}", "─".repeat(75));
                for j in jobs {
                    let status = j.last_run_status.as_deref().unwrap_or("Never Run");
                    println!("{:<20} {:<15} {:<12} {:<12} {} {}", j.name, j.schedule, j.concurrency_policy, status, j.executable, j.args.join(" "));
                }
            }
        }
        CronAction::Run { name } => {
            client.cron_trigger(&name).await?;
            println!("✓ Scheduled task '{name}' triggered for immediate execution");
        }
        CronAction::Rm { name } => {
            client.cron_remove(&name).await?;
            println!("✓ Scheduled task '{name}' removed");
        }
    }
    Ok(())
}

async fn run_portal(port: u16) -> Result<()> {
    let url = format!("http://127.0.0.1:{port}");
    println!("Opening VeloceNetwork Web Status Portal at {url} …");
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd").args(["/C", "start", &url]).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(&url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    }
    Ok(())
}

async fn run_metrics(port: u16) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .with_context(|| format!("connect to metrics endpoint on 127.0.0.1:{port} — is VeloceCore running?"))?;

    let req = format!("GET /metrics HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await?;

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    let resp = String::from_utf8_lossy(&buf);

    if let Some(idx) = resp.find("\r\n\r\n") {
        print!("{}", &resp[idx + 4..]);
    } else {
        print!("{}", resp);
    }
    Ok(())
}

async fn run_hub(action: HubAction) -> Result<()> {
    let mut client = connect_client("veloce-run-hub", vec![Capability::HubManage, Capability::SpawnNodes, Capability::NetRegister]).await?;
    match action {
        HubAction::List => {
            let apps = client.hub_list().await?;
            if apps.is_empty() {
                println!("No applications found in Veloce Hub catalog.");
            } else {
                println!("{:<18} {:<10} {:<10} {:<18} {}", "APP NAME", "CATEGORY", "VERSION", "HOSTNAME", "DESCRIPTION");
                println!("{}", "─".repeat(80));
                for a in apps {
                    let host = a.hostname.as_deref().unwrap_or("-");
                    println!("{:<18} {:<10} {:<10} {:<18} {}", a.name, a.category, a.version, host, a.description);
                }
            }
        }
        HubAction::Search { query } => {
            let apps = client.hub_list().await?;
            let q = query.to_lowercase();
            let matches: Vec<_> = apps.into_iter().filter(|a| {
                a.name.to_lowercase().contains(&q)
                    || a.description.to_lowercase().contains(&q)
                    || a.category.to_lowercase().contains(&q)
                    || a.author.to_lowercase().contains(&q)
            }).collect();
            if matches.is_empty() {
                println!("No applications matching '{query}' found in Veloce Hub.");
            } else {
                println!("{:<18} {:<10} {:<10} {:<18} {}", "APP NAME", "CATEGORY", "VERSION", "HOSTNAME", "DESCRIPTION");
                println!("{}", "─".repeat(80));
                for a in matches {
                    let host = a.hostname.as_deref().unwrap_or("-");
                    println!("{:<18} {:<10} {:<10} {:<18} {}", a.name, a.category, a.version, host, a.description);
                }
            }
        }
        HubAction::Publish { name, category, description, author, executable, args, hostname, port, cpu, mem, tls } => {
            let app = veloce_ipc::message::HubAppMsg {
                name: name.clone(),
                version: "1.0.0".into(),
                description,
                category,
                author,
                executable,
                args,
                env: vec![],
                port,
                hostname,
                cpu,
                mem,
                replicas: 1,
                auto_restart: true,
                tls,
            };
            client.hub_publish(app).await?;
            println!("✓ Application '{name}' published to Veloce Hub catalog");
        }
        HubAction::Deploy { name } => {
            let node_id = client.hub_deploy(&name).await?;
            println!("✓ Deployed application '{name}' from Veloce Hub (node_id={node_id})");
        }
        HubAction::Rm { name } => {
            client.hub_remove(&name).await?;
            println!("✓ Application '{name}' removed from Veloce Hub catalog");
        }
    }
    Ok(())
}


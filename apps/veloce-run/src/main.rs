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

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use veloce_ipc::message::{Capability, LogStream, NodeLimits, RestartPolicy, SpawnNodeMsg};
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
    /// Print version information
    Version,
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
        Some(Commands::Nrpt    { action }) => run_nrpt(action),
        Some(Commands::Version)            => { run_version(); Ok(()) }
        None => {
            let executable = cli.executable.expect("clap ensures executable is set when no subcommand");
            run_spawn(
                executable, cli.args, cli.name, cli.hostname,
                cli.port, cli.cpu, cli.mem, cli.restarts, cli.detach,
            ).await
        }
    }
}

// ── Mesh subcommands ──────────────────────────────────────────────────────────

async fn run_mesh(action: MeshAction) -> Result<()> {
    // MeshManage is required for join/leave; harmless to declare for identity/peers too.
    let mut client = connect_client("veloce-run-mesh", vec![Capability::MeshManage]).await?;
    match action {
        MeshAction::Identity { ttl, one_time } => {
            let info = client.mesh_info().await?;
            // The core returns the current join code in info.join_code (VM1 or VM2).
            // We print a VM3 code (with TTL/one-time) using the same addresses but
            // note that the actual VM3 encoding must be done server-side since the
            // private key is not available here.  For now, print the existing join code
            // with advisory TTL information so operators know what they're distributing.
            //
            // TODO: add a `MeshGetJoinCodeV3` IPC message to let Core generate VM3 codes.
            //       Until then we display the current join code plus TTL advisory.
            let join_code = &info.join_code;
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
                eprintln!("⚠  --one-time: VM3 code generation not yet wired to Core IPC");
                eprintln!("   This code above is NOT single-use.  Support is planned for v0.9.1.");
            } else if ttl != 15 {
                eprintln!("TTL hint:      {ttl}min  (advisory — not enforced by this code format)");
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
        eprintln!("NRPT is a Windows-only feature.");
        Ok(())
    }
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

    let msg = SpawnNodeMsg {
        app_name:        app_name.clone(),
        executable:      executable.clone(),
        args:            args.clone(),
        env:             vec![],
        limits,
        auto_kill:       !detach,
        restart_policy,
        use_appcontainer: false,
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

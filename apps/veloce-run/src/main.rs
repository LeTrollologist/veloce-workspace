/*!
veloce-run — launch any executable into the VeloceNetwork mesh.

Usage:
    veloce-run [OPTIONS] <EXECUTABLE> [ARGS]...
    veloce-run mesh identity
    veloce-run mesh join <CODE>
    veloce-run mesh peers
    veloce-run mesh leave <PEER_ID>

Examples:
    # Wrap ping and stream its output to the terminal
    veloce-run --watch -- ping -t 127.0.0.1

    # Start a Node.js server, register a .vln hostname, detach
    veloce-run --name api --hostname api.vln --port 3000 --detach -- node server.js

    # Print this machine's join code for another machine to use
    veloce-run mesh identity

    # Connect to a peer (run on Machine B, paste code from Machine A)
    veloce-run mesh join "VM1:AAA..."
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
}

#[derive(Subcommand, Debug)]
enum MeshAction {
    /// Print this machine's join code (share it with a peer to connect)
    Identity,
    /// Connect to a remote peer using its join code
    Join {
        /// Join code printed by `veloce-run mesh identity` on the remote machine
        code: String,
    },
    /// List all connected peers and their .vln hosts
    Peers,
    /// Disconnect from a peer
    Leave {
        /// Peer UUID from `veloce-run mesh peers`
        peer_id: uuid::Uuid,
    },
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
        Some(Commands::Mesh { action }) => run_mesh(action).await,
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
    let mut client = connect_client("veloce-run-mesh", vec![]).await?;
    match action {
        MeshAction::Identity => {
            let info = client.mesh_info().await?;
            println!("{}", info.join_code);
            eprintln!("machine_id: {}", info.machine_id);
            eprintln!("listening on port: {}", info.listen_port);
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
                println!("{:<36}  {:<20}  {:>7}ms  {}",
                    p.peer_id, p.peer_name, p.latency_ms, hosts);
            }
        }

        MeshAction::Leave { peer_id } => {
            client.mesh_disconnect(peer_id).await?;
            println!("✓ disconnected from {peer_id}");
        }
    }
    Ok(())
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

/*!
veloce-run — launch any executable into the VeloceNetwork mesh.

Usage:
    veloce-run [OPTIONS] <EXECUTABLE> [ARGS]...

Examples:
    # Wrap ping and stream its output to the terminal
    veloce-run --watch -- ping -t 127.0.0.1

    # Start a Node.js server, register a .vln hostname, detach
    veloce-run --name api --hostname api.vln --port 3000 --detach -- node server.js

    # Run with crash-restart and resource limits
    veloce-run --cpu 25 --mem 512 --restarts 5 -- worker.exe
*/

use anyhow::{Context, Result};
use clap::Parser;
use veloce_ipc::message::{Capability, LogStream, NodeLimits, RestartPolicy, SpawnNodeMsg};
use veloce_sdk::VeloceClient;

// ── CLI args ──────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name    = "veloce-run",
    about   = "Launch any executable into the VeloceNetwork mesh",
    version,
    // allow `veloce-run -- my.exe arg1 arg2`
    trailing_var_arg = true,
)]
struct Cli {
    /// Executable to launch (path or name on PATH)
    #[arg(required = true)]
    executable: String,

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

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Extract the last component (sans extension) of a path string.
fn basename(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Default app name: basename of the executable
    let app_name = cli
        .name
        .clone()
        .unwrap_or_else(|| basename(&cli.executable));

    // Determine capabilities we need
    let mut caps = vec![
        Capability::SpawnNodes,
        Capability::KillNodes,
        Capability::RegistryRead,
    ];
    if cli.hostname.is_some() {
        caps.push(Capability::NetRegister);
    }

    eprintln!("veloce-run: connecting to VeloceCore…");
    let mut client = VeloceClient::connect(&app_name, env!("CARGO_PKG_VERSION"), caps)
        .await
        .context("failed to connect to VeloceCore — is the service running?")?;
    eprintln!("veloce-run: connected (client_id={})", client.client_id);

    // ── Build spawn message ───────────────────────────────────────────────────
    let limits = if cli.cpu.is_some() || cli.mem.is_some() {
        Some(NodeLimits {
            cpu_pct:          cli.cpu.map(|c| c as u32),
            mem_mb:           cli.mem,
            max_lifetime_secs: None,
        })
    } else {
        None
    };

    let restart_policy = if cli.restarts > 0 {
        Some(RestartPolicy {
            max_restarts:    cli.restarts,
            base_delay_secs: 1,
            max_delay_secs:  30,
        })
    } else {
        None
    };

    let msg = SpawnNodeMsg {
        app_name:       app_name.clone(),
        executable:     cli.executable.clone(),
        args:           cli.args.clone(),
        env:            vec![],
        limits,
        auto_kill:        !cli.detach,  // kill node when this process exits, unless detached
        restart_policy,
        use_appcontainer: false,
    };

    // ── Spawn ─────────────────────────────────────────────────────────────────
    let spawned = client
        .spawn_node_with(msg)
        .await
        .context("spawn_node failed")?;

    eprintln!(
        "veloce-run: ✓ spawned  node_id={}  pid={}",
        spawned.node_id, spawned.pid
    );

    // ── Optional .vln registration ────────────────────────────────────────────
    if let (Some(host), Some(port)) = (&cli.hostname, cli.port) {
        client
            .register_host(host, spawned.node_id, port, 3600)
            .await
            .context("register_host failed")?;
        eprintln!("veloce-run: ✓ {host} → 127.0.0.1:{port}");
    }

    // ── Detach mode: print node ID and exit ───────────────────────────────────
    if cli.detach {
        println!("{}", spawned.node_id);
        return Ok(());
    }

    // ── Watch / stream mode ───────────────────────────────────────────────────
    let mut logs = client
        .subscribe_node_logs(spawned.node_id)
        .await
        .context("subscribe_node_logs failed")?;

    // Channel to signal the log loop to exit cleanly on Ctrl-C
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
    let stop_tx = std::sync::Mutex::new(Some(stop_tx));

    ctrlc::set_handler(move || {
        if let Some(tx) = stop_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    })
    .context("failed to install Ctrl-C handler")?;

    eprintln!("veloce-run: streaming logs (Ctrl-C to stop and kill node)…");

    loop {
        tokio::select! {
            chunk = logs.next() => {
                match chunk {
                    Some(c) => {
                        let text = String::from_utf8_lossy(&c.data);
                        match c.stream {
                            LogStream::Stdout => print!("{text}"),
                            LogStream::Stderr => eprint!("{text}"),
                        }
                    }
                    // Core closed the log channel — node has exited
                    None => {
                        eprintln!("\nveloce-run: node exited.");
                        break;
                    }
                }
            }
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

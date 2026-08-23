/*!
VeloceCore — entry point.

Run modes:
  veloce-core run            → foreground (dev)
  veloce-core install        → register as Windows service / install systemd unit
  veloce-core uninstall      → remove Windows service / remove systemd unit
  veloce-core start          → start the service (SCM / systemctl)
  veloce-core stop           → stop the service (SCM / systemctl)
  [no args / service call]   → service dispatch table (SCM launch) / systemd launch
*/

mod autoscale;
mod cron;
mod registry;
mod job;
mod ipc_server;
mod nrpt;
mod pipe_security;
mod policy;
mod reconciler;
mod secrets;
mod service;
mod state;
mod volume;
mod session;

#[cfg(unix)] mod socket_server;
#[cfg(unix)] mod socket_auth;
#[cfg(unix)] mod spawner;
#[cfg(unix)] mod cgroup;
#[cfg(unix)] mod systemd;
#[cfg(unix)] mod dns_config;
#[cfg(unix)] mod keyring;

#[cfg(not(windows))]
fn main() -> anyhow::Result<()> {
    init_logging();
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("");

    match cmd {
        "run" => {
            let rt = build_tokio_runtime()?;
            rt.block_on(run_core_unix())
        }
        "install" => service::install(),
        "uninstall" => service::uninstall(),
        "start" => service::start_service(),
        "stop" => service::stop_service(),
        "dns" => {
            let sub = args.get(2).map(|s| s.as_str()).unwrap_or("status");
            match sub {
                "enable" => {
                    nrpt::install(nrpt::VLN_DNS_ADDR)?;
                    println!(".vln DNS routing enabled.");
                    Ok(())
                }
                "disable" => {
                    nrpt::uninstall()?;
                    println!(".vln DNS routing disabled.");
                    Ok(())
                }
                "status" => {
                    if nrpt::is_installed() {
                        println!(".vln DNS routing is enabled ({})", nrpt::installed_addr().unwrap_or_default());
                    } else {
                        println!(".vln DNS routing is not configured.");
                    }
                    Ok(())
                }
                _ => {
                    eprintln!("Usage: veloce-core dns [enable|disable|status]");
                    Ok(())
                }
            }
        }
        // No argument = invoked by systemd
        _ => {
            let rt = build_tokio_runtime()?;
            rt.block_on(run_core_unix())
        }
    }
}

/// Unix entry point for running VeloceCore: sends READY=1 then runs the core loop.
#[cfg(not(windows))]
async fn run_core_unix() -> anyhow::Result<()> {
    // run_core() is the existing cross-platform async function.
    // Wire the Unix socket server instead of the named pipe server.
    run_core().await
}

#[cfg(not(windows))]
fn build_tokio_runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(Into::into)
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    // Initialise logging first (writes to %TEMP%\veloce-core.log in service mode)
    init_logging();

    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("");

    match cmd {
        "run" => {
            tracing::info!("VeloceCore starting in foreground mode");
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("veloce-worker")
                .build()?;
            rt.block_on(run_core())
        }
        "install"   => service::install(),
        "uninstall" => service::uninstall(),
        "start"     => service::start_service(),
        "stop"      => service::stop_service(),
        "nrpt" => {
            let sub = args.get(2).map(|s| s.as_str()).unwrap_or("status");
            match sub {
                "enable" => {
                    nrpt::install(nrpt::VLN_DNS_ADDR)?;
                    nrpt::restart_dnscache()
                        .unwrap_or_else(|e| tracing::warn!("Dnscache restart: {e}"));
                    println!("NRPT rule installed. *.vln → {}", nrpt::VLN_DNS_ADDR);
                    Ok(())
                }
                "disable" => {
                    nrpt::uninstall()?;
                    nrpt::restart_dnscache()
                        .unwrap_or_else(|e| tracing::warn!("Dnscache restart: {e}"));
                    println!("NRPT rule removed.");
                    Ok(())
                }
                _ => {
                    if nrpt::is_installed() {
                        let addr = nrpt::installed_addr().unwrap_or_else(|| "?".into());
                        println!("NRPT:    installed");
                        println!("DNS:     {addr}");
                    } else {
                        println!("NRPT:    not installed");
                        println!("Hint:    run `veloce-core nrpt enable` as Administrator");
                    }
                    Ok(())
                }
            }
        }
        _ => {
            // No recognised argument → assume SCM launched us as a service.
            service::dispatch()
        }
    }
}

/// Core async runtime — shared between service and foreground modes.
pub async fn run_core() -> anyhow::Result<()> {
    use state::CoreState;
    use std::sync::Arc;

    tracing::info!("Initialising VeloceCore v{}", env!("CARGO_PKG_VERSION"));

    // 1. Bring up the shared state (mmap registry + node table)
    let state = Arc::new(CoreState::new()?);
    tracing::info!("Registry mapped at {:?}", state.registry().path());

    // Wire the reconciler to CoreState (breaks the construction-time circular ref).
    state.reconciler().wire_state(state.clone());

    // 2. Start VeloceNet (DNS + routing + Ingress HTTP/HTTPS)
    let net_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = veloce_net::start_with_ingress_and_tls(
            net_state.net_registry().clone(),
            net_state.ingress_router().clone(),
            net_state.tls_manager().clone(),
        ).await {
            tracing::error!("VeloceNet error: {e}");
        }
    });

    // ── IPC server ──────────────────────────────────────────────────────────
    #[cfg(windows)]
    let ipc_handle = {
        let ipc_state = state.clone();
        tokio::spawn(async move { ipc_server::run(ipc_state).await })
    };
    #[cfg(unix)]
    let ipc_handle = {
        let ipc_state = state.clone();
        tokio::spawn(async move { socket_server::run(ipc_state).await })
    };

    // Suppress unused-variable warning when neither arm applies (shouldn't happen)
    let _ = ipc_handle;

    // 3b. Start the P2P mesh TCP server (if mesh is enabled).
    if let Some(mesh) = state.mesh.clone() {
        let listen_port = mesh.listen_port;
        tokio::spawn(async move {
            if let Err(e) = veloce_mesh::run_mesh_server(mesh, listen_port).await {
                tracing::error!("VeloceNet mesh server error: {e}");
            }
        });
    }

    // 4. Node health-check loop
    let hc_state = state.clone();
    tokio::spawn(async move {
        job::health_loop(hc_state).await;
    });

    // 5. Desired-state reconciler loop (v1.3)
    let rec = state.reconciler().clone();
    tokio::spawn(async move {
        rec.run().await;
    });

    tracing::info!("VeloceCore online — pipe: {}", veloce_ipc::PIPE_NAME);

    // Park the main task until shutdown signal
    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutdown signal received — stopping VeloceCore");
    state.shutdown();
    Ok(())
}

#[cfg(windows)]
fn init_logging() {
    use tracing_subscriber::fmt::writer::BoxMakeWriter;
    use std::fs::OpenOptions;

    let log_path = std::env::temp_dir().join("veloce-core.log");
    let file = OpenOptions::new()
        .create(true).append(true)
        .open(&log_path)
        .expect("cannot open log file");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "veloce_core=debug,veloce_net=debug,info".into())
        )
        .with_writer(BoxMakeWriter::new(move || {
            file.try_clone().expect("log file clone")
        }))
        .init();
}

#[cfg(not(windows))]
fn init_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_env("VELOCE_LOG")
                .add_directive(tracing::Level::INFO.into())
        )
        .with_writer(std::io::stderr)
        .init();
}

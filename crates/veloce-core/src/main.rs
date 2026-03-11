/*!
VeloceCore — entry point.

Run modes:
  veloce-core run            → foreground (dev)
  veloce-core install        → register as Windows service
  veloce-core uninstall      → remove Windows service
  veloce-core start          → start the service (SCM)
  veloce-core stop           → stop the service (SCM)
  [no args / service call]   → service dispatch table (SCM launch)
*/

mod registry;
mod job;
mod ipc_server;
mod service;
mod state;

#[cfg(not(windows))]
fn main() -> anyhow::Result<()> {
    eprintln!("VeloceCore only supports Windows targets. Use `cargo build --target x86_64-pc-windows-msvc`.");
    std::process::exit(1);
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
        "install" => service::install(),
        "uninstall" => service::uninstall(),
        "start" => service::start_service(),
        "stop"  => service::stop_service(),
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

    // 2. Start VeloceNet (DNS + routing)
    let net_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = veloce_net::start(net_state.net_registry()).await {
            tracing::error!("VeloceNet error: {e}");
        }
    });

    // 3. Start IPC named-pipe server
    let ipc_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = ipc_server::run(ipc_state).await {
            tracing::error!("IPC server error: {e}");
        }
    });

    // 4. Node health-check loop
    let hc_state = state.clone();
    tokio::spawn(async move {
        job::health_loop(hc_state).await;
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
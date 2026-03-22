//! Unix domain socket IPC server (Linux).
//!
//! Replaces the Windows named-pipe server (`ipc_server.rs`) for the Linux port.
//! Listens on `/run/veloce/core.sock` (privileged) or
//! `$XDG_RUNTIME_DIR/veloce/core.sock` (user mode).

#![cfg(unix)]

use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use tokio::net::UnixListener;

use crate::{
    session::ClientSession,
    socket_auth,
    state::CoreState,
};

pub async fn run(state: Arc<CoreState>) -> Result<()> {
    let socket_path = resolve_socket_path();

    // Ensure parent directory exists
    if let Some(parent) = Path::new(&socket_path).parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create socket dir {:?}", parent))?;
    }

    // Remove stale socket from a previous run
    if Path::new(&socket_path).exists() {
        std::fs::remove_file(&socket_path)
            .with_context(|| format!("remove stale socket {:?}", socket_path))?;
    }

    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("bind Unix socket {:?}", socket_path))?;

    // Restrict socket to owner-only (0o600) — only same-UID processes can connect.
    std::fs::set_permissions(
        &socket_path,
        std::fs::Permissions::from_mode(0o600),
    ).with_context(|| format!("chmod 600 {:?}", socket_path))?;

    tracing::info!("IPC server listening on {socket_path}");

    loop {
        if state.is_shutting_down() { break; }

        let (stream, _addr) = listener.accept().await
            .context("UnixListener::accept")?;

        // ── Peer auth: reject connections from other UIDs ─────────────────
        let (exe_path, client_pid) = match socket_auth::assert_client_is_owner(&stream) {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!("socket auth rejected connection: {e:#}");
                continue;
            }
        };

        // Server SID equivalent on Linux = server UID string
        let server_uid = socket_auth::server_uid_str();
        let client_state = state.clone();

        tokio::spawn(async move {
            let (read, write) = tokio::io::split(stream);
            let mut session = ClientSession::new(
                read, write, client_state, exe_path, client_pid, server_uid,
            );
            if let Err(e) = session.run().await {
                tracing::warn!("client session error: {e:#}");
            }
        });
    }

    // Clean up socket file on shutdown
    let _ = std::fs::remove_file(&socket_path);
    tracing::info!("IPC server stopped");
    Ok(())
}

fn resolve_socket_path() -> String {
    // Prefer privileged runtime directory (systemd creates /run/veloce/ via RuntimeDirectory=veloce)
    if Path::new("/run/veloce").exists() {
        return "/run/veloce/core.sock".to_string();
    }
    // Fall back to XDG_RUNTIME_DIR for user-mode
    veloce_ipc::socket_path_user().to_string_lossy().into_owned()
}

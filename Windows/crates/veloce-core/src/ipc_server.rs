/*!
Named-pipe IPC server (Windows).

Handles the Windows-specific accept loop and pipe security check.
The `ClientSession` protocol handler lives in `crate::session` and is
shared with the Unix socket server on Linux.
*/

#![cfg(windows)]

use anyhow::{Context, Result};
use std::sync::Arc;

use veloce_ipc::PIPE_NAME;

use crate::{
    pipe_security,
    session::ClientSession,
    state::CoreState,
};

// ── SERVER ENTRY POINT ────────────────────────────────────────────────────────

pub async fn run(state: Arc<CoreState>) -> Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    // Compute server's own user SID once; used to gate every incoming connection.
    let server_sid = pipe_security::server_user_sid()
        .context("resolve server user SID")?;
    tracing::debug!("pipe ACL: accepting connections from SID {server_sid}");

    let active_pipe = veloce_ipc::pipe_name();
    tracing::info!("IPC server listening on {active_pipe}");

    loop {
        if state.is_shutting_down() { break; }

        // Create a new pipe instance waiting for the next client
        let pipe = ServerOptions::new()
            .first_pipe_instance(false)
            .create(&active_pipe)
            .context("create named pipe instance")?;

        // Wait for a client to connect
        pipe.connect().await.context("pipe connect")?;

        // ── ACL gate: reject any process not running as the server's user ──
        let (exe_path, client_pid) = match pipe_security::assert_client_is_owner(&pipe, &server_sid) {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!("pipe ACL rejected connection: {e:#}");
                // Dropping `pipe` here disconnects the client immediately.
                continue;
            }
        };

        let client_state = state.clone();
        let sid_clone = server_sid.clone();
        tokio::spawn(async move {
            let (read, write) = tokio::io::split(pipe);
            let mut session = ClientSession::new(read, write, client_state, exe_path, client_pid, sid_clone);
            if let Err(e) = session.run().await {
                tracing::warn!("client session error: {e:#}");
            }
        });
    }

    tracing::info!("IPC server stopped");
    Ok(())
}

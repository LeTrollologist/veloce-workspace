//! Named-pipe IPC server (Windows).
//! On Linux, use `socket_server.rs` instead.

#![cfg(windows)]

use anyhow::{Context, Result};
use std::sync::Arc;

use veloce_ipc::PIPE_NAME;

use crate::{
    pipe_security,
    session::{ClientSession, spawn_event_forwarder, spawn_log_forwarder},
    state::CoreState,
};

pub async fn run(state: Arc<CoreState>) -> Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let server_sid = pipe_security::server_user_sid()
        .context("resolve server user SID")?;
    tracing::debug!("pipe ACL: accepting connections from SID {server_sid}");

    tracing::info!("IPC server listening on {PIPE_NAME}");

    loop {
        if state.is_shutting_down() { break; }

        let pipe = ServerOptions::new()
            .first_pipe_instance(false)
            .create(PIPE_NAME)
            .context("create named pipe instance")?;

        pipe.connect().await.context("pipe connect")?;

        let (exe_path, client_pid) = match pipe_security::assert_client_is_owner(&pipe, &server_sid) {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!("pipe ACL rejected connection: {e:#}");
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

/*!
`VeloceClient` — fully async, multiplexed IPC client.

One background reader task drains the pipe and routes replies by
`correlation_id` into per-request `oneshot` channels, so callers
can fire concurrent requests without locking.
*/

use anyhow::{bail, Context, Result};
use bytes::BytesMut;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use uuid::Uuid;

use veloce_ipc::{
    Codec,
    message::{
        Body, Capability, Envelope, Flags, NodeLimits, NodeSpawnedMsg,
        NetHostEntry, SpawnNodeMsg,
    },
    PIPE_NAME,
};

// ── REQUEST TABLE ─────────────────────────────────────────────────────────────

type PendingMap = Arc<Mutex<HashMap<Uuid, oneshot::Sender<Body>>>>;

// ── CLIENT ────────────────────────────────────────────────────────────────────

pub struct VeloceClient {
    /// Write half — protected so concurrent callers can send without races.
    writer:  Arc<AsyncMutex<tokio::io::WriteHalf<tokio::net::windows::named_pipe::NamedPipeClient>>>,
    pending: PendingMap,
    /// Our assigned client ID (set after handshake).
    pub client_id: Uuid,
}

impl VeloceClient {
    /// Connect to VeloceCore and perform the handshake.
    pub async fn connect(
        app_name:     &str,
        sdk_version:  &str,
        capabilities: Vec<Capability>,
    ) -> Result<Self> {
        #[cfg(not(windows))]
        {
            bail!("VeloceClient is only supported on Windows");
        }

        #[cfg(windows)]
        {
            use tokio::net::windows::named_pipe::ClientOptions;

            // Retry loop — Core might not be up yet
            let pipe = retry_connect(PIPE_NAME, Duration::from_secs(10)).await?;
            let (reader, writer_half) = tokio::io::split(pipe);

            let pending:  PendingMap = Arc::new(Mutex::new(HashMap::new()));
            let writer    = Arc::new(AsyncMutex::new(writer_half));

            // Spawn background reader
            let bg_pending = pending.clone();
            tokio::spawn(async move {
                if let Err(e) = reader_loop(reader, bg_pending).await {
                    tracing::warn!("VeloceClient reader error: {e:#}");
                }
            });

            let mut client = Self { writer, pending, client_id: Uuid::nil() };

            // Handshake
            let ack = client.request(Body::Handshake(
                veloce_ipc::message::HandshakeMsg {
                    app_name:    app_name.into(),
                    sdk_version: sdk_version.into(),
                    capabilities,
                    psk_hash: None,
                }
            )).await?;

            match ack {
                Body::HandshakeAck(a) => {
                    client.client_id = a.client_id;
                    tracing::info!(?client.client_id, "VeloceCore handshake OK");
                }
                Body::Error(e) => bail!("handshake rejected: {}", e.message),
                other          => bail!("unexpected handshake reply: {:?}", other.msg_type()),
            }

            Ok(client)
        }
    }

    // ── Node management ───────────────────────────────────────────────────────

    pub async fn spawn_node(
        &mut self,
        app_name:   &str,
        executable: &str,
        args:       &[&str],
    ) -> Result<NodeSpawnedMsg> {
        self.spawn_node_with(SpawnNodeMsg {
            app_name:   app_name.into(),
            executable: executable.into(),
            args:       args.iter().map(|s| s.to_string()).collect(),
            env:        vec![],
            limits:     None,
            auto_kill:  true,
        }).await
    }

    pub async fn spawn_node_with(&mut self, msg: SpawnNodeMsg) -> Result<NodeSpawnedMsg> {
        match self.request(Body::SpawnNode(msg)).await? {
            Body::NodeSpawned(s) => Ok(s),
            Body::Error(e)       => bail!("spawn failed: {}", e.message),
            other                => bail!("unexpected reply: {:?}", other.msg_type()),
        }
    }

    pub async fn kill_node(&mut self, node_id: Uuid) -> Result<u32> {
        match self.request(Body::KillNode(veloce_ipc::message::KillNodeMsg {
            node_id, exit_code: None,
        })).await? {
            Body::NodeKilled(k) => Ok(k.exit_code),
            Body::Error(e)      => bail!("kill failed: {}", e.message),
            other               => bail!("unexpected reply: {:?}", other.msg_type()),
        }
    }

    pub async fn list_nodes(&mut self) -> Result<Vec<veloce_ipc::message::NodeInfo>> {
        match self.request(Body::QueryNodes).await? {
            Body::NodeList(l) => Ok(l.nodes),
            Body::Error(e)    => bail!("query failed: {}", e.message),
            other             => bail!("unexpected reply: {:?}", other.msg_type()),
        }
    }

    // ── Registry ──────────────────────────────────────────────────────────────

    pub async fn registry_get(&mut self, key: &str) -> Result<Option<Vec<u8>>> {
        match self.request(Body::RegistryGet { key: key.into() }).await? {
            Body::RegistryValue(v) => Ok(v.value),
            Body::Error(e)         => bail!("{}", e.message),
            other                  => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    pub async fn registry_set(&mut self, key: &str, value: Vec<u8>) -> Result<()> {
        match self.request(Body::RegistrySet { key: key.into(), value }).await? {
            Body::RegistryAck { .. } => Ok(()),
            Body::Error(e)           => bail!("{}", e.message),
            other                    => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    // ── VeloceNet ─────────────────────────────────────────────────────────────

    pub async fn register_host(
        &mut self,
        hostname:   &str,
        node_id:    Uuid,
        local_port: u16,
        ttl_secs:   u64,
    ) -> Result<()> {
        match self.request(Body::NetRegisterHost(
            veloce_ipc::message::NetRegisterHostMsg {
                hostname:   hostname.into(),
                node_id,
                local_port,
                ttl_secs,
            }
        )).await? {
            Body::NetHostRegistered { .. } => Ok(()),
            Body::Error(e)                  => bail!("{}", e.message),
            other                           => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    pub async fn resolve_host(&mut self, hostname: &str) -> Result<Option<String>> {
        match self.request(Body::NetResolve { hostname: hostname.into() }).await? {
            Body::NetResolveResult(r) => Ok(r.address),
            Body::Error(e)            => bail!("{}", e.message),
            other                     => bail!("unexpected: {:?}", other.msg_type()),
        }
    }

    // ── Ping ──────────────────────────────────────────────────────────────────

    pub async fn ping(&mut self) -> Result<()> {
        match self.request(Body::Ping).await? {
            Body::Pong => Ok(()),
            other      => bail!("unexpected pong: {:?}", other.msg_type()),
        }
    }

    // ── Core send / receive ───────────────────────────────────────────────────

    /// Send a request and wait for a correlated reply (5-second timeout).
    async fn request(&self, body: Body) -> Result<Body> {
        let env    = Envelope::new(body);
        let cid    = env.correlation_id;
        let (tx, rx) = oneshot::channel();

        self.pending.lock().insert(cid, tx);

        let frame = Codec::encode(&env, Flags::EXPECTS_ACK)
            .context("encode request")?;

        self.writer.lock().await
            .write_all(&frame).await
            .context("write to pipe")?;

        let reply = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .context("response timeout")?
            .context("sender dropped")?;

        Ok(reply)
    }
}

// ── BACKGROUND READER ─────────────────────────────────────────────────────────

#[cfg(windows)]
async fn reader_loop<R>(mut reader: R, pending: PendingMap) -> Result<()>
where
    R: AsyncReadExt + Unpin,
{
    let mut buf = [0u8; 8192];
    let mut acc = BytesMut::with_capacity(32 * 1024);

    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 { break; }
        acc.extend_from_slice(&buf[..n]);

        loop {
            match Codec::decode_safe(&mut acc) {
                Ok(Some((_hdr, env))) => {
                    if let Some(tx) = pending.lock().remove(&env.correlation_id) {
                        let _ = tx.send(env.body);
                    } else {
                        // Push (unsolicited) — log for now
                        tracing::debug!("unsolicited push: {:?}", env.msg_type());
                    }
                }
                Ok(None)  => break,
                Err(e) => {
                    tracing::warn!("decode error in reader loop: {e:#}");
                    acc.clear();
                    break;
                }
            }
        }
    }

    tracing::info!("VeloceClient reader loop exited");
    Ok(())
}

// ── CONNECT WITH RETRY ────────────────────────────────────────────────────────

#[cfg(windows)]
async fn retry_connect(
    pipe: &str,
    timeout: Duration,
) -> Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match ClientOptions::new().open(pipe) {
            Ok(c)  => return Ok(c),
            Err(e) if e.raw_os_error() == Some(231 /* ERROR_PIPE_BUSY */) => {
                // Pipe exists but no available instances; wait
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) if e.raw_os_error() == Some(2 /* ERROR_FILE_NOT_FOUND */) => {
                // Core not up yet
                if tokio::time::Instant::now() >= deadline {
                    bail!("VeloceCore pipe not found after {}s — is the service running?", timeout.as_secs());
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(e) => return Err(e).context("open named pipe"),
        }
    }
}
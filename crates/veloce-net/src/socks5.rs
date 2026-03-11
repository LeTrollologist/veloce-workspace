/*!
SOCKS5 proxy server for VeloceNet.

Implements RFC 1928 (SOCKS5) — `CONNECT` command only.

Routing logic:
- If target hostname resolves in `NetRegistry` → connect to `127.0.0.1:<local_port>`
- Otherwise → connect to the real target (passthrough)

Sideloaded apps set `HTTP_PROXY=socks5://127.0.0.1:1055` (or equivalent
per-language settings) to get automatic `*.vln` routing.
*/

use anyhow::{bail, Context, Result};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::registry::NetRegistry;

// ── SOCKS5 CONSTANTS ──────────────────────────────────────────────────────────

const VERSION:       u8 = 0x05;
const AUTH_NONE:     u8 = 0x00;
const AUTH_NO_ACC:   u8 = 0xFF;
const CMD_CONNECT:   u8 = 0x01;
const ATYP_IPV4:     u8 = 0x01;
const ATYP_DOMAIN:   u8 = 0x03;
const ATYP_IPV6:     u8 = 0x04;
const REP_SUCCESS:   u8 = 0x00;
const REP_FAIL:      u8 = 0x01;
const REP_NO_ALLOW:  u8 = 0x02;
const REP_UNREACHABLE: u8 = 0x04;
const REP_CMD_UNSUP: u8 = 0x07;
const REP_ATYP_UNSUP: u8 = 0x08;

// ── SERVER ────────────────────────────────────────────────────────────────────

pub async fn serve(registry: Arc<NetRegistry>, port: u16) -> Result<()> {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    tracing::info!("SOCKS5 server listening on 127.0.0.1:{port}");

    loop {
        let (stream, peer) = listener.accept().await?;
        let reg = registry.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, peer, reg).await {
                tracing::debug!("SOCKS5 error from {peer}: {e:#}");
            }
        });
    }
}

async fn handle_connection(
    mut stream:   TcpStream,
    _peer:        SocketAddr,
    registry:     Arc<NetRegistry>,
) -> Result<()> {
    // ── Step 1: Auth negotiation ──────────────────────────────────────────────
    let ver = stream.read_u8().await?;
    if ver != VERSION {
        bail!("unsupported SOCKS version: {ver}");
    }
    let nmethods = stream.read_u8().await? as usize;
    let mut methods = vec![0u8; nmethods];
    stream.read_exact(&mut methods).await?;

    if !methods.contains(&AUTH_NONE) {
        // We only support no-auth for local connections
        stream.write_all(&[VERSION, AUTH_NO_ACC]).await?;
        bail!("client doesn't support no-auth");
    }
    stream.write_all(&[VERSION, AUTH_NONE]).await?;

    // ── Step 2: Request ──────────────────────────────────────────────────────
    let ver = stream.read_u8().await?;
    if ver != VERSION { bail!("bad version in request"); }

    let cmd = stream.read_u8().await?;
    let _rsv = stream.read_u8().await?; // reserved

    let atyp = stream.read_u8().await?;
    let (target_host, target_port) = read_address(&mut stream, atyp).await?;

    if cmd != CMD_CONNECT {
        send_reply(&mut stream, REP_CMD_UNSUP, Ipv4Addr::UNSPECIFIED, 0).await?;
        bail!("unsupported SOCKS command: {cmd}");
    }

    // ── Step 3: Route ─────────────────────────────────────────────────────────
    let real_addr = if registry.is_vln(&target_host) {
        match registry.resolve(&target_host) {
            Some(rec) => {
                tracing::debug!(hostname = %target_host, port = rec.local_port, "SOCKS5: vln route");
                format!("127.0.0.1:{}", rec.local_port)
            }
            None => {
                send_reply(&mut stream, REP_UNREACHABLE, Ipv4Addr::UNSPECIFIED, 0).await?;
                bail!("vln hostname not registered: {target_host}");
            }
        }
    } else {
        format!("{target_host}:{target_port}")
    };

    // ── Step 4: Connect to target ─────────────────────────────────────────────
    match TcpStream::connect(&real_addr).await {
        Ok(mut target) => {
            let local = target.local_addr()
                .unwrap_or_else(|_| SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)));
            let (bind_ip, bind_port) = match local {
                SocketAddr::V4(a) => (*a.ip(), a.port()),
                SocketAddr::V6(_) => (Ipv4Addr::UNSPECIFIED, 0),
            };
            send_reply(&mut stream, REP_SUCCESS, bind_ip, bind_port).await?;

            // Bidirectional copy
            let (mut cr, mut cw) = stream.split();
            let (mut tr, mut tw) = target.split();
            tokio::select! {
                r = io::copy(&mut cr, &mut tw) => { r?; }
                r = io::copy(&mut tr, &mut cw) => { r?; }
            }
        }
        Err(e) => {
            tracing::warn!("SOCKS5: cannot connect to {real_addr}: {e}");
            send_reply(&mut stream, REP_FAIL, Ipv4Addr::UNSPECIFIED, 0).await?;
        }
    }

    Ok(())
}

async fn read_address(stream: &mut TcpStream, atyp: u8) -> Result<(String, u16)> {
    let host = match atyp {
        ATYP_IPV4 => {
            let mut ip = [0u8; 4];
            stream.read_exact(&mut ip).await?;
            Ipv4Addr::from(ip).to_string()
        }
        ATYP_DOMAIN => {
            let len = stream.read_u8().await? as usize;
            let mut name = vec![0u8; len];
            stream.read_exact(&mut name).await?;
            String::from_utf8(name).context("invalid domain name")?
        }
        ATYP_IPV6 => {
            let mut ip = [0u8; 16];
            stream.read_exact(&mut ip).await?;
            std::net::Ipv6Addr::from(ip).to_string()
        }
        _ => {
            bail!("unsupported address type: {atyp}");
        }
    };
    let port = stream.read_u16().await?;
    Ok((host, port))
}

async fn send_reply(
    stream:    &mut TcpStream,
    rep:       u8,
    bind_addr: Ipv4Addr,
    bind_port: u16,
) -> Result<()> {
    let ip = bind_addr.octets();
    let reply = [
        VERSION, rep, 0x00,  // VER, REP, RSV
        ATYP_IPV4,           // ATYP
        ip[0], ip[1], ip[2], ip[3],
        (bind_port >> 8) as u8,
        (bind_port & 0xFF) as u8,
    ];
    stream.write_all(&reply).await?;
    Ok(())
}

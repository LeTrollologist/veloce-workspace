//! Minimal RFC 5389 / 8489 STUN binding-request client.
//!
//! Sends a Binding Request to a public STUN server and returns the
//! XOR-MAPPED-ADDRESS — the external (NAT-translated) IP address seen by the
//! server.  Only the IP is meaningful for our use-case; we always advertise
//! the mesh server port (7474) in the join code rather than the ephemeral UDP
//! source port that the STUN server observed.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use rand::RngCore;
use tokio::{
    net::UdpSocket,
    time::{timeout, Duration},
};

/// Built-in fallback list used when the caller supplies no servers.
const DEFAULT_STUN_SERVERS: &[&str] = &[
    "stun.l.google.com:19302",
    "stun.cloudflare.com:3478",
];

/// The STUN magic cookie value defined in RFC 5389 §6.
const MAGIC_COOKIE: u32 = 0x2112_A442;

/// STUN Binding Request message type.
const MSG_BINDING_REQUEST: u16 = 0x0001;
/// STUN Binding Success Response message type.
const MSG_BINDING_RESPONSE: u16 = 0x0101;

/// XOR-MAPPED-ADDRESS attribute type (RFC 5389 §15.2).
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

// ── Public API ────────────────────────────────────────────────────────────────

/// Discover the machine's external IP address by querying STUN servers.
///
/// `servers` is the operator-configured list from `veloce-policy.toml`.  If
/// empty, falls back to the built-in default list.  Tries each server in order;
/// returns `None` if all fail (e.g. offline, firewall blocks UDP, or timeout).
pub async fn discover_external_ip(servers: &[String]) -> Option<IpAddr> {
    let list: Vec<&str> = if servers.is_empty() {
        DEFAULT_STUN_SERVERS.to_vec()
    } else {
        servers.iter().map(String::as_str).collect()
    };

    for server in list {
        match try_stun(server).await {
            Ok(ip) => {
                tracing::debug!("STUN discovery via {server}: external IP = {ip}");
                return Some(ip);
            }
            Err(e) => {
                tracing::debug!("STUN server {server} failed: {e}");
            }
        }
    }
    None
}

// ── Internal helpers ──────────────────────────────────────────────────────────

async fn try_stun(server: &str) -> anyhow::Result<IpAddr> {
    // Resolve server hostname — prefer IPv4 so XOR decoding stays simple.
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host(server).await?.collect();
    let server_addr = addrs
        .into_iter()
        .find(|a| a.is_ipv4())
        .ok_or_else(|| anyhow::anyhow!("no IPv4 address resolved for {server}"))?;

    // Bind an ephemeral UDP socket.
    let sock = UdpSocket::bind("0.0.0.0:0").await?;

    // Build a 20-byte STUN Binding Request with no attributes.
    let mut req = [0u8; 20];
    req[0..2].copy_from_slice(&MSG_BINDING_REQUEST.to_be_bytes());
    // bytes 2–3: message length (0 — no attributes)
    req[4..8].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
    // bytes 8–19: 96-bit transaction ID (random)
    rand::rngs::OsRng.fill_bytes(&mut req[8..20]);
    let txn_id: [u8; 12] = req[8..20].try_into().unwrap();

    sock.send_to(&req, server_addr).await?;

    // Wait up to 3 s for the Binding Response.
    let mut buf = [0u8; 512];
    let (n, from) = timeout(Duration::from_secs(3), sock.recv_from(&mut buf)).await??;

    // N8: Reject responses from unexpected sources.
    anyhow::ensure!(
        from == server_addr,
        "STUN response from unexpected source {from} (expected {server_addr})"
    );

    parse_xor_mapped_address(&buf[..n], &txn_id)
}

/// Walk the STUN message attributes and extract the XOR-MAPPED-ADDRESS.
fn parse_xor_mapped_address(data: &[u8], txn_id: &[u8; 12]) -> anyhow::Result<IpAddr> {
    anyhow::ensure!(data.len() >= 20, "STUN response too short ({} bytes)", data.len());

    let msg_type = u16::from_be_bytes([data[0], data[1]]);
    anyhow::ensure!(
        msg_type == MSG_BINDING_RESPONSE,
        "unexpected STUN message type: 0x{msg_type:04X}"
    );

    // N9: Validate the RFC 5389 magic cookie (bytes 4–7 must equal 0x2112A442).
    let magic = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    anyhow::ensure!(
        magic == MAGIC_COOKIE,
        "STUN magic cookie mismatch: 0x{magic:08X} (expected 0x{MAGIC_COOKIE:08X})"
    );

    // Verify transaction ID (bytes 8–19).
    anyhow::ensure!(&data[8..20] == txn_id, "STUN transaction ID mismatch");

    let msg_len = u16::from_be_bytes([data[2], data[3]]) as usize;
    let body_end = 20 + msg_len;
    anyhow::ensure!(data.len() >= body_end, "STUN response truncated");

    let mut pos = 20usize;
    while pos + 4 <= body_end {
        let attr_type = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let attr_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        if attr_type == ATTR_XOR_MAPPED_ADDRESS {
            // Layout: [reserved: 1][family: 1][xport: 2][xaddr: 4 or 16]
            anyhow::ensure!(pos + 4 <= data.len(), "XOR-MAPPED-ADDRESS truncated");
            let family = data[pos + 1];

            match family {
                0x01 => {
                    // IPv4: XOR with MAGIC_COOKIE
                    anyhow::ensure!(pos + 8 <= data.len(), "IPv4 XOR-MAPPED-ADDRESS too short");
                    let xaddr = u32::from_be_bytes(data[pos + 4..pos + 8].try_into()?);
                    let addr = Ipv4Addr::from(xaddr ^ MAGIC_COOKIE);
                    return Ok(IpAddr::V4(addr));
                }
                0x02 => {
                    // IPv6: XOR first 4 bytes with MAGIC_COOKIE, remaining 12 with txn_id
                    anyhow::ensure!(pos + 20 <= data.len(), "IPv6 XOR-MAPPED-ADDRESS too short");
                    let mut octets = [0u8; 16];
                    octets.copy_from_slice(&data[pos + 4..pos + 20]);
                    let magic_bytes = MAGIC_COOKIE.to_be_bytes();
                    for i in 0..4 {
                        octets[i] ^= magic_bytes[i];
                    }
                    for i in 0..12 {
                        octets[4 + i] ^= txn_id[i];
                    }
                    return Ok(IpAddr::V6(Ipv6Addr::from(octets)));
                }
                _ => anyhow::bail!("unknown address family in XOR-MAPPED-ADDRESS: {family}"),
            }
        }

        // Attributes are padded to 4-byte boundaries.
        pos += (attr_len + 3) & !3;
    }

    anyhow::bail!("XOR-MAPPED-ADDRESS attribute not found in STUN response")
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ipv4_xor_mapped_address() {
        // Construct a minimal synthetic STUN success response with one
        // XOR-MAPPED-ADDRESS attribute for 1.2.3.4.
        let txn_id: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let xip = u32::from_be_bytes([1, 2, 3, 4]) ^ MAGIC_COOKIE;

        let mut msg = vec![0u8; 32];
        // Header
        msg[0..2].copy_from_slice(&MSG_BINDING_RESPONSE.to_be_bytes());
        msg[2..4].copy_from_slice(&12u16.to_be_bytes()); // attr section = 12 bytes
        msg[4..8].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
        msg[8..20].copy_from_slice(&txn_id);
        // XOR-MAPPED-ADDRESS attribute
        msg[20..22].copy_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
        msg[22..24].copy_from_slice(&8u16.to_be_bytes()); // length = 8
        msg[24] = 0; // reserved
        msg[25] = 0x01; // IPv4
        msg[26..28].fill(0); // xport (irrelevant for this test)
        msg[28..32].copy_from_slice(&xip.to_be_bytes());

        let ip = parse_xor_mapped_address(&msg, &txn_id).unwrap();
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)));
    }
}

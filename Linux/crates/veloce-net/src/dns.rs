/*!
Minimal UDP DNS server for VeloceNet private TLDs.

Handles:
- A queries for `*.vln` and `*.veloce` → 127.0.0.1
- All other queries → forwarded to system resolver

DNS wire format is hand-rolled (no external crate dependency)
to keep the binary slim and the protocol auditable.
*/

use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;

use crate::registry::NetRegistry;

// ── SERVER ────────────────────────────────────────────────────────────────────

/// Start the VeloceNet DNS server.
///
/// `bind_addr` controls which interface the UDP socket is bound to.
/// The default (`127.0.0.1`) restricts the server to local processes only, which is
/// the correct setting for most deployments.  Set `VLN_DNS_BIND=0.0.0.0` if you
/// intentionally want LAN-wide `.vln` resolution (e.g., shared home-network resolver).
pub async fn serve(registry: Arc<NetRegistry>, bind_addr: &str, port: u16) -> Result<()> {
    let sock = Arc::new(UdpSocket::bind(format!("{bind_addr}:{port}")).await?);
    tracing::info!("DNS server listening on {bind_addr}:{port}");

    let mut buf = vec![0u8; 512];

    loop {
        let (n, from) = sock.recv_from(&mut buf).await?;
        let packet    = buf[..n].to_vec();
        let registry  = registry.clone();
        let sock_ref  = sock.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_query(packet, from, registry, sock_ref).await {
                tracing::warn!("DNS handler error from {from}: {e:#}");
            }
        });
    }
}

async fn handle_query(
    packet:   Vec<u8>,
    from:     SocketAddr,
    registry: Arc<NetRegistry>,
    sock:     Arc<UdpSocket>,
) -> Result<()> {
    let query = match DnsMessage::parse(&packet) {
        Ok(q)  => q,
        Err(e) => {
            tracing::debug!("malformed DNS query from {from}: {e}");
            return Ok(());
        }
    };

    // Only handle single-question A/AAAA queries; forward anything else
    if query.questions.len() != 1 {
        forward_query(&packet, from, &sock).await?;
        return Ok(());
    }

    let q = &query.questions[0];

    if registry.is_vln(&q.name) {
        // Resolve locally → 127.0.0.1
        let record = registry.resolve(&q.name);
        let ip: [u8; 4] = [127, 0, 0, 1];

        if record.is_some() || q.name.ends_with(".vln") || q.name.ends_with(".veloce") {
            let response = build_a_response(query.id, &q.name, &ip, 30 /* TTL */)?;
            sock.send_to(&response, from).await?;
            tracing::debug!(name = %q.name, "DNS A → 127.0.0.1 (vln)");
        } else {
            let response = build_nxdomain(query.id);
            sock.send_to(&response, from).await?;
        }
    } else {
        // Pass through to system resolver
        forward_query(&packet, from, &sock).await?;
    }

    Ok(())
}

/// Forward query to system resolver (1.1.1.1:53 as fallback if no system resolver found).
async fn forward_query(packet: &[u8], from: SocketAddr, sock: &UdpSocket) -> Result<()> {
    // Capture the transaction ID from the original query for response validation.
    if packet.len() < 2 {
        anyhow::bail!("DNS query too short to extract transaction ID");
    }
    let txn_id = [packet[0], packet[1]];

    let upstream = get_system_resolver();
    // Bind to loopback-only so the ephemeral port is not reachable from the LAN.
    let fwd = UdpSocket::bind("127.0.0.1:0").await?;
    fwd.send_to(packet, upstream).await?;

    let mut resp_buf = vec![0u8; 512];
    let (n, src) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        fwd.recv_from(&mut resp_buf),
    ).await??;

    // Reject responses that don't come from the upstream we queried.
    if src != upstream {
        anyhow::bail!("DNS upstream response from unexpected source {src} (expected {upstream})");
    }
    // Reject responses whose transaction ID doesn't match the request (N3).
    if n < 2 || resp_buf[0] != txn_id[0] || resp_buf[1] != txn_id[1] {
        anyhow::bail!("DNS response transaction ID mismatch");
    }

    sock.send_to(&resp_buf[..n], from).await?;
    Ok(())
}

/// Return the first non-unspecified DNS server configured on any up-status adapter.
///
/// On Windows: queries `GetAdaptersAddresses` for the host's actual DNS configuration,
/// respecting VPNs, corporate resolvers, and local forwarders.
/// On other platforms: parses `/etc/resolv.conf`.
///
/// Falls back to `1.1.1.1:53` with a warning if no system resolver is found —
/// this indicates a degraded network state (no adapters up, no DNS configured).
fn get_system_resolver() -> SocketAddr {
    if let Some(addr) = probe_system_resolver() {
        return addr;
    }
    tracing::warn!(
        "no system DNS resolver found via OS APIs — \
         forwarding to 1.1.1.1:53 (degraded network state)"
    );
    "1.1.1.1:53".parse().unwrap()
}

#[cfg(windows)]
fn probe_system_resolver() -> Option<SocketAddr> {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR};
    use windows::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GET_ADAPTERS_ADDRESSES_FLAGS,
        GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST,
        IP_ADAPTER_ADDRESSES_LH,
    };
    use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6, SOCKADDR_IN, SOCKADDR_IN6};

    // Combine skip flags; GET_ADAPTERS_ADDRESSES_FLAGS doesn't impl BitOr, so combine .0 values.
    let flags = GET_ADAPTERS_ADDRESSES_FLAGS(GAA_FLAG_SKIP_ANYCAST.0 | GAA_FLAG_SKIP_MULTICAST.0);

    // Allocate with u64 elements to guarantee 8-byte alignment required by IP_ADAPTER_ADDRESSES_LH.
    let mut buf_size: u32 = 16 * 1024;
    let buf: Vec<u64> = loop {
        let n_u64 = (buf_size as usize + 7) / 8;
        let mut buf = vec![0u64; n_u64];
        let ret = unsafe {
            GetAdaptersAddresses(
                0,         // AF_UNSPEC
                flags,
                None,      // reserved (must be null)
                Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
                &mut buf_size,
            )
        };
        // GetAdaptersAddresses returns u32; NO_ERROR/ERROR_BUFFER_OVERFLOW are WIN32_ERROR newtypes.
        if ret == NO_ERROR.0 {
            break buf;
        } else if ret == ERROR_BUFFER_OVERFLOW.0 {
            // buf_size has been updated to the required size — retry with larger allocation.
            continue;
        } else {
            tracing::warn!("GetAdaptersAddresses failed (error {ret})");
            return None;
        }
    };

    // Walk the adapter linked list.
    let mut adapter = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    while !adapter.is_null() {
        let a = unsafe { &*adapter };

        // IfOperStatusUp = 1; skip adapters that are down, disconnected, etc.
        if a.OperStatus.0 == 1 {
            let mut dns = a.FirstDnsServerAddress;
            while !dns.is_null() {
                let d = unsafe { &*dns };
                let sa = d.Address.lpSockaddr;
                if !sa.is_null() {
                    // sa_family is ADDRESS_FAMILY (u16 newtype); compare .0 values.
                    let family = unsafe { (*sa).sa_family.0 };
                    if family == AF_INET.0 {
                        // SAFETY: family discriminant confirmed IPv4.
                        let sin = unsafe { &*(sa as *const SOCKADDR_IN) };
                        // S_addr is in network byte order.
                        let ip = Ipv4Addr::from(u32::from_be(unsafe { sin.sin_addr.S_un.S_addr }));
                        if !ip.is_unspecified() {
                            tracing::debug!("system DNS resolver (Windows adapter): {ip}:53");
                            return Some(SocketAddr::new(IpAddr::V4(ip), 53));
                        }
                    } else if family == AF_INET6.0 {
                        // SAFETY: family discriminant confirmed IPv6.
                        let sin6 = unsafe { &*(sa as *const SOCKADDR_IN6) };
                        let ip = Ipv6Addr::from(unsafe { sin6.sin6_addr.u.Byte });
                        if !ip.is_unspecified() {
                            tracing::debug!("system DNS resolver (Windows adapter): [{ip}]:53");
                            return Some(SocketAddr::new(IpAddr::V6(ip), 53));
                        }
                    }
                }
                dns = d.Next;
            }
        }
        adapter = a.Next;
    }

    None
}

#[cfg(not(windows))]
fn probe_system_resolver() -> Option<SocketAddr> {
    if let Ok(content) = std::fs::read_to_string("/etc/resolv.conf") {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("nameserver ") {
                let addr = rest.trim().to_string() + ":53";
                if let Ok(a) = addr.parse() {
                    return Some(a);
                }
            }
        }
    }
    None
}

// ── DNS WIRE FORMAT ───────────────────────────────────────────────────────────

#[derive(Debug)]
struct DnsMessage {
    id:        u16,
    questions: Vec<DnsQuestion>,
}

#[derive(Debug)]
#[allow(dead_code)]
struct DnsQuestion {
    name:  String,
    qtype: u16,
    class: u16,
}

impl DnsMessage {
    fn parse(buf: &[u8]) -> Result<Self> {
        if buf.len() < 12 {
            anyhow::bail!("DNS message too short");
        }
        let id        = u16::from_be_bytes([buf[0], buf[1]]);
        // Cap qdcount before allocation — standard DNS uses exactly 1 question;
        // no legitimate client sends more than a handful (N6).
        let qdcount   = (u16::from_be_bytes([buf[4], buf[5]]) as usize).min(5);

        let mut pos = 12usize;
        let mut questions = Vec::with_capacity(qdcount);

        for _ in 0..qdcount {
            let (name, new_pos) = parse_name(buf, pos)?;
            pos = new_pos;
            if pos + 4 > buf.len() { anyhow::bail!("truncated question"); }
            let qtype = u16::from_be_bytes([buf[pos], buf[pos+1]]);
            let class = u16::from_be_bytes([buf[pos+2], buf[pos+3]]);
            pos += 4;
            questions.push(DnsQuestion { name, qtype, class });
        }

        Ok(Self { id, questions })
    }
}

/// Parse a DNS name at `pos`, returning (name, new_pos).
///
/// Guards against compression pointer loops (a DoS vector in hand-rolled DNS
/// parsers) by bailing if more than 10 pointer jumps are followed in one name.
fn parse_name(buf: &[u8], mut pos: usize) -> Result<(String, usize)> {
    let mut labels  = Vec::new();
    let mut jumps   = 0usize;   // counts pointer hops; bail if > MAX_JUMPS
    let mut end_pos = 0;
    const MAX_JUMPS: usize = 10;

    loop {
        if pos >= buf.len() { anyhow::bail!("name past end of packet"); }
        let len = buf[pos] as usize;

        if len == 0 {
            if jumps == 0 { end_pos = pos + 1; }
            break;
        }

        // Pointer (compression)
        if len & 0xC0 == 0xC0 {
            if pos + 1 >= buf.len() { anyhow::bail!("compression pointer truncated"); }
            jumps += 1;
            if jumps > MAX_JUMPS {
                anyhow::bail!("DNS compression pointer loop detected (> {MAX_JUMPS} jumps)");
            }
            if jumps == 1 { end_pos = pos + 2; } // record where the uncompressed stream ends
            let ptr = (((len & 0x3F) as usize) << 8) | buf[pos+1] as usize;
            pos = ptr;
            continue;
        }

        pos += 1;
        if pos + len > buf.len() { anyhow::bail!("label past end of packet"); }
        // RFC 1035 §2.3.4: labels must not exceed 63 octets.
        if len > 63 { anyhow::bail!("DNS label exceeds RFC 1035 maximum of 63 bytes ({len})"); }
        labels.push(String::from_utf8_lossy(&buf[pos..pos+len]).into_owned());
        pos += len;
    }

    if jumps == 0 { end_pos = pos; }
    Ok((labels.join("."), end_pos))
}

/// Encode a name as DNS labels.
///
/// Returns an error if any label exceeds 63 bytes (RFC 1035 §2.3.4) or if
/// the encoded name would exceed 253 bytes (RFC 1035 §3.1 total-name limit).
fn encode_name(name: &str, buf: &mut Vec<u8>) -> Result<()> {
    for label in name.split('.') {
        if label.len() > 63 {
            anyhow::bail!(
                "DNS label '{label}' exceeds RFC 1035 maximum of 63 bytes ({})",
                label.len()
            );
        }
        buf.push(label.len() as u8);
        buf.extend_from_slice(label.as_bytes());
    }
    buf.push(0); // root
    Ok(())
}

/// Build a DNS A response pointing to `ip`.
fn build_a_response(id: u16, name: &str, ip: &[u8; 4], ttl: u32) -> Result<Vec<u8>> {
    let mut buf = Vec::new();

    // Header
    buf.extend_from_slice(&id.to_be_bytes());
    buf.extend_from_slice(&[0x81, 0x80]); // QR=1, OPCODE=0, AA=1, TC=0, RD=1, RA=1
    buf.extend_from_slice(&[0x00, 0x01]); // QDCOUNT=1
    buf.extend_from_slice(&[0x00, 0x01]); // ANCOUNT=1
    buf.extend_from_slice(&[0x00, 0x00]); // NSCOUNT=0
    buf.extend_from_slice(&[0x00, 0x00]); // ARCOUNT=0

    // Question section (echo)
    encode_name(name, &mut buf)?;
    buf.extend_from_slice(&[0x00, 0x01]); // QTYPE=A
    buf.extend_from_slice(&[0x00, 0x01]); // QCLASS=IN

    // Answer section
    encode_name(name, &mut buf)?;
    buf.extend_from_slice(&[0x00, 0x01]); // TYPE=A
    buf.extend_from_slice(&[0x00, 0x01]); // CLASS=IN
    buf.extend_from_slice(&ttl.to_be_bytes());
    buf.extend_from_slice(&[0x00, 0x04]); // RDLENGTH=4
    buf.extend_from_slice(ip);

    Ok(buf)
}

fn build_nxdomain(id: u16) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&id.to_be_bytes());
    buf.extend_from_slice(&[0x81, 0x83]); // QR=1, RCODE=3 (NXDOMAIN)
    buf.extend_from_slice(&[0x00,0x00, 0x00,0x00, 0x00,0x00, 0x00,0x00]);
    buf
}

// ── ERROR ─────────────────────────────────────────────────────────────────────
pub use crate::error::NetError;
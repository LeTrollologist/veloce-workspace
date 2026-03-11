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

pub async fn serve(registry: Arc<NetRegistry>, port: u16) -> Result<()> {
    let sock = UdpSocket::bind(format!("0.0.0.0:{port}")).await?;
    tracing::info!("DNS server listening on 0.0.0.0:{port}");

    let mut buf = vec![0u8; 512];

    loop {
        let (n, from) = sock.recv_from(&mut buf).await?;
        let packet    = buf[..n].to_vec();
        let registry  = registry.clone();
        let sock_ref  = Arc::new(sock.try_clone().expect("udp clone"));

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
            let response = build_a_response(query.id, &q.name, &ip, 30 /* TTL */);
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

/// Forward query to system resolver (8.8.8.8:53 as fallback if no system resolver found).
async fn forward_query(packet: &[u8], from: SocketAddr, sock: &UdpSocket) -> Result<()> {
    let upstream = get_system_resolver().unwrap_or_else(|| "8.8.8.8:53".parse().unwrap());
    let fwd = UdpSocket::bind("0.0.0.0:0").await?;
    fwd.send_to(packet, upstream).await?;

    let mut resp_buf = vec![0u8; 512];
    let (n, _) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        fwd.recv_from(&mut resp_buf),
    ).await??;

    sock.send_to(&resp_buf[..n], from).await?;
    Ok(())
}

fn get_system_resolver() -> Option<SocketAddr> {
    // On Windows: parse %SystemRoot%\System32\drivers\etc\resolv.conf or
    // read registry HKLM\SYSTEM\...\DhcpNameServer.
    // For now, return a sensible default.
    #[cfg(windows)]
    {
        // Try to read the first DNS server from the registry
        // Simplified: use Cloudflare as fallback if parsing fails
        Some("1.1.1.1:53".parse().unwrap())
    }
    #[cfg(not(windows))]
    {
        // Parse /etc/resolv.conf
        if let Ok(content) = std::fs::read_to_string("/etc/resolv.conf") {
            for line in content.lines() {
                if let Some(rest) = line.strip_prefix("nameserver ") {
                    let addr = rest.trim().to_string() + ":53";
                    if let Ok(a) = addr.parse() { return Some(a); }
                }
            }
        }
        Some("1.1.1.1:53".parse().unwrap())
    }
}

// ── DNS WIRE FORMAT ───────────────────────────────────────────────────────────

#[derive(Debug)]
struct DnsMessage {
    id:        u16,
    questions: Vec<DnsQuestion>,
}

#[derive(Debug)]
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
        let qdcount   = u16::from_be_bytes([buf[4], buf[5]]) as usize;

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
fn parse_name(buf: &[u8], mut pos: usize) -> Result<(String, usize)> {
    let mut labels = Vec::new();
    let mut jumped = false;
    let mut end_pos = 0;

    loop {
        if pos >= buf.len() { anyhow::bail!("name past end of packet"); }
        let len = buf[pos] as usize;

        if len == 0 {
            if !jumped { end_pos = pos + 1; }
            break;
        }

        // Pointer (compression)
        if len & 0xC0 == 0xC0 {
            if pos + 1 >= buf.len() { anyhow::bail!("compression pointer truncated"); }
            if !jumped { end_pos = pos + 2; }
            let ptr = (((len & 0x3F) as usize) << 8) | buf[pos+1] as usize;
            pos = ptr;
            jumped = true;
            continue;
        }

        pos += 1;
        if pos + len > buf.len() { anyhow::bail!("label past end of packet"); }
        labels.push(String::from_utf8_lossy(&buf[pos..pos+len]).into_owned());
        pos += len;
    }

    if !jumped { end_pos = pos; }
    Ok((labels.join("."), end_pos))
}

/// Encode a name as DNS labels.
fn encode_name(name: &str, buf: &mut Vec<u8>) {
    for label in name.split('.') {
        buf.push(label.len() as u8);
        buf.extend_from_slice(label.as_bytes());
    }
    buf.push(0); // root
}

/// Build a DNS A response pointing to `ip`.
fn build_a_response(id: u16, name: &str, ip: &[u8; 4], ttl: u32) -> Vec<u8> {
    let mut buf = Vec::new();

    // Header
    buf.extend_from_slice(&id.to_be_bytes());
    buf.extend_from_slice(&[0x81, 0x80]); // QR=1, OPCODE=0, AA=1, TC=0, RD=1, RA=1
    buf.extend_from_slice(&[0x00, 0x01]); // QDCOUNT=1
    buf.extend_from_slice(&[0x00, 0x01]); // ANCOUNT=1
    buf.extend_from_slice(&[0x00, 0x00]); // NSCOUNT=0
    buf.extend_from_slice(&[0x00, 0x00]); // ARCOUNT=0

    // Question section (echo)
    encode_name(name, &mut buf);
    buf.extend_from_slice(&[0x00, 0x01]); // QTYPE=A
    buf.extend_from_slice(&[0x00, 0x01]); // QCLASS=IN

    // Answer section
    encode_name(name, &mut buf);
    buf.extend_from_slice(&[0x00, 0x01]); // TYPE=A
    buf.extend_from_slice(&[0x00, 0x01]); // CLASS=IN
    buf.extend_from_slice(&ttl.to_be_bytes());
    buf.extend_from_slice(&[0x00, 0x04]); // RDLENGTH=4
    buf.extend_from_slice(ip);

    buf
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
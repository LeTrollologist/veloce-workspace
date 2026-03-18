# VeloceNetwork — Post-v0.7 Security Audit

**Date:** 2026-03-13
**Auditor:** Claude Sonnet 4.6 (automated code review)
**Codebase:** `claude/clever-keller` @ `06280da`
**Scope:** All crates (veloce-core, veloce-ipc, veloce-net, veloce-mesh, veloce-sdk)

This audit was performed after v0.7.0's nine remediations landed, to identify any
remaining exposure before work begins on v0.8.0.

---

## Executive Summary

**9 new findings** across 4 crates. Two are critical (missing RBAC on destructive IPC
operations). The remainder are high/medium/low hardening items affecting DNS, mesh
gossip, process spawning, and peer message parsing.

| Severity | Count |
|----------|-------|
| Critical | 2 |
| High | 2 |
| Medium | 3 |
| Low | 2 |
| **Total** | **9** |

**Recommendation:** Target all Critical and High findings for v0.8.0. The Medium and Low
items can follow in v0.8.x patches or be folded into the v0.9.0 final audit.

---

## Findings

---

### N1 — Missing Capability Checks on Destructive IPC Handlers

**Severity:** Critical
**File:** `crates/veloce-core/src/ipc_server.rs`, lines 377–508

**Root cause:** Six IPC handlers execute sensitive operations without calling
`self.require_cap(...)`. Any authenticated client — regardless of its declared
capability set — can invoke these handlers.

**Affected handlers and impact:**

| Handler | Missing Capability | Impact |
|---|---|---|
| `NetUnregisterHost` | `NetRegister` | Any client can unregister any `.vln` hostname, disrupting routing for other apps |
| `MeshConnect` | `MeshManage` (new) or `NetRegister` | Any client can initiate new encrypted mesh peer connections, expanding the attack surface |
| `MeshDisconnect` | Same as above | Any client can tear down active mesh tunnels |
| `PolicyReload` | Admin/privileged cap | Any client can force a hot-reload of `veloce-policy.toml`; if the file is writeable by the client, this is privilege escalation |

**Lower-severity missing checks (information disclosure):**

| Handler | Missing Capability | Impact |
|---|---|---|
| `QueryNodes` | `QueryNodes` (new) or `SpawnNodes` | Lists all running node IDs, PIDs, pipe paths |
| `QueryNodeResources` | Same | Exposes CPU/memory of all nodes |
| `SubscribeNodeEvents` / `SubscribeNodeLogs` | `SpawnNodes` or node-ownership check | Can subscribe to events/logs of nodes the client did not spawn |
| `PolicyGetRules` | — | Exposes the active policy file content (lower risk but unintended) |

**Fix:**
1. Add `self.require_cap(Capability::NetRegister)?;` to `NetUnregisterHost`.
2. Add a new `Capability::MeshManage` variant and gate `MeshConnect` / `MeshDisconnect`
   behind it.
3. Add `self.require_cap(Capability::PolicyAdmin)?;` (new variant) to `PolicyReload`.
4. Add appropriate capability checks to the read-only handlers listed above, or
   document them as intentionally open for all authenticated clients.

---

### N2 — `quote_arg` Does Not Escape Trailing Backslashes

**Severity:** Critical
**File:** `crates/veloce-core/src/job.rs`, line 627

**Root cause:** The Windows command-line quoting function escapes embedded double-quotes
but does not escape backslashes immediately before the closing quote character.

```rust
// Current:
format!("\"{}\"", s.replace('"', "\\\""))
// A path "C:\tools\" becomes "C:\tools\" ← the \" ends the quote prematurely
```

By the MSVC `CommandLineToArgvW` spec, a backslash before a closing `"` is interpreted
as an escape sequence. A node executable path ending in `\` (e.g., a bare directory
path), or an argument containing `\"`, will be parsed incorrectly by the spawned
process's arg parser, potentially injecting extra arguments.

**Fix:** Escape contiguous backslash runs that precede a double-quote:
```rust
fn quote_arg(s: &str) -> String {
    if s.chars().any(|c| c == ' ' || c == '"' || c == '\\') {
        // Escape backslashes before closing quote per MSVC CRT rules
        let inner = s.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{inner}\"")
    } else {
        s.to_string()
    }
}
```
(See Raymond Chen's "The Old New Thing" for the authoritative MSVC quoting rules.)

---

### N3 — DNS Forward Query Does Not Validate Transaction ID

**Severity:** High
**File:** `crates/veloce-net/src/dns.rs`, lines 84–103

**Root cause:** `forward_query` sends a query packet to the upstream resolver and
accepts the first UDP response whose source address matches `upstream`. It does **not**
validate that the response's DNS transaction ID (bytes 0–1) matches the transaction ID
from the original query packet.

The system resolver is hardcoded to `1.1.1.1:53` on Windows. A local process that can
craft a UDP packet with source `1.1.1.1:53` — possible via raw sockets under some
Windows configurations, or if the DNS port is reachable via loopback — can inject a
spoofed DNS response for any query, poisoning the DNS reply sent back to the original
requester.

**Fix:** Copy the transaction ID from the query, then verify it in the response:

```rust
async fn forward_query(packet: &[u8], from: SocketAddr, sock: &UdpSocket) -> Result<()> {
    if packet.len() < 2 { anyhow::bail!("DNS query too short"); }
    let txn_id = [packet[0], packet[1]];   // ← capture request ID

    let upstream = get_system_resolver().unwrap_or_else(|| "1.1.1.1:53".parse().unwrap());
    let fwd = UdpSocket::bind("127.0.0.1:0").await?;   // ← bind loopback-only
    fwd.send_to(packet, upstream).await?;

    let mut resp_buf = vec![0u8; 512];
    let (n, src) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        fwd.recv_from(&mut resp_buf),
    ).await??;

    if src != upstream {
        anyhow::bail!("DNS upstream response from unexpected source {src}");
    }
    if n < 2 || resp_buf[..2] != txn_id {   // ← validate response ID
        anyhow::bail!("DNS response transaction ID mismatch");
    }

    sock.send_to(&resp_buf[..n], from).await?;
    Ok(())
}
```

Note: also change the forward socket bind from `0.0.0.0:0` to `127.0.0.1:0` so the
ephemeral UDP port is not reachable from non-local addresses.

---

### N4 — Gossip LWW Timestamp Not Validated Against Local Clock

**Severity:** High
**File:** `crates/veloce-mesh/src/peer.rs`, lines 292–313

**Root cause:** The Last-Write-Wins conflict resolution for gossip entries uses the
timestamp supplied by the remote peer:

```rust
if entry.ts > ex.ts {
    *ex = entry.clone();
    // ... install forwarder
}
```

There is no validation against the local system clock. An attacker (or a misconfigured
peer) can set `entry.ts = u64::MAX` to permanently "win" all future LWW comparisons for
a given hostname. Once installed, no honest peer can ever evict the forwarder because
their timestamps will always be less than `u64::MAX`.

**Impact:** A connected mesh peer can permanently hijack any `.vln` hostname on the
local machine — all traffic destined for that host would be forwarded to the attacker's
port.

**Fix:** Reject entries whose timestamps fall outside a ±5 minute window of local time:

```rust
let now = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0);
const SKEW_SECS: u64 = 300; // 5 minutes

for entry in &entries {
    // Reject future-dated and heavily stale entries
    if entry.ts > now + SKEW_SECS || entry.ts + 86400 < now {
        tracing::warn!(
            hostname = %entry.hostname,
            entry_ts  = entry.ts,
            local_now = now,
            "gossip entry timestamp out of acceptable range — discarding"
        );
        continue;
    }
    // ... existing LWW logic
}
```

---

### N5 — Peer JSON Message Has No Maximum Size Check

**Severity:** Medium
**File:** `crates/veloce-mesh/src/peer.rs`, lines 209–220

**Root cause:** After decrypting a Noise transport frame, the peer reader immediately
deserializes the plaintext as JSON with no size check before deserialization:

```rust
let msg: PeerMsg = match serde_json::from_slice(&plain) { ... }
```

A `PeerMsg::RegistrySync { entries }` with tens of thousands of gossip entries, or a
`PeerMsg::Hello { machine_name }` containing a multi-megabyte string, forces large
heap allocations inside the JSON parser before any semantic validation can reject it.

The individual Noise frame is bounded at 65 535 bytes (MAX_MSG), but an attacker peer
can send many frames in rapid succession, each deserializing independently.

**Fix:** Cap the plaintext size before parsing:

```rust
const MAX_PEER_MSG_BYTES: usize = 65_535;
if plain.len() > MAX_PEER_MSG_BYTES {
    tracing::warn!("peer {peer_id_r} sent oversized message ({} bytes) — dropping", plain.len());
    continue;
}
let msg: PeerMsg = match serde_json::from_slice(&plain) { ... }
```

Additionally, cap `RegistrySync` entry count inside `handle_incoming`:

```rust
PeerMsg::RegistrySync { mut entries } => {
    entries.truncate(1_000); // never process more than 1000 gossip entries per message
    // ... existing logic
}
```

---

### N6 — DNS `qdcount` Not Capped Before `Vec::with_capacity`

**Severity:** Medium
**File:** `crates/veloce-net/src/dns.rs`, line 154

**Root cause:** The DNS parser allocates capacity for questions based on the raw
`qdcount` field from the untrusted UDP packet:

```rust
let qdcount = u16::from_be_bytes([buf[4], buf[5]]) as usize;
let mut questions = Vec::with_capacity(qdcount);  // up to 65535 slots
```

While the loop cannot iterate past the actual packet contents (so questions is bounded
by the 512-byte packet), `Vec::with_capacity(65535)` still performs a heap reservation
for 65535 × `sizeof(DnsQuestion)` upfront. A local process sending many crafted DNS
packets in rapid succession forces repeated large heap allocations.

The DNS socket is bound to `127.0.0.1`, so this is a local-only DoS vector, but any
authenticated application on the machine (including an attacker who passed the PSK) can
trigger it.

**Fix:**
```rust
let qdcount = (u16::from_be_bytes([buf[4], buf[5]]) as usize).min(5);
```

Standard DNS implementations use 1 question per query; capping at 5 is generous and
eliminates the allocation amplification.

---

### N7 — `forward_query` Forward Socket Bound to `0.0.0.0`

**Severity:** Medium
**File:** `crates/veloce-net/src/dns.rs`, line 87

**Root cause:** The ephemeral UDP socket used for upstream DNS forwarding is bound to
`0.0.0.0:0`:

```rust
let fwd = UdpSocket::bind("0.0.0.0:0").await?;
```

This exposes the ephemeral port on all interfaces. An attacker on the LAN who observes
the ephemeral port (via network scan or timing) can send a crafted UDP packet to it
from any interface before the real upstream responds.

**Fix:** Bind to `127.0.0.1:0`:
```rust
let fwd = UdpSocket::bind("127.0.0.1:0").await?;
```

(Combined with the transaction ID check in N3, this significantly hardens the forward
path against local response injection.)

---

### N8 — STUN Response Source Not Validated

**Severity:** Low
**File:** `crates/veloce-mesh/src/stun.rs`, line 80

**Root cause:** `try_stun` calls `sock.recv_from(&mut buf)` and ignores the `_from`
source address — it does not verify that the response came from `server_addr`. The
transaction ID check provides protection, but a lucky guess or brute-forced txn_id
(96-bit, computationally infeasible) would bypass it.

**Fix:**
```rust
let (n, from) = timeout(Duration::from_secs(3), sock.recv_from(&mut buf)).await??;
anyhow::ensure!(from == server_addr, "STUN response from unexpected source {from}");
```

---

### N9 — STUN Magic Cookie Not Validated

**Severity:** Low
**File:** `crates/veloce-mesh/src/stun.rs`, line 86

**Root cause:** `parse_xor_mapped_address` validates `msg_type` and `txn_id` but does
not check that bytes 4–7 equal the RFC 5389 magic cookie (`0x2112A442`). Responses
from non-STUN services that happen to echo the transaction ID would pass validation.

**Fix:** Add one `ensure!` after the existing message-type check:
```rust
let magic = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
anyhow::ensure!(magic == MAGIC_COOKIE, "STUN magic cookie mismatch: 0x{magic:08X}");
```

---

## False Positives / Mitigated Items

The following items were flagged during the audit but do not represent exploitable issues:

| Item | Reason Not a Finding |
|---|---|
| SOCKS5 allocation on `read_u8()` | `u8` max = 255 bytes per request; socket is localhost-only |
| Peer frame `vec![0u8; n]` allocation | `n` is a `u16` (max 65535); channel is post-Noise-handshake (authenticated) |
| Noise transport replay attacks | The `snow` crate implements an internal nonce counter and rejects replayed transport messages |
| FFI `value_len < 0` check | Already guarded correctly at `ffi.rs:312` |
| Registry `try_into().unwrap()` | Preceded by explicit length bounds checks throughout |

---

## Summary Table

| ID | Severity | Crate | Description |
|----|----------|-------|-------------|
| N1 | Critical | `veloce-core` | Missing `require_cap` on `NetUnregisterHost`, `MeshConnect`, `MeshDisconnect`, `PolicyReload` |
| N2 | Critical | `veloce-core` | `quote_arg` does not escape trailing backslashes — argument injection on Windows |
| N3 | High | `veloce-net` | DNS `forward_query` does not validate response transaction ID — poisoning vector |
| N4 | High | `veloce-mesh` | LWW gossip timestamp not compared to local clock — future-timestamp hostname hijack |
| N5 | Medium | `veloce-mesh` | No size cap on peer JSON before deserialization — memory exhaustion from authenticated peer |
| N6 | Medium | `veloce-net` | DNS `qdcount` not capped before `Vec::with_capacity` — local allocation DoS |
| N7 | Medium | `veloce-net` | DNS forward socket bound to `0.0.0.0` — ephemeral port exposed on all interfaces |
| N8 | Low | `veloce-mesh` | STUN response source address not validated |
| N9 | Low | `veloce-mesh` | STUN magic cookie (RFC 5389 §6) not validated in response parser |

---

## Recommended Scope for v0.8.0

All nine findings are tractable — none requires architectural changes. Suggested target:

**v0.8.0 must-fix (Critical + High):** N1, N2, N3, N4

**v0.8.0 should-fix (Medium):** N5, N6, N7

**v0.8.x / v0.9.0 (Low):** N8, N9

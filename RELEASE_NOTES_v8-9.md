# VeloceNetwork v0.8 – v0.9 — Release Notes

This file covers **v0.8.0** (Security Audit 2 of 3) and **v0.9.0** (Security Audit 3 of 3 +
Mesh Improvements). For earlier releases see `RELEASE_NOTES_v0.7.0.md` and the per-release
files for v0.4 – v0.6.

---

# v0.8.0

**Release:** v0.8.0
**Theme:** Security Audit 2 of 3 — IPC Capability Enforcement, Argument Injection Fix, DNS/Gossip Hardening

---

## Overview

v0.8.0 is the second of three planned security audits of the VeloceNetwork codebase.
No new user-facing features land in this release. Every change is a targeted remediation
of a finding from the post-v0.7 audit (`SECURITY_AUDIT_POST_v0.7.md`).

Seven findings (N1–N7) were identified and resolved across four crates. The two remaining
low-severity findings (N8, N9 — STUN response validation) were deferred to v0.9.

The audit classified findings by severity: **Critical (N/A in this audit)**, **High (N)**,
and **Medium (N)**.

---

## Security Findings & Remediations

### N1 — Missing Capability Checks on Destructive IPC Handlers

**Severity:** High (privilege escalation / DoS for authenticated clients)
**File:** `crates/veloce-core/src/ipc_server.rs`

**Root cause:** Four IPC handlers accepted requests from any authenticated client regardless
of the client's granted capability set:

| Handler | Risk |
|---|---|
| `NetUnregisterHost` | Any client could unregister any `.vln` hostname, disrupting routing for other apps |
| `MeshConnect` | Any client could initiate new encrypted mesh tunnels without the mesh capability |
| `MeshDisconnect` | Any client could tear down active mesh tunnels |
| `PolicyReload` | Any client could force a hot-reload of `veloce-policy.toml`; if the file were writable by the client, this was privilege escalation |

**Fix:**
- Added `self.require_cap(Capability::NetRegister)?` to `NetUnregisterHost`.
- Added two new capability variants: `Capability::MeshManage` and `Capability::PolicyAdmin`.
- `MeshConnect` and `MeshDisconnect` now require `Capability::MeshManage`.
- `PolicyReload` now requires `Capability::PolicyAdmin`.

**Impact:** Any SDK client relying on `MeshConnect`, `MeshDisconnect`, or `PolicyReload`
must declare the corresponding capability in its `VeloceClient::connect()` call.

---

### N2 — Command-Line Argument Injection via Trailing Backslash

**Severity:** High (argument injection into spawned node processes)
**File:** `crates/veloce-core/src/job.rs`

**Root cause:** `quote_arg` escaped embedded double-quotes but did not escape backslashes
immediately before the closing quote character. By the MSVC `CommandLineToArgvW` spec, a
backslash preceding a closing `"` is an escape sequence. A node executable path ending in
`\` (e.g., a bare directory) or an argument containing `\"` would be parsed incorrectly
by the spawned process, potentially injecting extra arguments.

```
// Before — "C:\tools\" becomes  "C:\tools\"  ← \" ends the quote prematurely
format!("\"{}\"", s.replace('"', "\\\""))
```

**Fix:** Backslash sequences that immediately precede a double-quote are now doubled before
the closing quote is appended, per the MSVC CRT quoting rules documented by Raymond Chen.

---

### N3 — DNS Upstream Response Transaction ID Not Validated

**Severity:** High (DNS response poisoning)
**File:** `crates/veloce-net/src/dns.rs`

**Root cause:** `forward_query` sent a DNS query to the upstream resolver and returned the
first response whose source address matched `upstream`. It did not validate that the
response's DNS transaction ID (bytes 0–1) matched the outgoing query. A local process
that could send a crafted UDP packet with source `1.1.1.1:53` before the real upstream
replied could inject a forged DNS response.

**Fix:**
- The transaction ID is captured from the outgoing query before it is forwarded.
- The response transaction ID is validated against the captured value; mismatches are
  silently discarded.
- The ephemeral forward socket is also rebound to `127.0.0.1:0` (see N7).

---

### N4 — Gossip LWW Timestamp Not Validated Against Local Clock

**Severity:** High (permanent `.vln` hostname hijack by a connected peer)
**File:** `crates/veloce-mesh/src/peer.rs`

**Root cause:** The Last-Write-Wins conflict resolution for gossip entries used the
timestamp supplied by the remote peer without any local-clock sanity check. A peer could
set `entry.ts = u64::MAX` to permanently win every future LWW comparison for a given
hostname, preventing any honest peer from ever evicting the forwarder — silently redirecting
all traffic destined for that hostname to the attacker's port.

**Fix:** Gossip entries whose timestamps fall outside a ±5-minute window relative to the
local `SystemTime` are discarded with a `warn!` log entry. This tolerates normal clock
skew between machines while rejecting far-future and heavily stale entries.

---

### N5 — No Size Cap on Peer JSON Messages Before Deserialization

**Severity:** Medium (memory exhaustion from authenticated peer)
**File:** `crates/veloce-mesh/src/peer.rs`

**Root cause:** After Noise transport decryption, the plaintext was passed directly to
`serde_json::from_slice` without a size check. An authenticated peer could send many
Noise frames, each containing a maximally-sized `RegistrySync` message with thousands
of gossip entries, forcing large heap allocations before any semantic validation.

**Fix:**
- `MAX_PEER_MSG_BYTES` constant (65,535 bytes) checked before JSON deserialization; oversized
  frames are dropped with a `warn!` log entry.
- `RegistrySync` entry lists are truncated to 1,000 entries per message inside
  `handle_incoming`, regardless of how many entries were deserialized.

---

### N6 — DNS `qdcount` Not Capped Before `Vec::with_capacity`

**Severity:** Medium (local allocation amplification DoS)
**File:** `crates/veloce-net/src/dns.rs`

**Root cause:** The DNS parser allocated `Vec::with_capacity(qdcount)` where `qdcount`
came directly from bytes 4–5 of the untrusted UDP packet (max 65,535). While the parse
loop was bounded by actual packet size, the upfront heap reservation for up to 65,535
slots was performed unconditionally. Any local process could trigger repeated large
allocations by sending crafted DNS queries in rapid succession.

**Fix:** `qdcount` is capped at 5 before the `with_capacity` call. Standard DNS
implementations use exactly 1 question per query; 5 is generous.

---

### N7 — DNS Forward Socket Bound to All Interfaces

**Severity:** Medium (ephemeral UDP port exposed externally)
**File:** `crates/veloce-net/src/dns.rs`

**Root cause:** The ephemeral UDP socket used to forward queries to the upstream resolver
was bound to `0.0.0.0:0`, exposing it on every network interface. An attacker on the
local network who observed the ephemeral port could potentially send a crafted response
from a non-local address before the real upstream replied.

**Fix:** Rebound to `127.0.0.1:0`. Combined with the transaction ID check in N3, this
closes the forward path against both local and remote response injection.

---

## Summary Table

| ID | Severity | Crate(s) | Description |
|----|----------|----------|-------------|
| N1 | High | `veloce-core` | Missing `require_cap` on `NetUnregisterHost`, `MeshConnect`, `MeshDisconnect`, `PolicyReload` |
| N2 | High | `veloce-core` | `quote_arg` backslash injection — MSVC `CommandLineToArgvW` quoting fixed |
| N3 | High | `veloce-net` | DNS `forward_query` transaction ID validation added |
| N4 | High | `veloce-mesh` | Gossip LWW timestamp validated against local clock (±5 min) |
| N5 | Medium | `veloce-mesh` | Peer JSON size cap + `RegistrySync` entry count limit |
| N6 | Medium | `veloce-net` | DNS `qdcount` capped at 5 before `Vec::with_capacity` |
| N7 | Medium | `veloce-net` | DNS forward socket rebound to `127.0.0.1:0` |

---

## Breaking Changes

**SDK consumers:** Any client that calls `MeshConnect`, `MeshDisconnect`, or `PolicyReload`
must now declare `Capability::MeshManage` or `Capability::PolicyAdmin` respectively in the
`connect()` capability list, or those calls will return `PolicyDenied (11)`.

All IPC discriminants are unchanged. The `veloce-policy.toml` schema is unchanged.

**SOCKS5 / DNS:** No behaviour changes beyond the hardening fixes above.

---

## What's Next

| Version | Focus |
|---|---|
| **v0.9** | Security Audit 3 of 3 + VM3 join codes, gossip ownership tracking, mesh diagnostic CLI |
| **v1.0** | WireGuard-NT kernel driver + signed auto-update installer + NRPT `.vln` routing |

---

---

# v0.9.0

**Release:** v0.9.0
**Theme:** Security Audit 3 of 3 + Mesh Improvements (VM3 Join Codes, Gossip Ownership, Re-sync, Diagnostic CLI)

---

## Overview

v0.9.0 is the third and final structured security audit before v1.0, combined with a set
of targeted mesh improvements that address gaps identified during the audit cycle. Two
remaining low-severity findings from the post-v0.7 audit are closed. Four improvement
items (S1, S2, O1, O3) land alongside them.

After v0.9.0, the codebase has been through three complete structured audits. The platform
is hardened and feature-complete for v1.0.

---

## Security Fixes

### N8 — STUN Response Source Address Not Validated

**Severity:** Low
**File:** `crates/veloce-mesh/src/stun.rs`

**Root cause:** `try_stun` called `sock.recv_from(&mut buf)` and ignored the source
address. While the transaction ID check provided meaningful protection, validating the
source address adds defense-in-depth at negligible cost.

**Fix:** After `recv_from`, `from` is checked against `server_addr`; responses from
unexpected sources are rejected with `anyhow::ensure!`.

---

### N9 — STUN Magic Cookie Not Validated

**Severity:** Low
**File:** `crates/veloce-mesh/src/stun.rs`

**Root cause:** `parse_xor_mapped_address` validated `msg_type` and `txn_id` but did not
check that bytes 4–7 equal the RFC 5389 magic cookie (`0x2112A442`). Non-STUN services
that echoed the transaction ID would pass validation.

**Fix:** An `ensure!` check on `data[4..8]` == `MAGIC_COOKIE` is added immediately after
the message-type check.

---

## New Features

### S1 — VM3 Join Codes (TTL + One-Time Use)

**Files:** `crates/veloce-mesh/src/identity.rs`, `crates/veloce-mesh/src/lib.rs`,
`apps/veloce-run/src/main.rs`

VM3 is a new join code format that extends VM2 with three additional fields:

```
VM2 payload (variable) | created_at[8 LE] | nonce[16] | ttl_mins[2 LE] | flags[1]
```

- `created_at`: Unix timestamp (seconds) of code creation.
- `nonce`: 16-byte cryptographic nonce, unique per code.
- `ttl_mins`: expiry window in minutes from `created_at`. `0` = no expiry.
- `flags`: bit 0 = `FLAG_ONE_TIME`. When set, the code may only be used once.

**TTL enforcement:** At connect time, `connect_to_peer()` decodes the VM3 metadata and
rejects codes where `now > created_at + ttl_mins * 60`.

**One-time-use enforcement:** `MeshState` maintains `used_nonces: Mutex<HashSet<[u8;16]>>`.
Before the Noise handshake begins, the nonce is inserted; if it was already present the
connection is rejected immediately. On handshake failure the nonce is removed (rollback),
so the code remains usable after a transient failure.

**CLI usage:**

```powershell
# Standard VM2 code (no expiry)
veloce-run mesh identity

# VM3 code expiring in 30 minutes
veloce-run mesh identity --ttl 30

# VM3 one-time code expiring in 1 hour
veloce-run mesh identity --ttl 60 --one-time
```

**Backward compatibility:** VM1 and VM2 codes are still accepted by `connect_to_peer()`.
VM3 codes are base64-encoded with the `VM3:` prefix; VeloceNetwork v0.8 and earlier will
reject them with a parse error.

---

### S2 — Gossip Ownership Tracking

**Files:** `crates/veloce-mesh/src/lib.rs`, `crates/veloce-mesh/src/peer.rs`

**Problem:** A connected mesh peer could silently overwrite any `.vln` hostname in the
local `NetRegistry` by gossiping a newer timestamp for a name it did not originate. This
allowed a peer to redirect traffic for names registered by other peers or by the local
machine.

**Fix:** `MeshState` now maintains `hostname_origins: Arc<Mutex<HashMap<String, Uuid>>>` —
a map from hostname to the peer UUID that first registered it. The `make_owner_fn()` method
produces a `OwnerFn` closure (type alias: `Arc<dyn Fn(&str, Uuid, &str) -> bool + Send + Sync>`)
that is passed into `PeerConnection::start()`. Inside `handle_incoming`, each `RegistrySync`
entry is checked against the origin map before being processed. If the entry's source peer
does not match the recorded origin, the entry is:

1. Dropped (not installed as a forwarder).
2. Logged at `warn!` level with the hostname, originating peer ID, and attempting peer ID.

This prevents both accidental and malicious hostname squatting by connected peers while
preserving normal gossip behaviour for entries that the peer legitimately owns.

---

### O1 — Periodic Gossip Re-Sync

**File:** `crates/veloce-mesh/src/peer.rs`

**Problem:** Gossip entries propagated only on change. If a peer missed a gossip message
(e.g., during a transient TCP stall or reconnect) its local `NetRegistry` could diverge
permanently from the peer's view, requiring a manual reconnect to repair.

**Fix:** `PeerConnection::start()` accepts a new `gossip_interval_secs: u64` parameter.
When non-zero, a `tokio::time::interval` is created and an extra arm added to the reader
task's `select!` loop. Every `gossip_interval_secs` seconds, the full local `NetRegistry`
is serialised as a `PeerMsg::RegistrySync` and sent to the peer. The default is 60 seconds.

No changes to the gossip wire format or any IPC message types.

---

### O3 — Mesh Diagnostic CLI Subcommands

**Files:** `apps/veloce-run/src/main.rs`, `crates/veloce-ipc/src/message.rs`,
`crates/veloce-sdk/src/client.rs`, `crates/veloce-core/src/ipc_server.rs`

Three new `veloce-run mesh` subcommands:

#### `mesh status`

Prints a summary table of all connected peers: peer ID, display name, last-sampled
latency in milliseconds, and number of remote `.vln` hosts visible through that peer.

```
$ veloce-run mesh status
PEER ID                              NAME         LATENCY  REMOTE HOSTS
a1b2c3d4-...                         DESKTOP-B    3 ms     4
e5f6a7b8-...                         LAP-DEV      12 ms    2
```

#### `mesh diagnose`

Prints a connectivity health report: mesh listen port, WAN address (from STUN), number of
active peers, and per-peer connection state (connected / reconnecting / error).

#### `mesh ping <peer-id>`

Measures and prints the round-trip latency to a specific peer using the new
`MeshPingPeer (0x58)` / `MeshPingResult (0x59)` IPC round-trip.

**New IPC messages:**

| Discriminant | Name | Direction | Body |
|---|---|---|---|
| `0x58` | `MeshPingPeer` | Client → Server | `{ peer_id: Uuid }` |
| `0x59` | `MeshPingResult` | Server → Client | `{ peer_id: Uuid, latency_ms: Option<u32> }` |

**Implementation:** Each `PeerConnection` maintains an `Arc<AtomicU32>` `latency_ms` field
updated by the reader task from the Noise keep-alive round-trip. The `MeshPingPeer` handler
reads `latency_ms.load(SeqCst)` and returns `None` for the zero sentinel (no sample yet).

**SDK method:** `VeloceClient::mesh_ping_peer(peer_id: Uuid) -> Result<Option<u32>>`.

---

## IPC Discriminant Table (new in v0.9)

| Discriminant | Name | Direction |
|---|---|---|
| `0x58` | `MeshPingPeer` | Client → Server |
| `0x59` | `MeshPingResult` | Server → Client |

All previous discriminants (0x00–0x72, 0x80–0x81) are unchanged.

---

## Breaking Changes

None for existing SDK consumers or CLI users. All changes are additive:
- VM2 and VM1 join codes continue to work.
- Clients that do not declare `MeshManage` / `PolicyAdmin` capabilities lose access to
  those handlers (this was the v0.8.0 breaking change; v0.9.0 adds no new restrictions).
- The new `gossip_interval_secs` parameter in `PeerConnection::start()` is internal to
  `veloce-mesh`; it is not part of any public API.

---

## Summary of Changes Across v0.8 and v0.9

| Release | Type | Item | Description |
|---------|------|------|-------------|
| v0.8.0 | Security | N1 | Missing `require_cap` on 4 IPC handlers |
| v0.8.0 | Security | N2 | `quote_arg` argument injection via trailing backslash |
| v0.8.0 | Security | N3 | DNS transaction ID validation |
| v0.8.0 | Security | N4 | Gossip LWW timestamp clock-skew guard |
| v0.8.0 | Security | N5 | Peer JSON size cap + `RegistrySync` entry limit |
| v0.8.0 | Security | N6 | DNS `qdcount` allocation cap |
| v0.8.0 | Security | N7 | DNS forward socket loopback bind |
| v0.9.0 | Security | N8 | STUN response source validation |
| v0.9.0 | Security | N9 | STUN RFC 5389 magic cookie validation |
| v0.9.0 | Feature | S1 | VM3 join codes — TTL + one-time-use nonce blacklist |
| v0.9.0 | Feature | S2 | Gossip ownership tracking — origin-bound hostname map |
| v0.9.0 | Feature | O1 | Periodic gossip re-sync (60 s default) |
| v0.9.0 | Feature | O3 | `mesh status` / `mesh diagnose` / `mesh ping` CLI; `MeshPingPeer`/`MeshPingResult` IPC |

---

## What's Next

| Version | Focus |
|---|---|
| **v1.0** | WireGuard-NT kernel driver · NRPT `.vln` routing · signed auto-update installer · winget/scoop package |
| **v2.0** | Linux port (cgroups v2 + Unix sockets) · Python / Node.js / Go SDK bindings |

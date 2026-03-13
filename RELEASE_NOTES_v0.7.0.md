# VeloceNetwork v0.7.0 — Release Notes

**Release:** v0.7.0
**Branch:** `claude/clever-keller`
**Theme:** Security Audit 1 of 3 — IPC Hardening, Mesh Hardening, Network Scope Restriction

---

## Overview

v0.7.0 is the first of three planned structured security audits of the VeloceNetwork codebase.
No new user-facing features land in this release. Every change is a targeted remediation of a
finding from the audit. Nine issues were identified and resolved across five crates.

The audit classified findings by severity: **Critical (C)**, **High (H)**, and **Medium (M)**.

---

## Security Findings & Remediations

### C1 — TOCTOU: Client Identity Resolved After Data Read

**Severity:** Critical
**Files:** `crates/veloce-core/src/pipe_security.rs`, `crates/veloce-core/src/policy.rs`,
`crates/veloce-ipc/src/message.rs`

**Root cause:** `assert_client_is_owner` previously returned `Result<()>` — it verified the
client's user SID but discarded the resolved exe path. Policy rules based on `app` name relied
on the client-declared `app_name` string from the IPC handshake, which any process can forge.

**Fix:**
- `assert_client_is_owner` now returns `Result<String>` — the kernel-verified Win32 image path
  resolved via `QueryFullProcessImageNameW` **while the process handle is open**, before any IPC
  frame is read. This eliminates the TOCTOU window.
- New `unsafe fn query_process_exe(proc: HANDLE) -> Result<String>` in `pipe_security.rs`.
- `PolicyRule` gains an `exe: Option<String>` field (serde default = `None`) — when present, it
  is matched against the kernel-verified path via `exe_glob_match` (full path or basename). The
  legacy `app` field remains for backward compatibility but is used only when `exe` is absent.
- `PolicyRuleMsg` in `veloce-ipc` gains `exe: Option<String>` with `#[serde(default)]` for
  backward-compatible wire encoding.

**Impact:** Policy rules can now be anchored to verified executable identity rather than
self-reported app names.

---

### C2 — Server-Authoritative Capability Grant

**Severity:** Critical
**File:** `crates/veloce-core/src/ipc_server.rs`, `crates/veloce-core/src/policy.rs`

**Root cause:** Capability grants were determined by intersecting the client's requested
capabilities with the policy, but only at individual handler dispatch time via
`check_capability(&self.app_name, cap)`. This used the unverified `app_name` string and
required every new handler to remember to call the check.

**Fix:**
- New `PolicyEngine::compute_max_caps(exe_path: &str) -> Vec<Capability>` computes the full
  server-authoritative grant once at handshake time from the kernel-verified exe path.
- The handshake handler intersects the client's request with the server grant:
  `granted = client_request ∩ server_max_caps`. The stored `self.capabilities` set is the
  authority for all per-request capability checks thereafter.
- Redundant per-handler `check_capability` calls removed from SpawnNode, KillNode, and
  NetRegisterHost.

---

### H1 — Pre-Authentication Protocol State Machine

**Severity:** High
**File:** `crates/veloce-core/src/ipc_server.rs`

**Root cause:** The IPC message dispatch loop processed any message type — including
`SpawnNode`, `NetRegisterHost`, etc. — before the client completed the `Handshake` exchange.
A malicious local process could trigger node-management actions without ever authenticating.

**Fix:** Non-`Handshake` messages received before `self.client_id` is set cause the connection
to be dropped immediately with no error response. The exact message type is logged at `warn`
level for forensic visibility.

---

### H2 — DNS Server Exposed on All Network Interfaces

**Severity:** High
**File:** `crates/veloce-net/src/dns.rs`

**Root cause:** `UdpSocket::bind("0.0.0.0:{port}")` exposed the DNS server on every network
interface, making it reachable from the LAN and potentially from the internet.

**Fix:** Bind changed to `127.0.0.1:{port}`. The DNS server serves only local processes.

---

### H3 — DNS Upstream Response Spoofing

**Severity:** High
**File:** `crates/veloce-net/src/dns.rs`

**Root cause:** `forward_query` called `socket.recv_from()` without validating that the reply
came from the configured upstream resolver. Any local process could race to send a spoofed DNS
response before the real upstream replied.

**Fix:** After `recv_from`, the source address is compared against the upstream socket address.
Replies from unexpected sources are silently discarded. The upstream socket is bound to
`127.0.0.1:0` so it is not reachable from non-local addresses.

---

### H4 — Noise Mesh Handshake Read Timeout Missing

**Severity:** High
**File:** `crates/veloce-mesh/src/noise.rs`

**Root cause:** `responder_handshake` called `read_frame(stream).await?` with no timeout on
the first read (msg1). An attacker with TCP access to the mesh port (`:7474`) could open
connections and stall indefinitely, exhausting Tokio task slots. `initiator_handshake` had a
timeout only on msg2.

**Fix:** Both reads are now wrapped in `tokio::time::timeout(Duration::from_secs(10), ...)`.
Connections that do not complete the handshake within 10 seconds are rejected with a timeout
error and the task exits cleanly.

---

### M1 — SOCKS5 Proxy Accepts Any Local Destination

**Severity:** Medium
**File:** `crates/veloce-net/src/socks5.rs`

**Root cause:** The SOCKS5 proxy accepted `CONNECT` requests to any hostname, routing arbitrary
internet traffic through VeloceNet. Any local process could use it as a general-purpose proxy.
Its intended scope is exclusively `*.vln` / `*.veloce` private-TLD routing.

**Fix:** Non-VLN hostnames (anything not ending in `.vln` or `.veloce`) receive a SOCKS5
`REP_UNREACHABLE` reply and the connection is closed. The restriction is documented in the
proxy's bind log message. IPv4/IPv6 literal `CONNECT` requests are still rejected as before.

---

### M2 — Registry Write Panics on Oversized Input

**Severity:** Medium
**File:** `crates/veloce-core/src/registry.rs`

**Root cause:** The shared mmap registry used `assert!` to validate that key and value sizes
fit within the wire-format length fields (`u16` for keys, `u32` for values). A connected client
sending an oversized registry write would trigger a `panic!` in the service process.

**Fix:** `assert!` replaced with `anyhow::ensure!` — the error propagates gracefully, the
offending request returns an `InvalidMessage` error to the client, and the service continues
running.

---

### M3 — `VELOCE_SKIP_PSK=1` Honoured in Production Service Context

**Severity:** Medium
**File:** `crates/veloce-core/src/ipc_server.rs`

**Root cause:** `std::env::var("VELOCE_SKIP_PSK") == Ok("1")` bypassed PSK authentication
unconditionally. If a misconfigured service launcher set this variable (e.g., a copied dev
launch configuration), any local process could connect to VeloceCore without the PSK.

**Fix:** When `VELOCE_SKIP_PSK=1` is detected and the service is running as the SYSTEM account
(`server_sid == "S-1-5-18"` — the SID of the Windows SYSTEM account, used in production
service context), the flag is **silently ignored** and a `tracing::error!` is emitted.
When running as a regular user (development mode), the flag is still honoured but logs at
`tracing::error!` to ensure it is always visible in any log aggregator.

---

## Summary Table

| ID | Severity | Crate(s) Affected | Description |
|----|----------|-------------------|-------------|
| C1 | Critical | `veloce-core`, `veloce-ipc` | TOCTOU: kernel-verified exe path replaces client-declared app name |
| C2 | Critical | `veloce-core` | Server-authoritative capability grant computed at handshake time |
| H1 | High | `veloce-core` | Pre-auth state machine: non-Handshake messages before auth drop connection |
| H2 | High | `veloce-net` | DNS server bound to `127.0.0.1` instead of `0.0.0.0` |
| H3 | High | `veloce-net` | DNS upstream response source validation |
| H4 | High | `veloce-mesh` | Noise handshake 10-second read timeout on both sides |
| M1 | Medium | `veloce-net` | SOCKS5 proxy scoped to `.vln`/`.veloce` destinations only |
| M2 | Medium | `veloce-core` | Registry size guards use `anyhow::ensure!` instead of `assert!` |
| M3 | Medium | `veloce-core` | `VELOCE_SKIP_PSK=1` blocked when running as SYSTEM account |

---

## Breaking Changes

None for SDK consumers or CLI users. All IPC discriminants and SDK method signatures are
unchanged. The `veloce-policy.toml` schema gains an optional `exe` field per rule — existing
files with only `app` rules continue to work.

**Behaviour changes to be aware of:**
- The SOCKS5 proxy (`:1055`) no longer forwards connections to non-VLN hostnames. Applications
  using it as a general-purpose proxy must be updated to connect directly instead.
- Registry writes with oversized keys or values now return `InvalidMessage` rather than
  panicking the service.

---

## What's Next

| Version | Focus |
|---|---|
| **v0.7.x** | Patch releases for any regressions or follow-up fixes discovered post-audit |
| **v0.8** | Security Audit 2 of 3 — second structured audit cycle |
| **v0.9** | Security Audit 3 of 3 — final pre-1.0 audit; optimisation and profiling |
| **v1.0** | WireGuard-NT kernel driver + signed auto-update installer |

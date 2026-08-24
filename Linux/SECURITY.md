VeloceNetwork Security Policy (`SECURITY.md`)

#### 1. Our Commitment

VeloceNetwork is built on a "Security-First" philosophy. We believe that while our management core is proprietary, the protocols and networking layers must be transparent and verifiable. We are committed to a three-stage audit cycle (v0.7–v0.9) to ensure a hardened v1.0 release.

#### 2. Scope

This policy covers the following VeloceNetwork components:

* **Open Modules:** `veloce-net`, `veloce-mesh`, `veloce-ipc`, and the `veloce-sdk`.
* **Proprietary Core:** `veloce-core` (Binary analysis and behavioral auditing welcome).
* **Protocols:** The Veloce IPC framing and Noise_IK implementation.

#### 3. Reporting a Vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

If you discover a security flaw, please report it via the following channel:

* **Email:** trollologistog@gmail.com

**Please include the following in your report:**

1. A description of the vulnerability and its potential impact.
2. Steps to reproduce the issue (or a proof-of-concept script).
3. The version of VeloceNetwork affected.

#### 4. The "Veloce Disclosure" Timeline

We aim to acknowledge all reports within **48 hours**. We follow a coordinated disclosure timeline:

* **Remediation:** We prioritize fixes based on severity (Critical/High/Medium/Low).
* **Disclosure:** Once a fix is verified and shipped, we will publish a security advisory (similar to our v0.7.0 release notes) to inform the community.
* **Credit:** With your permission, we will credit you in our release notes and our "Security Hall of Fame."

#### 5. Safe Harbor

We encourage the community to research and audit VeloceNetwork. We will not take legal action against researchers who:

* Perform research without harming VeloceNetwork users or their data.
* Avoid DoS attacks against our infrastructure (e.g., STUN servers).
* Provide us a reasonable amount of time to fix the issue before public disclosure.

---

## Security Reference

The remainder of this document is a technical reference: threat model, security feature descriptions, and the complete audit history. It is intended for security researchers, auditors, and developers building on the platform.

---

## Threat Model

VeloceNetwork is a **single-machine desktop orchestration service** that extends across
machines via an encrypted peer-to-peer mesh. The core service (`veloce-core`) runs as a
Windows service under the SYSTEM account. Clients connect over a named pipe.

### Trust Boundaries

| Boundary | What crosses it | Protections |
|---|---|---|
| Named pipe / Unix socket (IPC) | Client ↔ VeloceCore | SID ACL (kernel-enforced) / UID check, OsRng PSK, pre-auth state machine |
| Mesh TCP `:7474` | Peer VeloceCore ↔ VeloceCore | Noise_IK mutual authentication, 10 s handshake timeout, join-code TTL/one-time |
| DNS UDP `:5354` | Local app ↔ VeloceNet | Localhost-only bind; upstream response source + transaction-ID validation |
| SOCKS5 TCP `:1055` | Local app ↔ VeloceNet | Localhost-only bind; `.vln`/`.veloce` scope restriction |
| Ingress HTTP `:8080` | Local app ↔ VeloceNet Ingress | Localhost-only bind; longest-prefix match, `NetRegister` capability required |
| Gossip & Control Plane | Multi-node cluster coordination | LWW clock-skew guard, ownership tracking, term monotonicity validation |

### Actors

- **Trusted local user** — the Windows user who installed and runs VeloceCore. Full access.
- **Authenticated client** — local process that presents the correct PSK; capability-restricted.
- **Unauthenticated local process** — any local process that can reach the named pipe. Must complete handshake before any action is taken.
- **Connected mesh peer** — remote VeloceCore that completed a Noise_IK handshake. May only install `.vln` hostnames it originated; all gossip is clock-validated.
- **Network adversary (LAN)** — can observe traffic and send packets to open UDP/TCP ports. Cannot break Noise_IK encryption; DNS/SOCKS5 services are localhost-bound.
- **Network adversary (WAN)** — same as LAN adversary. Mesh connections initiated from WAN require a valid join code.

### Out of Scope

- Physical access to the machine.
- Kernel-level malware or root-equivalent local attackers.
- The security of the applications spawned as nodes (VeloceNetwork sandboxes them but does not audit their code).

---

## Named-Pipe IPC Security

### SID-Based Pipe ACL

The named pipe `\\.\pipe\VeloceCore` is created with a **kernel-enforced DACL** that
restricts connection to the user SID that owns the running service. Cross-user connections
— including attempts by SYSTEM or other accounts — are rejected at the kernel level before
any data is read.

Implementation: `crates/veloce-core/src/pipe_security.rs` — `create_secure_pipe_acl()`.

### Per-Session OsRng PSK

At every startup, VeloceCore generates a fresh **32-byte (256-bit) OsRng PSK** and stores
it in the `CoreState`. Every IPC `Handshake` message must present the correct PSK. This
invalidates any connections held over from a previous service session (e.g., after a
restart or crash recovery) and ensures that processes that previously connected cannot
reconnect without re-reading the PSK from the service.

### Pre-Authentication State Machine

The IPC dispatch loop enforces a strict state machine: any message type other than
`Handshake` received before the client has successfully authenticated causes the
connection to be dropped immediately with no error response. The message type is logged at
`warn` for forensic visibility.

Implementation: `crates/veloce-core/src/ipc_server.rs`.

### Kernel-Verified Executable Identity (TOCTOU Prevention)

When a client connects, `assert_client_is_owner` resolves the **kernel-verified Win32
image path** via `QueryFullProcessImageNameW` **while the process handle is still open**,
before any IPC frame is read. This eliminates the TOCTOU window between process identity
check and message processing. Policy rules keyed on the `exe` field use this verified path
— clients cannot spoof their identity by declaring a false `app_name`.

Implementation: `crates/veloce-core/src/pipe_security.rs` — `query_process_exe()`.

### Server-Authoritative Capability Grant

At handshake time, `PolicyEngine::compute_max_caps(exe_path)` computes the full allowed
capability set for the connecting process using its **kernel-verified path**. The
client's requested capabilities are intersected with this server grant:

```
granted_caps = client_requested ∩ server_max_caps
```

The resulting `granted_caps` set is stored in `ClientSession` and is the sole authority
for all per-request capability checks. No handler re-checks the policy independently.

Capabilities (v0.9.0): `SpawnNodes`, `KillNodes`, `QueryNodes`, `RegistryRead`,
`RegistryWrite`, `NetRegister`, `MeshManage`, `PolicyAdmin`.

### `VELOCE_SKIP_PSK` Production Guard

The `VELOCE_SKIP_PSK=1` environment variable bypasses PSK authentication for development
convenience. When VeloceCore is running as the **SYSTEM account** (`server_sid == "S-1-5-18"`
— the production service context), this flag is **silently ignored** and a
`tracing::error!` is emitted. In non-SYSTEM contexts (developer/user mode), the flag is
still honoured but always logged at `error` level so it is visible in any log aggregator.

---

## Policy Engine

The **PolicyEngine** (`crates/veloce-core/src/policy.rs`) applies declarative RBAC to
all client actions:

- **Tier 1 — Process RBAC**: `veloce-policy.toml` rules may allow or deny specific
  capabilities per process, matched by kernel-verified exe path (`exe` field, full path
  or basename glob) or legacy app-name string (`app` field).
- **Tier 2 — Mesh ACLs**: rules filter which peer-gossiped `.vln` hostnames are installed
  as local TCP forwarders, optionally scoped to a specific source peer UUID.

The policy file is optional — absent = allow-all, fully backward compatible. It can be
hot-reloaded at runtime via `veloce-run policy reload` (requires `PolicyAdmin` capability).

Glob support: `"*"` and `"*.suffix"` in both exe names/paths and hostnames.

---

## Mesh Security

### Noise_IK Authenticated Key Exchange

Mesh connections use **Noise_IK_25519_ChaChaPoly_BLAKE2s** — the same cryptographic
pattern as WireGuard. Both initiator and responder mutually authenticate via **static
x25519 key pairs**. Identity keys are generated with `OsRng` at first run and stored in
`veloce-identity.key` with read-only, owner-only file ACL.

This is the **default and permanent** mesh transport. The optional WireGuard-NT kernel
backend (planned for v1.0) shares the same key material and handshake protocol but routes
packets through the NT driver for hardware offload — it does not change the security model.

### Handshake Timeout

Both `initiator_handshake` and `responder_handshake` apply a **10-second
`tokio::time::timeout`** to every read step. Connections that do not complete the
handshake within 10 seconds are rejected and the task exits cleanly. This prevents
task-slot exhaustion from idle TCP connections to the mesh port.

### Join Code Formats

| Format | Contents | Expiry |
|---|---|---|
| **VM1** | `pub_key[32]` + `addr[6]` | None |
| **VM2** | `pub_key[32]` + `n_addrs[1]` + `[family+ip+port]*` + `created_at[8 LE]` | None |
| **VM3** | VM2 payload + `nonce[16]` + `ttl_mins[2 LE]` + `flags[1]` | TTL (optional) + one-time (optional) |

**VM3 TTL enforcement:** `connect_to_peer()` rejects codes where
`now > created_at + ttl_mins * 60` (0 = no expiry).

**VM3 one-time-use enforcement:** When `flags & FLAG_ONE_TIME != 0`, the nonce is checked
against `MeshState::used_nonces` (a `Mutex<HashSet<[u8;16]>>`). If the nonce is already
present the connection is rejected. The nonce is rolled back on handshake failure so
transient failures do not consume the code.

### Gossip Timestamp Validation

Peer-supplied gossip timestamps are validated against the local `SystemTime`. Entries whose
timestamps fall outside a **±5-minute window** are discarded with a `warn!` log entry.
This prevents a peer from setting `ts = u64::MAX` to permanently win all future LWW
comparisons and hijack any `.vln` hostname on the local machine.

### Gossip Ownership Tracking

`MeshState` maintains a `hostname_origins` map from hostname to the UUID of the peer that
first registered it. Each `PeerConnection` receives an `OwnerFn` closure at start time.
Inside `handle_incoming`, gossip entries from a peer that does not match the recorded
origin are **silently discarded** and logged at `warn`. This prevents:

- Silent hostname squatting by a newly-connected peer.
- A compromised peer overwriting names it did not originate.

### Peer Message Size Limits

After Noise transport decryption, plaintext is subject to a `MAX_PEER_MSG_BYTES` (65,535)
check before JSON deserialization. `RegistrySync` entry lists are additionally truncated to
1,000 entries per message inside `handle_incoming`, regardless of frame count.

### STUN Response Validation

The STUN client (`crates/veloce-mesh/src/stun.rs`) validates:
1. Response source address == `server_addr`.
2. RFC 5389 magic cookie (bytes 4–7 == `0x2112A442`).
3. Message type == `BINDING_RESPONSE (0x0101)`.
4. Transaction ID matches the outgoing request.

---

## Network Surface Security

### DNS Server (`:5354`)

- **Localhost-only bind**: `UdpSocket::bind("127.0.0.1:5354")` — not reachable from LAN or WAN.
- **Compression loop protection**: the hand-rolled DNS parser enforces a maximum of 10 pointer jumps, preventing infinite loops from crafted compression chains.
- **`qdcount` allocation cap**: `Vec::with_capacity(qdcount.min(5))` prevents heap amplification from crafted `qdcount` values.
- **Upstream source validation**: `forward_query` validates the reply source against the upstream socket address; spoofed replies are silently discarded.
- **Transaction ID validation**: DNS response transaction ID is validated against the outgoing query before forwarding to the client.
- **Ephemeral forward socket**: bound to `127.0.0.1:0` — not reachable from non-local addresses.

### SOCKS5 Proxy (`:1055`)

- **Localhost-only bind**: not reachable from LAN or WAN.
- **VLN-only scope**: `CONNECT` requests to any destination not ending in `.vln` or `.veloce` receive `REP_UNREACHABLE` and the connection is closed. IPv4/IPv6 literal `CONNECT` requests are also rejected. The proxy cannot be used as a general-purpose internet proxy.

---

## Node Spawner Security

### Job Objects + AppContainer

Each node process runs inside a **Windows Job Object** with configurable CPU, memory, and
lifetime limits. Optional **AppContainer** sandboxing further restricts filesystem access,
registry access, and network egress at the kernel level — without requiring admin elevation.

### Command-Line Argument Quoting

`quote_arg` escapes arguments per the MSVC `CommandLineToArgvW` spec, including
**trailing backslash sequences** that precede the closing double-quote character. This
prevents argument injection for node executable paths or arguments that contain `\"` or
end in `\`.

---

## Audit History

VeloceNetwork has undergone three structured security audits across the v0.7–v0.9 release
cycle. All findings have been remediated.

### Audit 1 — v0.7.0 (9 findings)

Scope: IPC security, mesh handshake, DNS/SOCKS5 network surface, policy engine identity model.

| ID | Severity | Description |
|----|----------|-------------|
| C1 | Critical | TOCTOU — kernel-verified exe path replaces client-declared app name |
| C2 | Critical | Server-authoritative capability grant computed once at handshake |
| H1 | High | Pre-auth state machine — non-Handshake messages before auth drop connection |
| H2 | High | DNS bound to `127.0.0.1` instead of `0.0.0.0` |
| H3 | High | DNS upstream response source validated |
| H4 | High | Noise handshake 10-second read timeout on both initiator and responder |
| M1 | Medium | SOCKS5 scoped to `.vln`/`.veloce` destinations only |
| M2 | Medium | Registry size guards use `anyhow::ensure!` instead of `assert!` |
| M3 | Medium | `VELOCE_SKIP_PSK=1` blocked when running as SYSTEM account |

### Audit 2 — v0.8.0 (7 findings)

Scope: IPC capability model completeness, node spawner, DNS transaction integrity, gossip
timestamp integrity, peer message size bounds.

| ID | Severity | Description |
|----|----------|-------------|
| N1 | High | Missing `require_cap` on `NetUnregisterHost`, `MeshConnect`, `MeshDisconnect`, `PolicyReload` |
| N2 | High | `quote_arg` argument injection via trailing backslash |
| N3 | High | DNS `forward_query` transaction ID validation |
| N4 | High | Gossip LWW timestamp clock-skew guard (±5 min window) |
| N5 | Medium | Peer JSON `MAX_PEER_MSG_BYTES` cap + `RegistrySync` entry limit |
| N6 | Medium | DNS `qdcount` capped at 5 before `Vec::with_capacity` |
| N7 | Medium | DNS forward socket rebound to `127.0.0.1:0` |

### Audit 3 — v0.9.0 (2 findings + hardening features)

Scope: STUN client response validation; gossip ownership; join code replay/expiry.

| ID | Severity | Description |
|----|----------|-------------|
| N8 | Low | STUN response source address validated |
| N9 | Low | STUN RFC 5389 magic cookie validated |

Additional hardening shipped alongside the audit findings:
- **VM3 join codes** — TTL-limited and one-time-use join codes with nonce replay blacklist
- **Gossip ownership tracking** — hostname origin map prevents peer hostname squatting
- **Periodic gossip re-sync** — closes convergence gaps after transient failures

### Audit 4 — v3.5.1 (Comprehensive Security Audit)

Scope: Linux secrets KDF, gossip asymmetry, memory bounds, ingress parsing, RBAC, path traversal.

| ID | Severity | Description |
|----|----------|-------------|
| SEC-01 | Critical | Linux secrets keyring upgraded from XOR-fold to SHA-256 KDF |
| SEC-02 | High | Gossip past clock skew tightened from 24h to ±5 minutes |
| SEC-03 | High | VM3 one-time nonce storage bounded with timestamp-based sliding-window eviction |
| SEC-04 | Medium | Ingress HTTP `Host` header parsing made case-insensitive and robust |
| SEC-05 | Medium | Added `HubManage` and `MeshKvManage` to policy engine `ALL` capability grant list |
| SEC-06 | Low | CI/CD GitHub Actions pinned to immutable commit SHAs |
| SEC-07 | Low | `Makefile` release targets updated to generate and push signed git tags |
| SEC-08 | Low | `QueryNodes`, `QueryNodeStatus`, and `QueryNodeResources` require `RegistryRead` capability |
| SEC-09 | Info | Volume names validated against path traversal (`..`, `/`, `\`) |

---

## Secrets Threat Model & Environment Variables

Secrets managed by `veloce-core` are decrypted in memory and passed to child container processes via environment variables (`VELOCE_SECRET_{NAME}`).
- **Access Scope**: Environment variables are accessible to processes running as the same OS user (via `/proc/{pid}/environ` on Linux or PEB on Windows).
- **Isolation**: Workloads running in separate user accounts or Windows AppContainers cannot read environment variables across process boundaries.

---

## Cryptographic Primitives

| Use | Algorithm | Implementation |
|---|---|---|
| Mesh authenticated key exchange | Noise_IK_25519_ChaChaPoly_BLAKE2s | `snow` crate |
| Static identity keys | x25519 Diffie-Hellman | `x25519-dalek` crate |
| Identity key signing (peer identity) | Ed25519 | `ed25519-dalek` crate |
| Linux Secrets Encryption | XChaCha20-Poly1305 + SHA-256 KDF | `chacha20poly1305` crate + FIPS 180-4 |
| Windows Secrets Encryption | DPAPI (User & System scopes) | Win32 CryptProtectData |
| IPC session PSK | 32-byte `OsRng` random | Rust `rand` / Windows `CryptGenRandom` |
| Join code nonce (VM3) | 16-byte `OsRng` random | Rust `rand` |
| WebSocket Telemetry Upgrade | SHA-1 + Base64 (RFC 6455) | FIPS 180-1 SHA-1 |

All cryptographic operations use standard algorithms. VeloceNetwork does not
use unvetted or homebrew cryptographic primitives.

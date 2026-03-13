# VeloceNetwork

**VeloceNetwork** is a Windows-native runtime platform for launching, managing, and privately networking isolated application nodes — all without kernel drivers, VPNs, or elevated privileges beyond a single background service.

---

## What Is VeloceNetwork?

VeloceNetwork is a lightweight orchestration layer that runs on any Windows machine. It acts as a local control plane: your applications connect to it via a named-pipe SDK, request compute nodes, and communicate with each other over a private virtual namespace.

With v0.4, the mesh extends transparently across machines — two `veloce-core` instances exchange a join code, perform a Noise_IK handshake, and each other's `.vln` hostnames become locally resolvable on both sides. No VPN client, no admin elevation, no manual port rules.

With v0.5, the mesh reaches across NAT and the internet automatically: STUN discovers each machine's WAN IP at startup, the join code is upgraded to a dual-address VM2 format, and a declarative TOML policy engine controls which applications may request which capabilities and which peer-gossiped hostnames are installed locally.

With v0.6, the dashboard is rebuilt from scratch in **Svelte 5** with raw **Canvas 2D** rendering. Every Noise tunnel and every `.vln` hostname is now instrumented with live byte counters; the dashboard visualises traffic as an interactive drag-and-drop topology, a 60-cell heatmap per peer, and inline sparklines per node row.

With v0.7, the first of three structured security audits lands: nine findings (two critical, three high, three medium) are remediated across IPC, DNS, mesh, and SOCKS5 — no new features, all hardening.

With v0.8, the second audit closes seven more findings: missing capability checks on four IPC handlers, a command-line argument injection in the node spawner, DNS transaction-ID spoofing, gossip timestamp manipulation, peer message size exhaustion, DNS allocation amplification, and a DNS forward socket interface leak.

With v0.9, the third and final pre-1.0 audit lands alongside a set of mesh improvements: **VM3 join codes** with per-code TTL and one-time-use enforcement; **gossip ownership tracking** that prevents a peer from quietly overwriting hostnames it did not originate; a **periodic gossip re-sync ticker** (60 s) for resilience after transient disconnects; and three new `veloce-run mesh` diagnostic subcommands (`status`, `diagnose`, `ping`). The platform is now audited, hardened, and feature-complete ahead of v1.0.

Think of it as a stripped-down version of Kubernetes Service Mesh ideas, designed for desktop environments, developer tooling, and lightweight commercial applications rather than cloud infrastructure.

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                      Your Application                   │
│   veloce-sdk (Rust) ──or── veloce_sdk.dll (C FFI)       │
│   veloce-run (CLI)                                      │
└────────────────────┬────────────────────────────────────┘
                     │  Named pipe  \\.\pipe\VeloceCore
                     ▼
┌─────────────────────────────────────────────────────────┐
│                      VeloceCore                         │
│  • Session auth (SID ACL + OsRng PSK)                   │
│  • Node lifecycle (spawn / kill / monitor)              │
│  • Job Objects + AppContainer (CPU / memory / sandbox)  │
│  • Policy Engine (RBAC + mesh ACLs, TOML hot-reload)    │
│  • Shared mmap registry (fast key-value store)          │
│  • VeloceNet + Mesh integration                         │
└──────────┬──────────────────────┬───────────────────────┘
           │                      │
           ▼                      ▼
  ┌─────────────────┐   ┌──────────────────────────────┐
  │  Node Processes │   │     VeloceNet                │
  │  (Job Objects / │   │  DNS  :5354  (*.vln)         │
  │  AppContainer)  │   │  SOCKS5  :1055               │
  └─────────────────┘   │  Mesh TCP  :7474  ◄──────────┼── Remote machines
                        └──────────────────────────────┘
```

| Crate | Role |
|---|---|
| `veloce-core` | Background Windows service — single source of truth |
| `veloce-ipc` | Wire protocol (framing, message types, codec) shared by all components |
| `veloce-net` | Userspace DNS resolver and SOCKS5 proxy for the `*.vln` namespace |
| `veloce-mesh` | Noise_IK P2P mesh — encrypted tunnels, `.vln` gossip, STUN WAN discovery |
| `veloce-sdk` | Async Rust client + C FFI layer for sideloaded apps |
| `apps/dashboard` | Tauri 2 desktop GUI — Svelte 5 + Canvas 2D; nodes, templates, log viewer, resource sparklines, mesh UI, topology canvas, traffic heatmap |
| `apps/installer` | Glassmorphic 5-step Tauri installer |
| `apps/veloce-run` | CLI launcher — wraps any exe into the mesh with full flag set |

---

## Core Features

### Node Management
- Spawn isolated child processes as **Windows Job Objects** — each node is sandboxed from interfering with others
- Set per-node resource limits: CPU %, working-set memory cap, max wall-clock lifetime
- Real-time health monitoring: crash detection, memory threshold alerts, CPU throttle events
- Push-event subscriptions so clients receive live `Started`, `Exited`, `Crashed` notifications
- `auto_kill` flag — nodes can be tied to their client's lifetime, cleaning up automatically on disconnect

### VeloceNet — Private `.vln` Namespace
- Register any hostname like `myapp.vln` and map it to a node's local TCP port
- Built-in **DNS server** (UDP :5354) that resolves `*.vln` / `*.veloce` queries internally and forwards everything else to the system resolver
- Built-in **SOCKS5 proxy** (:1055) that routes `.vln` / `.veloce` traffic locally — scoped to the private TLD only; no kernel modules, no TAP adapters, no admin required
- TTL-based registration expiry with automatic garbage collection
- Apps set `VELOCE_DNS` and `VELOCE_SOCKS` environment variables and get transparent `.vln` routing

### Shared Registry
- Fast memory-mapped key-value store scoped per session
- Any authorised client can read or write — suitable for inter-node coordination, feature flags, and configuration

### Node Templates
- Save any node configuration (executable, args, resource limits, hostname, restart policy) as a named template
- Spawn from a template with a single command — no need to repeat flags every time
- Templates stored in the shared mmap registry; visible to all connected clients

### Resource Monitoring
- Live CPU% delta (utilisation since last poll) displayed per node in the Dashboard
- Peak memory (working-set MB) tracked and displayed without a separate monitoring agent
- Resource columns update in real time as nodes run

### Log Streaming
- `stdout` and `stderr` of every node captured and streamed over the IPC channel
- Dashboard log viewer shows live output; `veloce-run --watch` streams directly to the terminal
- Log chunks framed with node ID and stream tag — multiple subscribers can attach simultaneously

### AppContainer Isolation
- Optional per-node **Windows AppContainer** kernel sandbox — tighter than Job Objects alone
- Restricts filesystem access, registry access, and network egress without admin elevation
- Enabled per spawn via `use_appcontainer: true`; transparent to the node process itself

### veloce-run CLI
- Zero-friction wrapper: `veloce-run -- myapp.exe` registers the process as a mesh node instantly
- Full flag set: `--name`, `--hostname`, `--port`, `--cpu`, `--mem`, `--restarts`, `--watch`, `--detach`
- `--watch` streams live stdout/stderr to the terminal; `--detach` prints the node ID and exits
- `mesh` subcommand group for P2P mesh management:
  - `mesh identity [--ttl <mins>] [--one-time]` — print join code (VM2 or VM3)
  - `mesh join <code>` — connect to a peer via join code
  - `mesh peers` — list connected peers
  - `mesh leave <peer-id>` — disconnect a peer
  - `mesh status` — connected peers with latency and remote host count
  - `mesh diagnose` — connectivity health report (listen port, WAN address, peer states)
  - `mesh ping <peer-id>` — measure round-trip latency to a peer

### Multi-Machine VeloceNet (v0.4+)
- Two machines share a **join code** (one command each) to establish an encrypted P2P tunnel
- Crypto: **Noise_IK_25519_ChaChaPoly_BLAKE2s** — same algorithm as WireGuard, pure Rust, zero-admin
- Each machine's `.vln` hosts are gossiped to the peer via LWW (last-write-wins) CRDT protocol
- Remote `.vln` hosts appear **locally resolvable** — DNS and SOCKS5 require no changes
- Transparent TCP forwarder: traffic to a remote `.vln` host is silently tunnelled through the Noise channel
- Peer identities derived from x25519 static keys; persisted across restarts as `veloce-identity.key`
- **v0.5 — STUN WAN Mesh**: at startup, VeloceCore probes a STUN server to discover the machine's external IP; the join code is upgraded to **VM2** format (dual LAN + WAN addresses); `connect_to_peer()` races all addresses with a 250 ms stagger so NAT traversal is automatic
- **v0.9 — VM3 Join Codes**: the `--ttl <minutes>` flag issues a code that expires automatically; `--one-time` issues a code that is invalidated after the first successful use — one-time nonces are tracked in a replay blacklist on the receiving side

### Policy Engine (v0.5+)
- Declarative **TOML policy file** (`veloce-policy.toml`) — absent = allow-all, fully backward compatible
- **Tier 1 — Process RBAC**: per-app `allow`/`deny` lists for capabilities (`SpawnNodes`, `KillNodes`, `NetRegister`, …)
  - `exe` field: matched against the **kernel-verified Win32 image path** — cannot be spoofed by the client process
  - `app` field: legacy glob match against the client-declared name — kept for backward compatibility
- **Tier 2 — Mesh ACLs**: filter which peer-gossiped `.vln` hostnames are installed as local forwarders, optionally scoped by source peer
- Glob patterns: `"*"` (any) and `"*.suffix"` supported in both exe names/paths and hostnames
- Hot-reloadable at runtime via `veloce-run policy reload` — no service restart required
- `veloce-run policy show` prints a formatted table of all active rules

### Security

#### Foundational (v0.1–v0.5)
- **SID-based pipe ACL**: the named pipe is restricted to the owning Windows user at the kernel level — cross-user connections are rejected before any data is read
- **OsRng PSK**: VeloceCore generates a fresh 32-byte random key (full 256-bit entropy) at every startup; invalidates connections from prior sessions automatically
- **Noise_IK authentication**: mesh peers mutually authenticate via static x25519 key pairs — no certificates, no CA
- **DNS compression loop protection**: hand-rolled DNS parser enforces max 10 pointer jumps (DoS fix)
- **Identity key file ACL**: `veloce-identity.key` is set read-only and owner-only at creation
- **Capability negotiation**: clients declare exactly which operations they need (`SpawnNodes`, `KillNodes`, `RegistryRead`, `NetRegister`, …) and Core enforces the grant
- **Policy Engine**: declarative TOML RBAC enforced server-side — blocked capabilities return `PolicyDenied (11)` before any action is taken; mesh ACLs prevent untrusted peers from installing forwarders for sensitive hostnames

#### Security Audit 1 — v0.7 (9 findings remediated)
- **Kernel-verified exe-path RBAC** (C1): `assert_client_is_owner` now returns the Win32 image path resolved via `QueryFullProcessImageNameW` while the process handle is open. Policy rules keyed on `exe` use this path — clients cannot spoof their identity via a declared `app_name`.
- **Server-authoritative capability grant** (C2): `PolicyEngine::compute_max_caps(exe_path)` computes the full allowed capability set at handshake time. Client requests are intersected with the server grant — no per-handler re-checks required.
- **Pre-authentication state machine** (H1): non-`Handshake` messages received before the client completes authentication immediately drop the connection with no error response.
- **DNS localhost-only bind** (H2): the DNS server (`:5354`) now binds exclusively to `127.0.0.1` rather than `0.0.0.0`.
- **DNS upstream response validation** (H3): `forward_query` validates that DNS replies come from the configured upstream address; spoofed replies from unexpected sources are discarded.
- **Noise handshake timeout** (H4): both `initiator_handshake` and `responder_handshake` apply a 10-second `tokio::time::timeout` to all read steps — stalled TCP connections no longer hold Tokio task slots indefinitely.
- **SOCKS5 VLN-only scope** (M1): the SOCKS5 proxy rejects `CONNECT` requests to any destination outside `.vln` / `.veloce` with `REP_UNREACHABLE`.
- **Registry size validation** (M2): oversized registry keys and values return `InvalidMessage` to the client instead of panicking the service.
- **`VELOCE_SKIP_PSK` production guard** (M3): `VELOCE_SKIP_PSK=1` is silently ignored and logged at `error` level when the service runs as the SYSTEM account (`S-1-5-18`).

#### Security Audit 2 — v0.8 (7 findings remediated)
- **Missing IPC capability checks** (N1): `NetUnregisterHost`, `MeshConnect`, `MeshDisconnect`, and `PolicyReload` were open to any authenticated client. Two new capability variants (`MeshManage`, `PolicyAdmin`) were added; all four handlers now call `require_cap`.
- **Command-line argument injection** (N2): `quote_arg` did not escape trailing backslashes per MSVC `CommandLineToArgvW` rules. A node path ending in `\` could inject extra arguments into the spawned process command line.
- **DNS transaction-ID spoofing** (N3): `forward_query` now validates that the upstream DNS response transaction ID matches the outgoing query before forwarding, preventing local cache poisoning.
- **Gossip LWW timestamp manipulation** (N4): peer-supplied gossip timestamps are validated against the local clock (±5 minute window). A peer setting `ts = u64::MAX` can no longer permanently win all future LWW comparisons and hijack any `.vln` hostname.
- **Peer JSON message size cap** (N5): a `MAX_PEER_MSG_BYTES` guard is checked before `serde_json::from_slice`; `RegistrySync` entry lists are truncated to 1,000 entries per message.
- **DNS `qdcount` allocation amplification** (N6): `Vec::with_capacity(qdcount)` is capped at 5, preventing repeated large heap allocations from crafted local DNS packets.
- **DNS forward socket interface leak** (N7): the ephemeral UDP socket used for upstream forwarding is now bound to `127.0.0.1:0` instead of `0.0.0.0:0`.

#### Security Audit 3 + Mesh Hardening — v0.9
- **VM3 join codes with TTL and one-time enforcement** (S1): join codes carry a creation timestamp, a TTL in minutes, and a one-time-use bit. Expired codes are rejected at connect time; one-time nonces are recorded in a `HashSet` blacklist — replay attempts are rejected even if the code is still within its TTL window.
- **Gossip ownership tracking** (S2): `MeshState` maintains a per-hostname origin map. If a peer tries to gossip a hostname it did not originally register, the entry is discarded and logged at `warn` level, preventing silent hostname squatting by a connected peer.
- **Periodic gossip re-sync** (O1): every 60 seconds each `PeerConnection` re-broadcasts its full local `NetRegistry` to its peer, ensuring hostname state converges automatically after transient failures without waiting for the next organic gossip event.

### Dashboard (v0.6)
- Tauri 2 desktop app (Windows, ships as a lightweight installer) — rebuilt in **Svelte 5** with **Canvas 2D** (zero new runtime JS dependencies)
- **Nodes tab**: live node table with inline 80×22 px CPU% sparklines per row; click to expand a detail panel with 120-point CPU and memory history graphs
- **Templates tab**: save and launch named node configurations with one click
- **VeloceNet tab**: hostname registration/unregistration, mesh join-code display and copy, peer connect, connected peers table, collapsible Policy Engine panel with App Rules and Mesh ACL tables
- **Logs tab**: live stdout/stderr viewer with search filter, stream toggles, timestamp toggle, auto-scroll, 5 000-line cap
- **Topology tab**: drag-and-drop Canvas 2D topology canvas; edge width and colour indicate live traffic rate (green→red gradient); 60-cell per-peer traffic heatmap (2-minute window); live byte-counter tables for tunnels and `.vln` hosts
- Traffic snapshots pushed from backend every 2 s via Tauri event (`"traffic-update"`) — no frontend polling
- Auto-pings Core every 10 s to keep connection status accurate

### SDK
- Async Rust client (`VeloceClient`) — ergonomic `.connect()`, `.spawn_node()`, `.kill_node()`, `.register_host()`, mesh methods, …
- C FFI (`veloce_sdk.dll` + `veloce_sdk.h`) — drop-in for any language with a C ABI: Python, C++, Go, Node, Delphi, etc.
- `veloce_poll_event()` non-blocking event pump for FFI consumers that can't use async runtimes

---

## Use Cases

### Commercial / Enterprise
- **Microservice dev environments** — spin up a set of named services locally, wire them together over `.vln` hostnames, and tear down cleanly without port conflicts or leftover processes
- **Background worker orchestration** — run and monitor a fleet of worker processes with hard resource limits; restart on crash; stream events to a supervisor
- **Sandboxed plugin hosts** — load third-party plugins as isolated Job Object / AppContainer nodes; a crashed plugin cannot take down the host
- **Multi-machine dev cluster** — share `.vln` namespaces across developer VMs over the encrypted mesh; no VPN required

### Personal / Developer
- **Local multi-service projects** — run several microservices in development without Docker or WSL, with automatic cleanup when your IDE closes
- **Game server management** — host multiple game server instances with memory caps, monitor health, and kill/restart without a terminal
- **Automation pipelines** — chain scripts and programs as nodes, subscribe to their exit events, trigger next steps programmatically

### Recreational
- **Hobby clusters** — experiment with node orchestration and P2P mesh concepts on desktop hardware
- **Modding platforms** — host game mods or extensions as isolated nodes with defined resource budgets
- **Stream/broadcast tooling** — manage a set of capture, encoding, and overlay processes with coordinated start/stop and live status monitoring

---

## Roadmap

| Feature | Status |
|---|---|
| Windows Named-Pipe IPC + SID/PSK security | ✅ v0.1 |
| VeloceNet DNS + SOCKS5 | ✅ v0.1 |
| Job Objects (CPU / memory / lifetime limits) | ✅ v0.1 |
| Health policies + log streaming | ✅ v0.2 |
| Node templates | ✅ v0.2 |
| Resource usage display (CPU% + peak memory) | ✅ v0.2 |
| Glassmorphic Tauri installer | ✅ v0.2 |
| veloce-run CLI + AppContainer isolation | ✅ v0.3 |
| Multi-Machine VeloceNet (Noise_IK P2P mesh) | ✅ v0.4 |
| Policy Engine (process RBAC + mesh ACLs) | ✅ v0.5 |
| STUN WAN mesh + VM2 join codes | ✅ v0.5 |
| Dashboard v2 (Svelte 5 + Canvas 2D; topology, heatmap, sparklines) | ✅ v0.6 |
| Backend traffic instrumentation (per-tunnel + per-host byte counters) | ✅ v0.6 |
| **Security Audit 1 of 3** — IPC hardening, mesh hardening, DNS/SOCKS5 scope restriction | ✅ v0.7 |
| **Security Audit 2 of 3** — IPC capability enforcement, arg injection fix, DNS/gossip hardening | ✅ v0.8 |
| **Security Audit 3 of 3** — VM3 join codes, gossip ownership, re-sync, mesh diagnostic CLI | ✅ v0.9 |
| WireGuard-NT kernel driver (perf upgrade) | 📋 v1.0 |
| NRPT `.vln` routing (system-wide DNS without `VELOCE_DNS`) | 📋 v1.0 |
| Signed installer with auto-update (winget/scoop) | 📋 v1.0 |
| Linux port (cgroups v2 + Unix sockets) | 📋 v2.0 |
| Python / Node.js / Go SDK bindings | 📋 v2.0 |

### v0.7 — Security Audit 1 of 3 ✅

v0.7 is the first of three dedicated security audit releases. No new user-facing features land.
Nine findings were identified and resolved:

- **C1** — Kernel-verified exe-path RBAC replaces client-declared app name in policy engine
- **C2** — Server-authoritative capability grant computed once at handshake; per-handler re-checks removed
- **H1** — Pre-auth state machine: non-Handshake messages before auth drop the connection immediately
- **H2** — DNS server bound to `127.0.0.1` only (was `0.0.0.0`)
- **H3** — DNS upstream response source validated; spoofed replies discarded
- **H4** — Noise handshake 10-second read timeout on both initiator and responder sides
- **M1** — SOCKS5 proxy scoped to `.vln` / `.veloce` destinations only
- **M2** — Registry oversized-input panics converted to graceful `InvalidMessage` errors
- **M3** — `VELOCE_SKIP_PSK=1` blocked when service runs as SYSTEM account

### v0.8 — Security Audit 2 of 3 ✅

v0.8 is the second structured security audit. Seven findings (N1–N7) identified and resolved:

- **N1** — Missing `require_cap` gates on `NetUnregisterHost`, `MeshConnect`, `MeshDisconnect`, `PolicyReload`; new `MeshManage` and `PolicyAdmin` capabilities added
- **N2** — `quote_arg` now escapes trailing backslashes per MSVC `CommandLineToArgvW` spec — argument injection fixed
- **N3** — `forward_query` validates DNS response transaction ID against the outgoing query
- **N4** — Gossip LWW timestamps validated against local clock (±5 min skew window); future-dated entries discarded
- **N5** — `MAX_PEER_MSG_BYTES` guard before JSON deserialization; `RegistrySync` entry list capped at 1,000
- **N6** — `Vec::with_capacity(qdcount)` capped at 5 to prevent DNS allocation amplification
- **N7** — DNS forward socket bound to `127.0.0.1:0` instead of `0.0.0.0:0`

### v0.9 — Security Audit 3 of 3 + Mesh Improvements ✅

v0.9 is the third and final structured audit before v1.0, combined with targeted mesh
improvements that close gaps identified during the audit cycle:

- **VM3 join codes** — new join code format adds creation timestamp, TTL (minutes), and one-time-use flag; expired codes rejected; one-time nonces tracked in a replay blacklist
- **Gossip ownership tracking** — each gossiped hostname is bound to the peer that first registered it; ownership conflicts are logged at `warn` and the new entry is discarded
- **Periodic gossip re-sync** — every 60 s each peer re-broadcasts its full local `NetRegistry`, ensuring convergence after transient failures without waiting for organic gossip
- **New `veloce-run mesh` subcommands** — `mesh status` (connected peers + latency), `mesh diagnose` (connectivity health report), `mesh ping <peer-id>` (latency round-trip)
- **`MeshPingPeer` / `MeshPingResult` IPC messages** (0x58 / 0x59) — SDK-accessible ping round-trip for peer latency measurement

After v0.9 ships stable, **v1.0** introduces the WireGuard-NT kernel driver for
hardware-offloaded throughput and a signed installer with automatic update delivery.

### Future Features — Deferred to v1.0 or Later

The following items were evaluated and explicitly deferred. They are not in scope for any
patch release prior to v1.0.

| Item | Reason Deferred |
|------|-----------------|
| NRPT `.vln` routing (system-wide DNS) | Requires UAC elevation; scoped to installer track |
| WireGuard-NT kernel driver | Major platform work, explicitly v1.0 |
| Bincode frame versioning / migration | Requires protocol versioning framework; breaking change |
| Dashboard force layout + minimap | Frontend-only; can ship independently |
| `/metrics` Prometheus endpoint | New infra dependency; better as a separate crate |
| Integration test suite (mesh recovery) | Build infra work; long tail |
| `veloce-examples` repo + `veloce-compose.toml` | Documentation/tooling, not core |
| C FFI / Python SDK improvements | SDK scope, separate track |
| winget / scoop installer package | v1.0 release track |
| `--perf-mode` raw socket bypass | Risky footgun; deferred until profiling confirms need |

---

## Getting Started

### Prerequisites
- Windows 10/11 (x86-64)
- [Rust toolchain](https://rustup.rs) (`stable`, `x86_64-pc-windows-msvc` target)
- Node.js ≥ 18 + npm (dashboard only)
- [Tauri CLI v2](https://tauri.app) (dashboard only)

### Build

```powershell
# Build everything
cargo build --release

# Build only the core service
cargo build -p veloce-core --release

# Build the dashboard (dev mode with hot-reload)
cd apps/dashboard
npm install
cargo tauri dev
```

### Run VeloceCore (foreground / dev mode)

```powershell
.\target\release\veloce-core.exe run
```

### Install as a Windows Service

```powershell
# Run once in an elevated terminal
.\target\release\veloce-core.exe install
.\target\release\veloce-core.exe start
```

### Launch nodes with veloce-run

```powershell
# Wrap any exe into the mesh and stream its logs live
veloce-run --name worker --cpu 25 --mem 512 -- worker.exe

# Register a .vln hostname and detach (prints node ID)
veloce-run --hostname api.vln --port 3000 --detach -- node server.js

# Stream stdout/stderr of a long-running process
veloce-run --watch -- ping -t 127.0.0.1
```

### Connect two machines via P2P mesh (v0.4+)

```powershell
# Machine A — print the join code (VM2 by default; VM3 with TTL or one-time flag)
veloce-run mesh identity
# VM2:BBBB...==                       ← VM2 if internet available (LAN + WAN)
# machine_id: xxxxxxxx-...
# listening on port: 7474
# wan: 203.0.113.45  (via stun.l.google.com)

# Optionally issue a time-limited or one-time-use VM3 code
veloce-run mesh identity --ttl 30              # expires in 30 minutes
veloce-run mesh identity --ttl 60 --one-time   # single-use, expires in 1 hour

# Machine B — connect using the join code from Machine A (works across NAT)
veloce-run mesh join "VM2:BBBB...=="
# ✓ connected to DESKTOP-A (peer_id=abc-123...)

# Verify both sides see each other
veloce-run mesh peers

# Check latency and connection health
veloce-run mesh status
veloce-run mesh ping abc-123...

# Machine A registers a service
veloce-run --hostname api.vln --port 8080 --detach -- node server.js

# Machine B resolves it transparently — no config changes required
curl --proxy socks5://127.0.0.1:1055 http://api.vln/health
# → 200 OK  (traffic routed through Noise_IK tunnel to Machine A)
```

### Manage access with the Policy Engine (v0.5+)

```powershell
# View the active policy rules (default: allow-all when no file is present)
veloce-run policy show

# Create a policy file at %ProgramData%\VeloceSolutions\VeloceCore\veloce-policy.toml
# Example: block an untrusted sideloaded agent from spawning nodes
#
# [[rules]]
# app  = "untrusted-agent"
# deny = ["SpawnNodes", "KillNodes"]

# Hot-reload without restarting the service
veloce-run policy reload
```

### Connect via the SDK

```rust
use veloce_sdk::VeloceClient;
use veloce_ipc::message::Capability;

let mut client = VeloceClient::connect(
    "my-app",
    env!("CARGO_PKG_VERSION"),
    vec![Capability::SpawnNodes, Capability::NetRegister],
).await?;

let node = client.spawn_node("worker", "worker.exe", &[]).await?;
println!("Node {} running as PID {}", node.node_id, node.pid);
```

---

## Wire Protocol

Frames are fixed-header + variable payload:

```
Offset  Size  Field
──────  ────  ──────────────────────────────────────────────────────────────
0       4     Magic: 0x56454C43 ("VELC")
4       1     Version (0x01)
5       1     MessageType discriminant
6       2     Flags (u16 LE): COMPRESSED | EXPECTS_ACK | PUSH | URGENT
8       4     PayloadLen (u32 LE, max 4 MiB)
12      N     Payload: bincode-encoded Envelope { correlation_id, body }
```

All message types are defined in `crates/veloce-ipc/src/message.rs` and are assigned stable numeric discriminants — existing values are never renumbered.

---

## License

Proprietary — © VeloceSolutions. All rights reserved.

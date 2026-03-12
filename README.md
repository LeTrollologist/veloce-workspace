# VeloceNetwork

**VeloceNetwork** is a Windows-native runtime platform for launching, managing, and privately networking isolated application nodes — all without kernel drivers, VPNs, or elevated privileges beyond a single background service.

---

## What Is VeloceNetwork?

VeloceNetwork is a lightweight orchestration layer that runs on any Windows machine. It acts as a local control plane: your applications connect to it via a named-pipe SDK, request compute nodes, and communicate with each other over a private virtual namespace.

With v0.4, the mesh extends transparently across machines — two `veloce-core` instances exchange a join code, perform a Noise_IK handshake, and each other's `.vln` hostnames become locally resolvable on both sides. No VPN client, no admin elevation, no manual port rules.

With v0.5, the mesh reaches across NAT and the internet automatically: STUN discovers each machine's WAN IP at startup, the join code is upgraded to a dual-address VM2 format, and a declarative TOML policy engine controls which applications may request which capabilities and which peer-gossiped hostnames are installed locally.

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
| `apps/dashboard` | Tauri 2 desktop GUI — nodes, templates, log viewer, resource meters, mesh UI |
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
- Built-in **SOCKS5 proxy** (:1055) that routes `.vln` traffic locally — no kernel modules, no TAP adapters, no admin required
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
- `mesh` subcommand group for P2P mesh management (see Multi-Machine section below)

### Multi-Machine VeloceNet (v0.4+)
- Two machines share a **join code** (one command each) to establish an encrypted P2P tunnel
- Crypto: **Noise_IK_25519_ChaChaPoly_BLAKE2s** — same algorithm as WireGuard, pure Rust, zero-admin
- Each machine's `.vln` hosts are gossiped to the peer via LWW (last-write-wins) CRDT protocol
- Remote `.vln` hosts appear **locally resolvable** — DNS and SOCKS5 require no changes
- Transparent TCP forwarder: traffic to a remote `.vln` host is silently tunnelled through the Noise channel
- Peer identities derived from x25519 static keys; persisted across restarts as `veloce-identity.key`
- **v0.5 — STUN WAN Mesh**: at startup, VeloceCore probes a STUN server to discover the machine's external IP; the join code is upgraded to **VM2** format (dual LAN + WAN addresses); `connect_to_peer()` races all addresses with a 250 ms stagger so NAT traversal is automatic

### Policy Engine (v0.5)
- Declarative **TOML policy file** (`veloce-policy.toml`) — absent = allow-all, fully backward compatible
- **Tier 1 — Process RBAC**: per-app `allow`/`deny` lists for capabilities (`SpawnNodes`, `KillNodes`, `NetRegister`, …)
- **Tier 2 — Mesh ACLs**: filter which peer-gossiped `.vln` hostnames are installed as local forwarders, optionally scoped by source peer
- Glob patterns: `"*"` (any) and `"*.suffix"` supported in both app names and hostnames
- Hot-reloadable at runtime via `veloce-run policy reload` — no service restart required
- `veloce-run policy show` prints a formatted table of all active rules

### Security
- **SID-based pipe ACL**: the named pipe is restricted to the owning Windows user at the kernel level — cross-user connections are rejected before any data is read
- **OsRng PSK**: VeloceCore generates a fresh 32-byte random key (full 256-bit entropy) at every startup; invalidates connections from prior sessions automatically
- **Noise_IK authentication**: mesh peers mutually authenticate via static x25519 key pairs — no certificates, no CA
- **DNS compression loop protection**: hand-rolled DNS parser enforces max 10 pointer jumps (DoS fix)
- **Identity key file ACL**: `veloce-identity.key` is set read-only and owner-only at creation
- Capability negotiation: clients declare exactly which operations they need (`SpawnNodes`, `KillNodes`, `RegistryRead`, `NetRegister`, …) and Core enforces the grant
- **Policy Engine**: declarative TOML RBAC enforced server-side — blocked capabilities return `PolicyDenied (11)` before any action is taken; mesh ACLs prevent untrusted peers from installing forwarders for sensitive hostnames

### Dashboard
- Tauri 2 desktop app (Windows, ships as a lightweight installer)
- Dark-themed UI with live node table, per-node Kill button, resource meters, log viewer
- Templates tab for saving and spawning named configurations
- VeloceNet tab with hostname registration, mesh join-code input, and connected peers table
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
| Dashboard v2 (topology canvas, heatmap, log viewer) | 📋 v0.6 |
| WireGuard-NT kernel driver (perf) | 📋 v1.0 |
| Signed installer with auto-update | 📋 v1.0 |
| Linux port (cgroups v2 + Unix sockets) | 📋 v2.0 |
| Python / Node.js / Go SDK bindings | 📋 v2.0 |

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
# Machine A — print the join code
veloce-run mesh identity
# VM2:BBBB...==                       ← VM2 if internet available (LAN + WAN)
# machine_id: xxxxxxxx-...
# listening on port: 7474
# wan: 203.0.113.45  (via stun.l.google.com)

# Machine B — connect using the join code from Machine A (works across NAT)
veloce-run mesh join "VM2:BBBB...=="
# ✓ connected to DESKTOP-A (peer_id=abc-123...)

# Verify both sides see each other
veloce-run mesh peers

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

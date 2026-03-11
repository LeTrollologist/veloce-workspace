# VeloceNetwork

**VeloceNetwork** is a Windows-native runtime platform for launching, managing, and privately networking isolated application nodes — all without kernel drivers, VPNs, or elevated privileges beyond a single background service.

---

## What Is VeloceNetwork?

VeloceNetwork is a lightweight orchestration layer that runs on any Windows machine. It acts as a local control plane: your applications connect to it via a named-pipe SDK, request compute nodes, and communicate with each other over a private virtual namespace — all confined to the user's own machine and session.

Think of it as a stripped-down, single-machine version of the ideas behind Kubernetes or Service Mesh, but designed for desktop environments, developer tooling, and lightweight commercial applications rather than cloud infrastructure.

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                      Your Application                   │
│   veloce-sdk (Rust) ──or── veloce_sdk.dll (C FFI)       │
└────────────────────┬────────────────────────────────────┘
                     │  Named pipe  \\.\pipe\VeloceCore
                     ▼
┌─────────────────────────────────────────────────────────┐
│                      VeloceCore                         │
│  • Session auth (SID ACL + PSK)                         │
│  • Node lifecycle (spawn / kill / monitor)              │
│  • Job Objects (CPU / memory / lifetime limits)         │
│  • Shared mmap registry (fast key-value store)          │
│  • VeloceNet integration                                │
└──────────┬──────────────────────┬───────────────────────┘
           │                      │
           ▼                      ▼
  ┌─────────────────┐   ┌──────────────────────┐
  │  Node Processes │   │     VeloceNet         │
  │  (Job Objects)  │   │  DNS  :5354  (*.vln)  │
  │                 │   │  SOCKS5  :1055        │
  └─────────────────┘   └──────────────────────┘
```

| Crate | Role |
|---|---|
| `veloce-core` | Background Windows service — the single source of truth |
| `veloce-ipc` | Wire protocol (framing, message types, codec) shared by all components |
| `veloce-net` | Userspace DNS resolver and SOCKS5 proxy for the `*.vln` namespace |
| `veloce-sdk` | Rust async client + C FFI layer for sideloaded apps |
| `apps/dashboard` | Tauri 2 desktop GUI — connect, manage nodes, register hostnames |

---

## Core Features (v0.1)

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

### Security
- **SID-based pipe ACL**: the named pipe is restricted to the owning Windows user at the kernel level — cross-user connections are rejected before any data is read
- **Per-session PSK**: VeloceCore generates a fresh 32-byte random key at every startup, writes it to `%LOCALAPPDATA%\VeloceCore\session.key`, and rejects any client that cannot echo it — invalidating connections from prior sessions automatically
- Capability negotiation: clients declare exactly which operations they need (`SpawnNodes`, `KillNodes`, `RegistryRead`, `NetRegister`, …) and Core enforces the grant

### Dashboard
- Tauri 2 desktop app (Windows, ships as a lightweight `.exe` installer)
- Dark-themed UI with live node table, per-node Kill button, and a VeloceNet tab for hostname registration
- Auto-pings Core every 10 s to keep connection status accurate
- No web server required — the frontend runs fully embedded

### SDK
- Async Rust client (`VeloceClient`) — ergonomic `.connect()`, `.spawn_node()`, `.kill_node()`, `.register_host()`, …
- C FFI (`veloce_sdk.dll` + `veloce_sdk.h`) — drop-in for any language with a C ABI: Python, C++, Go, Node, Delphi, etc.
- `veloce_poll_event()` non-blocking event pump for FFI consumers that can't use async runtimes

---

## Use Cases

### Commercial / Enterprise
- **Microservice dev environments** — spin up a set of named services locally, wire them together over `.vln` hostnames, and tear down cleanly without port conflicts or leftover processes
- **Background worker orchestration** — run and monitor a fleet of worker processes with hard resource limits; restart on crash; stream events to a supervisor
- **Sandboxed plugin hosts** — load third-party plugins as isolated Job Object nodes; a crashed plugin cannot take down the host
- **Internal tooling middleware** — build internal desktop tools that talk to each other over a private namespace without exposing anything on the network

### Personal / Developer
- **Local multi-service projects** — run several microservices in development without Docker or WSL, with automatic cleanup when your IDE closes
- **Game server management** — host multiple game server instances with memory caps, monitor health, and kill/restart without a terminal
- **Automation pipelines** — chain scripts and programs as nodes, subscribe to their exit events, trigger next steps programmatically

### Recreational
- **Hobby clusters** — experiment with node orchestration concepts on a single desktop machine
- **Modding platforms** — host game mods or extensions as isolated nodes with defined resource budgets
- **Stream/broadcast tooling** — manage a set of capture, encoding, and overlay processes with coordinated start/stop and live status monitoring

---

## Roadmap

The following capabilities are planned or in active exploration:

| Feature | Description |
|---|---|
| **Multi-machine VeloceNet** | Extend the `.vln` namespace across machines on a LAN or WireGuard tunnel — no public IPs needed |
| **Node templates** | Define reusable node configurations (executable, limits, env, args) as named templates in the registry |
| **Health policies** | Automatic restart-on-crash with back-off, max-restart limits, and alerting hooks |
| **Log streaming** | Capture stdout/stderr from nodes and stream them to clients or to disk via the IPC channel |
| **Linux / macOS support** | Port `veloce-core` to Unix — replace named pipes with Unix domain sockets, Job Objects with cgroups/rlimits |
| **Node-to-node messaging** | Typed message bus between nodes brokered by Core — no shared memory or raw sockets required |
| **Dashboard v2** | Historical metrics, log viewer, drag-and-drop node wiring, per-node resource graphs |
| **Policy engine** | Declarative TOML/JSON policies: which apps can spawn which nodes, network rules, capability deny-lists |
| **Encrypted VeloceNet** | TLS/QUIC between `.vln` peers for multi-machine deployments |
| **SDK bindings** | Official Python, Node.js, and Go client libraries built on the C FFI |
| **Installer** | NSIS/WiX signed installer with auto-update, service installation, and dashboard shortcut |

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

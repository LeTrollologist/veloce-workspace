# VeloceNetwork (Linux) — Userspace Orchestration & Service Mesh

> **VeloceNetwork for Linux** is a lightweight, zero-kernel runtime for launching, managing, and privately networking isolated application nodes — built natively for Linux with `cgroups v2`, Unix domain sockets, and `systemd` integration.

[![Linux CI](https://github.com/LeTrollologist/veloce-workspace/actions/workflows/linux.yml/badge.svg)](https://github.com/LeTrollologist/veloce-workspace/actions/workflows/linux.yml)
[![Release](https://img.shields.io/github/v/release/LeTrollologist/veloce-workspace?label=version)](https://github.com/LeTrollologist/veloce-workspace/releases/tag/v3.0.0-linux)
[![License](https://img.shields.io/badge/license-proprietary-blue.svg)](LICENSE)

---

## What Is VeloceNetwork?

VeloceNetwork is a lightweight userspace orchestration layer designed for developer environments, local clusters, and desktop/server nodes. It acts as an unprivileged local control plane: your applications connect to it via a high-performance Unix domain socket SDK, request compute nodes with strict resource limits, and communicate over an isolated private `.vln` virtual namespace.

- **No Root Required:** Runs completely in userspace as a regular user service (`systemd --user`).
- **No Virtual Interfaces:** Built-in userspace DNS (`127.0.0.1:5354`) and SOCKS5 proxy (`127.0.0.1:1055`) — no TUN/TAP devices, no `iptables` modifications, and no root elevation.
- **WireGuard-Grade P2P Mesh:** Multi-machine mesh using pure Rust **Noise_IK_25519_ChaChaPoly_BLAKE2s** encryption with automated STUN NAT traversal.
- **Layer-7 HTTP Ingress:** Built-in reverse proxy (`127.0.0.1:8080`) routing HTTP requests by `Host:` header (`api.vln`) and path prefix (`/api`).
- **Distributed Control Plane:** Multi-node cluster coordination with term tracking and automatic replica scheduling.

---

## Architecture (Linux Native)

```
┌─────────────────────────────────────────────────────────┐
│                      Your Application                   │
│   veloce-sdk (Rust) ──or── libveloce_sdk.so (C FFI)     │
│   veloce-run (CLI)                                      │
└────────────────────┬────────────────────────────────────┘
                     │  Unix domain socket  $XDG_RUNTIME_DIR/veloce/core.sock
                     ▼
┌─────────────────────────────────────────────────────────┐
│                      VeloceCore                         │
│  • Session authentication (UID check + OsRng PSK)       │
│  • Node lifecycle (spawn / kill / signal / monitor)     │
│  • cgroups v2 resource limits (cpu.max / memory.max)   │
│  • Process groups for clean tree termination (SIGKILL) │
│  • Policy Engine (RBAC + mesh ACLs, TOML hot-reload)    │
│  • Shared mmap registry (fast key-value store)          │
│  • VeloceNet DNS / SOCKS5 / Ingress + Noise_IK Mesh    │
│  • systemd watchdog & sd_notify ready signalling        │
└──────────┬──────────────────────┬───────────────────────┘
           │                      │
           ▼                      ▼
  ┌─────────────────┐   ┌──────────────────────────────┐
  │  Node Processes │   │     VeloceNet                │
  │  (cgroups v2 /  │   │  DNS     :5354  (*.vln)      │
  │  Process Groups)│   │  SOCKS5  :1055               │
  │                 │   │  Ingress :8080  (HTTP L7)    │
  └─────────────────┘   │  Mesh TCP :7474 ◄────────────┼── Remote machines
                        └──────────────────────────────┘
```

| Component | Role |
|---|---|
| `veloce-core` | Background Linux daemon / systemd user service — single source of truth |
| `veloce-ipc` | Zero-copy binary wire protocol with fixed `VELC` framing shared across platforms |
| `veloce-net` | Userspace DNS resolver, SOCKS5 proxy, and Layer-7 HTTP Ingress reverse proxy |
| `veloce-mesh` | Noise_IK P2P mesh — encrypted tunnels, `.vln` gossip CRDT, STUN WAN discovery |
| `veloce-sdk` | Async Rust client + C shared library (`libveloce_sdk.so`) for any language with C ABI |
| `veloce-run` | CLI launcher, Compose engine, secrets vault, and mesh management tool |

---

## Core Features on Linux

### 1. Node Isolation via `cgroups v2` & Process Groups
- **Hard CPU limits:** Enforces `cpu.max` quotas dynamically without kernel modules.
- **Memory Ceilings:** Applies strict `memory.max` caps; kernel OOM signals are captured and reported cleanly.
- **Tree Kill Guarantees:** Nodes execute in dedicated process groups; stopping a node delivers `SIGTERM`/`SIGKILL` to the entire process tree, leaving zero orphaned processes.

### 2. VeloceNet Private `.vln` Namespace
- Register any hostname (e.g. `backend.vln`, `redis.vln`) mapped to a node's local TCP port.
- Built-in **DNS server** (UDP `127.0.0.1:5354`) resolving `.vln` queries and passing through system DNS queries.
- Built-in **SOCKS5 proxy** (TCP `127.0.0.1:1055`) routing `.vln` traffic locally.
- Applications set `export VELOCE_DNS=127.0.0.1:5354` and `export VELOCE_SOCKS=127.0.0.1:1055` for transparent routing.

### 3. Layer-7 HTTP Ingress Reverse Proxy (v2.1)
- Built-in async HTTP reverse proxy running on `127.0.0.1:8080` (`VLN_INGRESS_PORT`).
- Routes requests by `Host:` header (`api.vln`, `app.vln`) and path prefix (`/api`, `/v1`) to local backend ports.
- Supports longest-prefix matching and path stripping (`--strip-prefix`).
- Dynamically managed via `veloce-run ingress [add|rm|list]`.

### 4. Noise_IK Multi-Machine Mesh & STUN Discovery
- P2P encrypted tunnels using **Noise_IK_25519_ChaChaPoly_BLAKE2s**.
- Automated WAN IP discovery via STUN; dual-address VM2 join codes race LAN and WAN paths.
- **VM3 Join Codes:** Signed time-limited join codes with single-use anti-replay nonce tracking.
- Gossip ownership tracking prevents malicious peers from claiming hostnames they do not own.

### 5. Multi-Node Control Plane (v3.0)
- Distributed `ClusterCoordinator` managing cluster term numbers and leader election states (`Follower`, `Candidate`, `Leader`).
- Monotonic term validation over Noise_IK mesh connections preventing split-brain conditions.
- Deterministic multi-node replica allocation (`assign_replicas()`) distributing service instances across mesh peers.

### 6. Veloce Compose, Volumes, & Secrets
- Declarative `veloce-compose.yml` multi-service orchestration (`veloce-run up`, `down`, `ps`).
- Named persistent directories and host bind mounts stored across node restarts.
- Encrypted runtime secrets vault (`veloce-run secret set/rm/list`) injected at spawn.

---

## Quickstart & CLI Usage

### 1. Start VeloceCore

Run directly in a terminal:
```bash
veloce-core
```

Or as a `systemd` user service:
```bash
# Enable and start user service
systemctl --user enable --now veloce-core.service

# View live daemon logs
journalctl --user -u veloce-core -f
```

### 2. Launching Nodes with `veloce-run`

```bash
# Launch a background service with a .vln hostname and resource limits
veloce-run \
  --name web-app \
  --hostname web.vln \
  --port 3000 \
  --cpu 50 \
  --mem 256 \
  --restarts 3 \
  --detach \
  -- ./my-web-server

# Stream live stdout/stderr logs from a node
veloce-run --name worker --watch -- ./worker-binary
```

### 3. Layer-7 HTTP Ingress Routing

```bash
# Route http://api.vln/v1 → 127.0.0.1:4000 (stripping the /v1 prefix)
veloce-run ingress add -H api.vln -p /v1 -t 4000 --strip-prefix

# Route default app traffic http://app.vln → 127.0.0.1:3000
veloce-run ingress add -H app.vln -t 3000

# List active ingress rules
veloce-run ingress list

# Test via curl hitting the Ingress proxy (port 8080)
curl -H "Host: api.vln" http://127.0.0.1:8080/v1/healthz

# Remove an ingress rule
veloce-run ingress rm api.vln
```

### 4. P2P Mesh Connectivity

```bash
# On Machine A: generate a 15-minute single-use VM3 join code
veloce-run mesh identity --ttl 15 --one-time
# Output: VM3:AAA...===

# On Machine B: connect to Machine A
veloce-run mesh join "VM3:AAA...==="

# Check connected peers and latency
veloce-run mesh peers
veloce-run mesh status
veloce-run mesh ping <peer-uuid>

# Machine B can immediately access Machine A's services:
curl --proxy socks5://127.0.0.1:1055 http://web.vln/
```

### 5. Multi-Service Stacks with Veloce Compose

Create a `veloce-compose.yml`:
```yaml
version: "1.0"
services:
  database:
    executable: /usr/bin/redis-server
    args: ["--port", "6379"]
    hostname: redis.vln
    port: 6379
    healthcheck:
      tcp: 6379
      interval_secs: 5

  api:
    executable: ./api-server
    hostname: api.vln
    port: 8080
    ports:
      - "8080:8080"
    depends_on:
      database:
        condition: service_healthy
```

Start the stack:
```bash
veloce-run up -d
veloce-run ps
veloce-run down
```

---

## SDK Integration (Rust & C ABI)

### Async Rust (`veloce-sdk`)

```rust
use veloce_sdk::VeloceClient;
use veloce_ipc::message::Capability;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut client = VeloceClient::connect(
        "my-service",
        env!("CARGO_PKG_VERSION"),
        vec![Capability::SpawnNodes, Capability::NetRegister],
    ).await?;

    let node = client.spawn_node("worker", "./worker", &["--threads", "4"]).await?;
    println!("Spawned worker PID {} (node_id: {})", node.pid, node.node_id);

    client.register_host("service.vln", 8080, 0).await?;
    println!("Registered service.vln -> port 8080");

    Ok(())
}
```

### C / Python / Go FFI (`libveloce_sdk.so`)

Drop `libveloce_sdk.so` and `veloce_sdk.h` into your project for zero-dependency integration from Python (`ctypes`), C/C++, Go (`cgo`), or Node.js.

---

## Building from Source

### Prerequisites
- Linux kernel ≥ 5.4 with `cgroups v2` enabled (default on Ubuntu 20.04+, Debian 11+, Fedora, Arch, RHEL 9+)
- Rust stable (`cargo`, `rustc`)
- `pkg-config`, `libssl-dev`

### Build

```bash
cd Linux/

# Build all workspace crates and CLI binaries
cargo build --release

# Run the full unit and integration test suite
cargo test --workspace
```

Binaries will be generated at `Linux/target/release/`:
- `veloce-core` — daemon service
- `veloce-run` — CLI tool
- `libveloce_sdk.so` — C shared library

---

## License

Proprietary — © VeloceSolutions. All rights reserved.

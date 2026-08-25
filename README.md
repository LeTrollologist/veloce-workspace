# veloce-workspace

[![Windows CI](https://github.com/LeTrollologist/veloce-workspace/actions/workflows/windows.yml/badge.svg)](https://github.com/LeTrollologist/veloce-workspace/actions/workflows/windows.yml)
[![Linux CI](https://github.com/LeTrollologist/veloce-workspace/actions/workflows/linux.yml/badge.svg)](https://github.com/LeTrollologist/veloce-workspace/actions/workflows/linux.yml)
[![Release](https://img.shields.io/github/v/release/LeTrollologist/veloce-workspace?label=release)](https://github.com/LeTrollologist/veloce-workspace/releases)
[![License](https://img.shields.io/badge/license-proprietary-blue.svg)](LICENSE)

Monorepo containing the unified cross-platform codebase for **VeloceNetwork** — a lightweight, zero-kernel runtime for launching, managing, and privately networking isolated application workloads across Windows and Linux.

---

## 📁 Repository Layout

```text
veloce-workspace/
├── Windows/             ← Windows-native workspace (Job Objects, Named Pipes, DPAPI, NRPT, AppContainer)
│   ├── apps/            ← veloce-run, veloce-launcher, veloce-shell, dashboard, installer
│   ├── crates/          ← veloce-core, veloce-ipc, veloce-mesh, veloce-net, veloce-sdk
│   └── Cargo.toml       ← Windows workspace manifest (MSVC / GNU targets)
├── Linux/               ← Linux-native workspace (cgroups v2, Unix Domain Sockets, systemd)
│   ├── apps/            ← veloce-run, veloce-launcher, veloce-shell, dashboard, installer
│   ├── crates/          ← veloce-core, veloce-ipc, veloce-mesh, veloce-net, veloce-sdk
│   └── Cargo.toml       ← Linux workspace manifest (x86_64-unknown-linux-gnu)
├── .github/workflows/   ← Automated CI & Release build distribution workflows
└── Makefile             ← Subtree synchronization and release management
```

---

## 🌐 Production Mirrors

| Repository | Target Platform | Monorepo Subtree | Description |
|---|---|---|---|
| [**VeloceNetwork-Windows**](https://github.com/LeTrollologist/VeloceNetwork-Windows) | Windows (x86_64) | `Windows/` subtree | Production mirror for Windows releases & installers |
| [**VeloceNetwork-Linux**](https://github.com/LeTrollologist/VeloceNetwork-Linux) | Linux (x86_64) | `Linux/` subtree | Production mirror for Linux releases & systemd units |

---

## 🚀 Feature Status & Roadmap

| Milestone | Key Capabilities & Architecture Additions | Status |
|---|---|:---:|
| **v1.0.0** | Core Engine, Named Pipe / Unix Socket IPC, Job Objects / cgroups, Userspace DNS (:5354) & SOCKS5 (:1055), Noise_IK Mesh, NRPT Windows Routing | ✅ Complete |
| **v1.1.0** | Veloce Compose (`veloce-compose.yml`), TCP Port Forwarding, Health Probes (HTTP/TCP/Exec), `depends_on` startup ordering | ✅ Complete |
| **v1.2.0** | Stateful Workloads: Named Volumes (`VolumeRegistry`), Host Bind Mounts, DPAPI Encrypted Secrets Vault (`veloce-run secret`) | ✅ Complete |
| **v1.3.0** | Rolling Deployments & Desired-State Reconciler Loop (`veloce-run ps`, `veloce-run status`) | ✅ Complete |
| **v1.3.1** | Server-Signed VM3 Join Codes (`MeshGetJoinCodeV3` / `MeshJoinCodeV3Result` IPC) with TTL & single-use anti-replay protection | ✅ Complete |
| **v2.0.0** | Linux Engine Parity: Unix Domain Sockets, `cgroups v2` controllers (`cpu.max`, `memory.max`), `systemd` user service integration | ✅ Complete |
| **v2.1.0** | Layer-7 HTTP Ingress Reverse Proxy (`veloce-net/src/ingress.rs`, `veloce-run ingress` on `:8080`) | ✅ Complete |
| **v3.0.0** | Distributed Control Plane & Consensus (`ClusterCoordinator`, Term Tracking, Multi-Node Replica Scheduling) | ✅ Complete |
| **v3.1.0** | Dynamic Horizontal Process Autoscaler (HPA) & CronJob Scheduler with concurrency policies (`Allow`, `Forbid`, `Replace`) | ✅ Complete |
| **v3.2.0** | TLS Termination & Automatic HTTPS Ingress (`:8443`) with ephemeral self-signed SAN certificates & custom PEM loading | ✅ Complete |
| **v3.3.0** | Prometheus Metrics Exposition (`:9090/metrics`) & Embedded Zero-Dependency Web Status Portal (`:9090/`) | ✅ Complete |
| **v3.4.0** | Veloce Hub Application Registry & 1-Click Web Portal Deploy (`veloce-run hub search/publish/deploy`) | ✅ Complete |
| **v3.5.0** | Real-Time WebSocket Telemetry (`:9090/ws`), Web Terminal Console & P2P Replicated Mesh Key-Value Store (`veloce-run mesh kv`) | ✅ Complete |
| **v3.5.1** | CLI argument validation hardening, `--help` / `--version` service dispatcher bypass, cross-platform release synchronization | ✅ Complete |
| **v3.6.0** | Userspace `.vpack` Application Packager: Single-file archives, Ed25519 cryptographic signing/verification, sandboxed zero-root runtime (`veloce-run pack`) | ✅ Complete |
| **v3.6.1** | Non-Admin Desktop Compatibility: Automatic `%LOCALAPPDATA%` unprivileged storage fallback, interactive console mode, double-click pause protection | ✅ Complete |
| **v3.7.0** | Android Mobile Integration: Native Rust JNI runtime (`veloce-mobile`), zero-root `VpnService` for `*.vln` routing, Jetpack Compose companion app | ✅ Complete |
| **v3.8.0** | Enterprise OIDC SSO & ZTNA: PKCE browser auth (`veloce login`), group-based RBAC, Mesh ACL role bindings, Web Portal SSO | ✅ Complete |
| **v3.9.0** | First-Class WebAssembly (Wasm/WASI) Orchestration: Zero-root userspace runtime, WASI preview 1 IO, mesh host bindings (`veloce-run wasm`) | ✅ Complete |
| **v4.0.0** | "Bridge to Cloud" Unprivileged Kubernetes Remote Telepresence & Traffic Interceptor: In-cluster DNS (*.svc.cluster.local), header-based live traffic shadowing (`veloce-run bridge`) | ✅ Complete |
| **v4.1.0** | Zero-Trust Team Share ("Unprivileged Secure Tunnels"): Ephemeral VM3 share tokens (`vshare://...`), 1-command peer port sharing (`veloce-run share`, `veloce-run join`) | ✅ Complete |

---

## 🏗️ System Architecture

```text
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Your Applications                              │
│   • veloce-sdk (Rust async client)      • veloce_sdk.dll / libveloce_sdk.so │
│   • veloce-run (CLI orchestration)      • Web Status Portal (127.0.0.1:9090)│
└──────────────────────────────────────┬──────────────────────────────────────┘
                                       │ Named Pipe (Windows) / Unix Domain Socket (Linux)
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                                 VeloceCore                                  │
│  • Session Authentication (SID ACL / UID check + OsRng PSK)                 │
│  • Node Lifecycle & Supervision (Process Trees / Health Probes / Restarts)  │
│  • Process Sandboxing (Windows Job Objects & AppContainer / Linux cgroups v2│
│  • Policy Engine (RBAC, Mesh ACLs, Hot-reloaded TOML rules)                 │
│  • Desired-State Reconciler & HPA Autoscaler Loop                           │
│  • CronJob Scheduled Task Executor (cron syntax + @every interval)         │
│  • Embedded Web Status Portal & Real-Time WebSocket Telemetry (:9090)       │
│  • Veloce Hub Application Catalog Engine                                    │
│  • Shared Memory (mmap) Registry & DPAPI/ChaCha Encrypted Secrets Vault     │
└──────────────────────────┬──────────────────────────┬───────────────────────┘
                           │                          │
                           ▼                          ▼
   ┌────────────────────────────────┐   ┌───────────────────────────────────┐
   │         Node Workloads         │   │             VeloceNet             │
   │  • Microservices / Web Backends│   │  • Userspace DNS    :5354 (*.vln) │
   │  • Databases & Caches          │   │  • SOCKS5 Proxy     :1055         │
   │  • Batch / Scheduled Tasks     │   │  • HTTP L7 Ingress  :8080         │
   │  • Named Volumes & Bind Mounts │   │  • HTTPS L7 Ingress :8443 (TLS)   │
   │                                │   │  • P2P Mesh (Noise) :7474 ◄───────┼── Remote Peers
   └────────────────────────────────┘   └───────────────────────────────────┘
```

---

## ⚡ Quick Start & CLI Reference

### 1. Launch Processes into Private Mesh

```bash
# Launch a background service with resource limits and a .vln hostname
veloce-run --name api --hostname api.vln --port 3000 --cpu 50 --mem 512 -- node server.js

# Stream stdout/stderr in watch mode
veloce-run --name worker --watch -- python worker.py
```

### 2. Multi-Machine Mesh Networking

```bash
# Machine A: Generate a secure, time-limited join code
veloce-run mesh identity --ttl 30

# Machine B: Connect to Machine A using the code
veloce-run mesh join "VM3:ey..."

# Inspect mesh status and peer latency
veloce-run mesh peers
veloce-run mesh status
veloce-run mesh ping <PEER_ID>
```

### 3. P2P Replicated Mesh Key-Value Store (v3.5)

```bash
# Set a shared key replicated across all connected mesh peers
veloce-run mesh kv set config/database_url "postgres://db.vln:5432/prod"

# Read key
veloce-run mesh kv get config/database_url

# List all stored keys in the cluster
veloce-run mesh kv list
```

### 4. Multi-Service Orchestration (`veloce-compose.yml`)

```bash
# Deploy all services declared in veloce-compose.yml
veloce-run up -d

# Inspect live cluster status and desired vs actual replicas
veloce-run ps

# Tear down compose services
veloce-run down
```

### 5. Layer-7 HTTP & HTTPS Ingress Reverse Proxy

```bash
# Route http://api.vln/v1/* to localhost port 4000 (strip /v1 prefix)
veloce-run ingress add -H api.vln -p /v1 -t 4000 --strip-prefix

# Route HTTPS with automatic TLS certificate generation
veloce-run ingress add -H secure.vln -t 3000 --tls

# List all active routes
veloce-run ingress list
```

### 6. Autoscaling (HPA) & CronJobs

```bash
# Configure autoscaling between 2 and 10 replicas targeting 75% CPU
veloce-run autoscale set api --min 2 --max 10 --cpu 75

# Schedule a cron job executing every 15 minutes
veloce-run cron create db-backup -s "*/15 * * * *" -- python backup.py
```

### 7. Veloce Hub & Web Status Portal

```bash
# Search and deploy applications from Veloce Hub
veloce-run hub search web
veloce-run hub deploy redis

# Open the embedded browser Status Portal & Real-Time Console
veloce-run portal
```

### 8. Userspace `.vpack` Application Packager (v3.6)

```bash
# Generate an Ed25519 publisher keypair
veloce-run pack keygen --out publisher

# Initialize and build a signed .vpack package
veloce-run pack init ./my-app --name my-app
veloce-run pack build ./my-app --out my-app-1.0.0.vpack --sign publisher.priv

# Inspect metadata and verify cryptographic signature
veloce-run pack inspect my-app-1.0.0.vpack
veloce-run pack verify my-app-1.0.0.vpack --pubkey publisher.pub

# Extract package contents into a directory
veloce-run pack extract my-app-1.0.0.vpack --dir ./extracted-app

# Launch directly from the .vpack file into the mesh
veloce-run pack run my-app-1.0.0.vpack --name my-app --port 8080
```

---

## 🛠️ Development & Build Workflow

### Building and Testing (Windows)

```powershell
cd Windows
cargo test --workspace
cargo build --workspace --release
```

### Building and Testing (Linux)

```bash
cd Linux
cargo test --workspace
cargo build --workspace --release
```

### Monorepo Sync & Releases

```bash
# Push commits to monorepo
git push origin main

# Synchronize subtrees to downstream production mirrors
make sync-windows
make sync-linux
make sync-prod

# Tag and push production releases
make release-windows TAG=v3.5.1
make release-linux   TAG=v3.5.1
```

---

## 📄 License

Proprietary — © VeloceSolutions. All rights reserved.

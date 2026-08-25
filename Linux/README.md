# veloce-workspace

[![Windows CI](https://github.com/LeTrollologist/veloce-workspace/actions/workflows/windows.yml/badge.svg)](https://github.com/LeTrollologist/veloce-workspace/actions/workflows/windows.yml)
[![Linux CI](https://github.com/LeTrollologist/veloce-workspace/actions/workflows/linux.yml/badge.svg)](https://github.com/LeTrollologist/veloce-workspace/actions/workflows/linux.yml)
[![Release](https://img.shields.io/github/v/release/LeTrollologist/veloce-workspace?label=release)](https://github.com/LeTrollologist/veloce-workspace/releases)
[![License](https://img.shields.io/badge/license-proprietary-blue.svg)](LICENSE)

Monorepo containing the unified cross-platform codebase for **VeloceNetwork** — a lightweight, zero-kernel, zero-root userspace service mesh and runtime for launching, managing, privately networking, and observing isolated application workloads across Windows, Linux, and Android.

---

## 📁 Repository Layout

```text
veloce-workspace/
├── Windows/             ← Windows-native workspace (Job Objects, Named Pipes, DPAPI, NRPT, AppContainer)
│   ├── apps/            ← veloce-run, veloce-launcher, veloce-shell, dashboard, installer
│   ├── crates/          ← veloce-core, veloce-ipc, veloce-mesh, veloce-net, veloce-sdk, veloce-mobile
│   └── Cargo.toml       ← Windows workspace manifest (MSVC / GNU targets)
├── Linux/               ← Linux-native workspace (cgroups v2, Unix Domain Sockets, systemd)
│   ├── apps/            ← veloce-run, veloce-launcher, veloce-shell, dashboard, installer
│   ├── crates/          ← veloce-core, veloce-ipc, veloce-mesh, veloce-net, veloce-sdk, veloce-mobile
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
| **v4.2.0** | OpenTelemetry (OTel) Native Distributed Tracing & Observability: W3C trace context, zero-config OTLP JSON export (`:4318`), live trace waterfall UI (`veloce-run trace`) | ✅ Complete |

---

## 🏗️ System Architecture

```text
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Your Applications                              │
│   • veloce-sdk (Rust async client)      • veloce_sdk.dll / libveloce_sdk.so │
│   • veloce-run (CLI orchestration)      • Web Status Portal (127.0.0.1:9090)│
│   • Wasm / WASI Sandboxed Modules       • Mobile Clients (Android VpnService│
└──────────────────────────────────────┬──────────────────────────────────────┘
                                       │ Named Pipe (Windows) / Unix Domain Socket (Linux)
                                       ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                                 VeloceCore                                  │
│  • Session Authentication (SID ACL / UID check + OsRng PSK)                 │
│  • Enterprise OIDC Identity & RBAC (Microsoft Entra ID, Okta, GitHub SSO)   │
│  • Node Lifecycle & Supervision (Process Trees / Health Probes / Restarts)  │
│  • Process Sandboxing (Windows Job Objects & AppContainer / Linux cgroups v2│
│  • Embedded Pure-Rust WebAssembly Engine (Wasm / WASI Preview 1)            │
│  • OpenTelemetry (OTel) Engine: W3C Context, Ring Buffer, OTLP Exporter     │
│  • Zero-Trust Team Share Engine: Ephemeral VM3 Tokens (vshare://...)        │
│  • Kubernetes Telepresence Bridge: In-cluster DNS & Live Traffic Intercept  │
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
   │  • Wasm Edge Modules (.wasm)   │   │  • HTTP L7 Ingress  :8080         │
   │  • .vpack Standalone Archives  │   │  • HTTPS L7 Ingress :8443 (TLS)   │
   │  • Named Volumes & Bind Mounts │   │  • P2P Mesh (Noise) :7474 ◄───────┼── Remote Peers
   └────────────────────────────────┘   └───────────────────────────────────┘
```

---

## ⚡ Quick Start & CLI Reference

### 1. Launch Processes & Wasm Modules into Private Mesh

```bash
# Launch a background service with resource limits and a .vln hostname
veloce-run --name api --hostname api.vln --port 3000 --cpu 50 --mem 512 -- node server.js

# Execute a WebAssembly module with sandboxed WASI runtime
veloce-run wasm run ./service.wasm --env LOG_LEVEL=debug

# Inspect exports and imports of a WebAssembly module
veloce-run wasm inspect ./service.wasm
```

### 2. Multi-Machine Mesh Networking & Zero-Trust Share

```bash
# Share a local port with a teammate or client via ephemeral VM3 share token
veloce-run share 3000 --name dev-api --ttl 2h

# Teammate connects to the shared service instantly
veloce-run join vshare://vm3-eyJhbGciOi...

# List and manage active share links
veloce-run share list
veloce-run share revoke sh-9f82ab12
```

### 3. OpenTelemetry (OTel) Distributed Tracing

```bash
# List recent distributed traces across local and remote mesh services
veloce-run trace list

# Inspect an end-to-end trace waterfall and latency breakdown in terminal ASCII
veloce-run trace inspect 4bf92f3577b34da6a3ce929d0e0e4736

# Export traces directly to Jaeger, Grafana Tempo, or OTel Collector
veloce-run trace export --endpoint http://localhost:4318/v1/traces --enable
```

### 4. "Bridge to Cloud" (Kubernetes Telepresence & Interceptor)

```bash
# Connect local environment to remote staging Kubernetes cluster
veloce-run bridge connect --peer 10.96.0.10:7474 --namespace staging

# Shadow live cluster traffic carrying debug header to local IDE debugger
veloce-run bridge intercept --service payment-service --header "X-Debug: true" --target 9000

# Inspect active cloud bridges
veloce-run bridge list
```

### 5. Enterprise OIDC Single Sign-On (SSO) & ZTNA

```bash
# Authenticate with corporate identity provider (Entra ID, Okta, GitHub)
veloce-run login --provider https://login.microsoftonline.com/tenant-id/v2.0 --client-id <ID>

# Inspect active OIDC session and Mesh RBAC groups
veloce-run auth status

# Logout and revoke token
veloce-run auth logout
```

### 6. Userspace `.vpack` Application Packager

```bash
# Generate an Ed25519 publisher keypair
veloce-run pack keygen --out publisher

# Build and sign a standalone .vpack archive
veloce-run pack build ./my-app --out my-app-1.0.0.vpack --sign publisher.priv

# Verify signature and launch directly into the mesh
veloce-run pack verify my-app-1.0.0.vpack --pubkey publisher.pub
veloce-run pack run my-app-1.0.0.vpack --name my-app --port 8080
```

---

## 🛠️ Development & Build Workflow

### Building and Testing (Windows)

```powershell
cd Windows
$env:PATH = "C:\Users\Owner\.gemini\tools\mingw64\bin;C:\Users\Owner\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained;$env:PATH"
cargo test --workspace -j 2
cargo build --workspace --release -j 2
```

### Building and Testing (Linux)

```bash
cd Linux
cargo test --workspace -j 2
cargo build --workspace --release -j 2
```

---

## 📄 License

Proprietary — © VeloceSolutions. All rights reserved.

# VeloceNetwork — Comprehensive Project Scope

## 1. Project Overview

* **Project Name:** VeloceNetwork
* **Type:** Cross-Platform Userspace Service Mesh, Orchestration Engine & Edge Runtime
* **Target Platforms:** Windows (10/11/Server), Linux (x86_64, aarch64), Android (ARM64, x86_64 via JNI)
* **Core Value Proposition:** A lightweight, zero-kernel, zero-root runtime for launching, managing, privately networking, securing, and observing isolated application workloads across multi-cloud, edge, desktop, and mobile environments without kernel drivers, VPNs, or elevated privileges.
* **Target Users:** Software Engineers, DevOps/SREs, Platform Engineers, Commercial ISVs, Enterprise IT & Security Teams.

---

## 2. Problem Statement & Architecture Solution

### Current Developer & Infrastructure Challenges
1. **Container / VM Overhead:** Running local microservices traditionally requires heavy virtualization (Docker Desktop, WSL2, Hyper-V) that consumes massive RAM/CPU and requires administrative host privileges.
2. **Fragile Cross-Platform Dependencies:** Packaging and distributing native microservices across diverse OS platforms without Docker is brittle.
3. **Complex Telepresence & Remote Staging:** Connecting local developer IDEs to remote Kubernetes clusters typically requires administrative host privileges, virtual network adapters (TUN/TAP), and fragile VPN tunnels.
4. **Friction in Ad-Hoc Service Sharing:** Sharing a local development port with colleagues or clients requires opening insecure firewall ports or relying on third-party SaaS tunnels (e.g. ngrok).
5. **Observability Gaps in Local Dev:** Developers lack production-grade distributed tracing (W3C / OpenTelemetry) for locally running microservice meshes.

### VeloceNetwork Solution
1. **100% Userspace Isolation:** Native OS process sandboxing using Windows Job Objects + AppContainers and Linux `cgroups v2` with strict CPU %, memory caps, and wall-clock lifetimes.
2. **Private `.vln` Virtual Networking:** Built-in userspace DNS (`:5354`) and SOCKS5 proxy (`:1055`) that resolve private domain names without host elevation.
3. **WireGuard-Grade P2P Mesh:** `Noise_IK_25519_ChaChaPoly_BLAKE2s` encrypted multi-machine mesh with STUN WAN traversal and server-signed VM3 join codes.
4. **Embedded WebAssembly (Wasm/WASI) Runtime:** Zero-root, OS-agnostic Wasm execution engine with WASI Preview 1 host bindings.
5. **"Bridge to Cloud" Telepresence:** Transparent userspace forwarding and header-based live traffic shadowing from remote Kubernetes clusters.
6. **Zero-Trust Team Share:** 1-command peer port sharing using cryptographically signed, ephemeral VM3 tokens (`vshare://...`).
7. **Native OpenTelemetry (OTel) Engine:** Built-in W3C distributed tracing, in-memory span ring buffers, terminal ASCII waterfall inspection, and zero-config OTLP export (`:4318`).

---

## 3. Component Scope Matrix

| Subsystem | Primary Responsibilities & Shipped Capabilities | Shipped In |
|---|---|:---:|
| **`veloce-core`** | Background Daemon, Session Auth (SID ACL / UID check + 256-bit PSK), Job Objects / cgroups v2 Sandboxing, PolicyEngine (RBAC / Mesh ACLs), Desired-State Reconciler, HPA Autoscaler, CronJob Scheduler, Embedded Web Portal (`:9090`), Real-time WebSocket Telemetry (`:9090/ws`), Veloce Hub Engine, Wasm Runtime, Kubernetes Telepresence Bridge, Zero-Trust Share Engine, OpenTelemetry Engine | v1.0 – v4.2 |
| **`veloce-ipc`** | Wire protocol framing (`VELC`), bincode serialization, complete message catalog (0x00–0x97), capability grants (`SpawnNodes`, `NetRegister`, `BridgeManage`, `ShareManage`, `TraceRead`, `TraceAdmin`, `OidcAuth`) | v1.0 – v4.2 |
| **`veloce-net`** | Userspace DNS resolver (`:5354`), SOCKS5 proxy (`:1055`), Layer-7 HTTP/HTTPS reverse proxy (`:8080` / `:8443`), TLS termination with dynamic SAN certificates, upstream query validation | v1.0 – v3.2 |
| **`veloce-mesh`** | Noise_IK mutual authentication, LWW CRDT gossip protocol, STUN NAT discovery, VM2/VM3 join codes, P2P replicated Mesh Key-Value Store (`veloce-run mesh kv`), cluster consensus coordinator | v0.4 – v3.5 |
| **`veloce-mobile`** | Native Rust JNI library (`libveloce_mobile.so`), Android `VpnService` zero-root router, mobile mesh lifecycle | v3.7 |
| **`veloce-sdk`** | Async Rust client (`VeloceClient`), C FFI (`veloce_sdk.dll` / `libveloce_sdk.so`), non-blocking event pump | v1.0 – v4.2 |
| **`veloce-run`** | Unified CLI orchestration suite: process spawner, compose (`up`/`down`), share, trace, bridge, wasm, login/auth, mesh, ingress, pack, secret, autoscale, cron, hub, portal | v0.3 – v4.2 |
| **`apps/dashboard`**| Tauri 2 + Svelte 5 + Canvas 2D GUI: live topology canvas, traffic heatmap, node inspection, log streaming | v0.6 – v3.5 |
| **`apps/installer`**| Glassmorphic 5-step desktop installer with background service registration | v0.2 |

---

## 4. Completed Milestone Deliverables

### Phase 1: Core Engine & Mesh Foundations (v0.1 – v0.9)
- [x] **v0.1.0**: Background service daemon, Job Objects, async Rust SDK, C FFI bindings.
- [x] **v0.2.0**: Glassmorphic 5-step installer, Node Templates, resource sparklines, auto-restart policies.
- [x] **v0.3.0**: `veloce-run` CLI launcher, AppContainer kernel isolation.
- [x] **v0.4.0**: Multi-machine P2P mesh (`Noise_IK`), `.vln` gossip protocol, STUN WAN discovery.
- [x] **v0.5.0**: PolicyEngine with TOML RBAC and Mesh ACLs, VM2 multi-address join codes.
- [x] **v0.6.0**: Svelte 5 + Canvas 2D dashboard, live topology canvas, 60-cell peer heatmap.
- [x] **v0.7.0 – v0.9.0**: Three comprehensive security audits, VM3 join codes with TTL/one-time replay prevention, gossip ownership validation.

### Phase 2: Production Service Mesh & Orchestration (v1.0 – v3.0)
- [x] **v1.0.0**: Production release, NRPT system-wide DNS routing, signed installers, CI/CD pipeline.
- [x] **v1.1.0**: Veloce Compose (`veloce-compose.yml`), TCP port forwarding, HTTP/TCP/Exec health probes.
- [x] **v1.2.0**: Stateful workloads, named volumes, host bind mounts, DPAPI-backed runtime secrets vault.
- [x] **v1.3.0**: Rolling deployment strategies, desired-state reconciler loop (`veloce-run ps/status`).
- [x] **v2.0.0**: Full Linux engine parity: Unix Domain Sockets, `cgroups v2` controllers, `systemd` user units.
- [x] **v2.1.0**: Layer-7 HTTP Ingress reverse proxy (`:8080`) with longest-prefix routing.
- [x] **v3.0.0**: Distributed cluster coordinator, term tracking, Raft-style election, multi-node replica scheduling.

### Phase 3: Edge Runtime, Ingress TLS & Observability (v3.1 – v3.7)
- [x] **v3.1.0**: Horizontal Process Autoscaler (HPA) targeting CPU utilization, CronJob scheduler.
- [x] **v3.2.0**: Layer-7 HTTPS Ingress (`:8443`) with automatic TLS certificate termination and custom PEM loading.
- [x] **v3.3.0**: Prometheus metrics exposition (`:9090/metrics`), embedded zero-dependency Web Status Portal.
- [x] **v3.4.0**: Veloce Hub application catalog & 1-click web deployment (`veloce-run hub`).
- [x] **v3.5.0**: Real-time WebSocket telemetry (`:9090/ws`), web terminal console, P2P Replicated Mesh KV store.
- [x] **v3.6.0**: Userspace `.vpack` Application Packager with Ed25519 cryptographic signing and verification.
- [x] **v3.6.1**: Non-Admin Desktop Compatibility with automatic `%LOCALAPPDATA%` fallback.
- [x] **v3.7.0**: Android Mobile Runtime (`veloce-mobile` Rust JNI) and zero-root `VpnService` companion app.

### Phase 4: Enterprise ZTNA, Cloud Telepresence & OpenTelemetry (v3.8 – v4.2)
- [x] **v3.8.0 — Enterprise OIDC Identity & ZTNA**:
  - [x] Browser PKCE authentication flow (`veloce-run login`) supporting Entra ID, Okta, and GitHub SSO.
  - [x] Role-based access control (RBAC) and automatic Mesh ACL binding based on identity claims.
- [x] **v3.9.0 — First-Class WebAssembly (Wasm/WASI) Orchestration**:
  - [x] Zero-dependency, pure-Rust WebAssembly interpreter (`wasmi`) embedded directly in VeloceCore.
  - [x] WASI Preview 1 host bindings, environment variable injection, and linear memory sandboxing.
  - [x] CLI commands: `veloce-run wasm run <FILE.wasm>` and `veloce-run wasm inspect <FILE.wasm>`.
- [x] **v4.0.0 — "Bridge to Cloud" Kubernetes Remote Telepresence**:
  - [x] In-cluster Kubernetes telepresence proxy without requiring local root elevation or TAP adapters.
  - [x] Header-based live traffic shadowing (`X-Debug: true`, `X-Veloce-Intercept`) to local IDE debuggers.
  - [x] In-cluster DNS forwarding for `*.svc.cluster.local`.
- [x] **v4.1.0 — Zero-Trust Team Share ("Unprivileged Secure Tunnels")**:
  - [x] 1-Command peer port sharing via cryptographically signed VM3 share tokens (`vshare://...`).
  - [x] Ephemeral TTLs, single-use anti-replay enforcement, dynamic DNS synthesis (`*.shared.vln`).
  - [x] CLI commands: `veloce-run share <PORT>`, `connect`, `list`, `revoke`, and `veloce-run join <CODE>`.
- [x] **v4.2.0 — OpenTelemetry (OTel) Native Observability & Distributed Tracing**:
  - [x] W3C Trace Context propagation (`traceparent: 00-{trace_id}-{span_id}-01`) across ingress & mesh.
  - [x] In-memory ring buffer (last 2,000 spans) for instant query and waterfall aggregation.
  - [x] Pure-Rust zero-dependency OTLP/HTTP JSON exporter streaming directly to Jaeger / Grafana Tempo (`:4318`).
  - [x] CLI live waterfall inspector (`veloce-run trace inspect <ID>`) and Web Status Portal REST API (`/api/traces`).

---

## 5. Future Roadmap 📋

- **Dedicated Mobile UI App**: Packaged Android APK / Jetpack Compose application for Play Store distribution.
- **Wasm Component Model (WASI Preview 2)**: Support for Wasm components, WASI 0.2 sockets, and HTTP client/server interfaces.
- **Multi-Cloud Staging Fleet Coordinator**: Central management portal for enterprise-wide Kubernetes telepresence bridges.
- **eBPF Acceleration (Optional Linux Backend)**: Kernel-bypass packet steering for high-throughput edge nodes.

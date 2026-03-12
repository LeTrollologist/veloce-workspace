# VeloceNetwork — Project Scope

## 1. Project Overview

**Project Name:** VeloceNetwork
**Type:** Windows-native desktop orchestration platform
**Core Summary:** A lightweight runtime for launching, managing, and privately networking isolated application nodes on Windows without kernel drivers, VPNs, or elevated privileges beyond a single background service.
**Target Users:** Developers, DevOps engineers, commercial software vendors, system administrators

---

## 2. Problem Statement

### Current Challenges
- Running multiple local microservices requires complex setups (Docker, WSL, port management)
- No lightweight isolation for desktop applications beyond full virtualization
- Creating private namespaces requires kernel-level changes or VPN software
- Process management lacks fine-grained resource control and health monitoring
- Cross-process communication requires manual socket/pipe implementation

### VeloceNetwork Solution
- Single-machine (and now multi-machine) service mesh without kernel drivers
- Job Object-based process isolation with resource limits
- Userspace DNS + SOCKS5 proxy for `.vln` private namespace
- Named-pipe IPC with SID ACL and per-session PSK security
- Noise_IK encrypted P2P mesh for transparent cross-machine `.vln` routing

---

## 3. Scope Definition

### In Scope (v0.1 – v0.4)

| Component | Features shipped |
|---|---|
| **veloce-core** | Background Windows service, SID ACL + OsRng PSK, Job Objects (CPU/memory/lifetime), mmap registry, health-loop, AppContainer kernel sandbox, MeshState, mesh TCP server (:7474) |
| **veloce-net** | DNS :5354 for `*.vln`/`*.veloce`, SOCKS5 :1055, TTL GC; DNS compression DoS fix (max 10 pointer jumps) |
| **veloce-ipc** | VELC framing, bincode encoding, message types 0x00–0x61 (core/net/sdk) + 0x50–0x57 (mesh), stable discriminants |
| **veloce-sdk** | `VeloceClient` async Rust client, C FFI (`veloce_sdk.dll`), `veloce_poll_event()` pump, 5 template methods, 4 mesh methods |
| **veloce-mesh** | x25519 identity (Noise_IK_25519_ChaChaPoly_BLAKE2s), PeerConnection gossip (LWW CRDT), transparent TCP forwarder, join-code pairing |
| **apps/dashboard** | Tauri 2 GUI — nodes table, templates, log viewer, resource meters (CPU% + peak MB), VeloceNet tab, mesh UI (This Machine card + Connected Peers) |
| **apps/installer** | Glassmorphic 5-step Tauri installer; service registration, PATH, registry |
| **apps/veloce-run** | CLI launcher (`--name`, `--hostname`, `--cpu`, `--mem`, `--restarts`, `--watch`, `--detach`) + `mesh identity/join/peers/leave` subcommand group |

### Out of Scope (Future Releases)

| Feature | Target |
|---|---|
| Policy Engine (process RBAC + mesh ACLs) | v0.5 |
| STUN/ICE WAN hole-punching | v0.5 |
| Dashboard v2 (topology canvas, heatmap, log viewer panel) | v0.6 |
| WireGuard-NT kernel driver (perf upgrade, requires admin) | v1.0 |
| Signed installer with auto-update | v1.0 |
| Linux port (cgroups v2, Unix domain sockets) | v2.0 |
| Unprivileged user namespaces (rootless Linux) | v2.0 |
| Python / Node.js / Go SDK bindings | v2.0 |

---

## 4. Objectives

### Primary Objectives
1. **Provide isolated node execution** — Spawn processes as Windows Job Objects (+ optional AppContainer) with configurable CPU %, memory caps, and lifetime limits
2. **Enable private networking** — Route `.vln` hostnames locally (and across machines via mesh) via built-in DNS and SOCKS5 proxy
3. **Secure local IPC** — SID-based pipe ACL + per-session OsRng PSK authentication
4. **Offer cross-language SDK** — Rust client + C FFI for Python, C++, Go, Node, Delphi

### Secondary Objectives
5. **Developer experience** — Dashboard GUI for visual node management; `veloce-run` CLI for zero-friction launching
6. **Production readiness** — Windows service installation, auto-start capability, glassmorphic installer
7. **Extensibility** — Capability-based permissions, registry for inter-node coordination, mesh for multi-machine topologies

---

## 5. Deliverables

### v0.1.0 ✅ Released

- [x] `veloce-core.exe` — Background service with node management
- [x] `veloce-sdk` — Rust async client library
- [x] `veloce_sdk.dll` + `veloce_sdk.h` — C FFI bindings
- [x] `apps/dashboard` — Tauri 2 desktop application (dark-themed, live node table)
- [x] Wire protocol specification (`crates/veloce-ipc`)
- [x] Examples: `net_demo`, `hello_node`

### v0.2.0 ✅ Released

- [x] Glassmorphic 5-step Tauri installer (service registration, PATH, registry)
- [x] Node Templates — save named configurations, spawn with one command
- [x] Resource Usage Display — live CPU% delta + peak memory MB in nodes table
- [x] Health policies — exponential back-off auto-restart on crash
- [x] Log streaming — stdout/stderr captured and streamed to Dashboard

### v0.3.0 ✅ Released — Phase 1 "Windows Gold Standard" complete

- [x] `apps/veloce-run` CLI — `--name`, `--hostname`, `--cpu`, `--mem`, `--restarts`, `--watch`, `--detach`
- [x] AppContainer isolation — optional per-node kernel-enforced sandbox (no admin required)

### v0.4.0 🟡 PR #10 open

- [ ] `crates/veloce-mesh` — Noise_IK P2P encrypted mesh (same crypto as WireGuard, zero-admin)
- [ ] Multi-machine `.vln` namespace via join-code pairing + LWW gossip protocol
- [ ] `veloce-run mesh identity / join / peers / leave` CLI subcommands
- [ ] Dashboard mesh UI (This Machine card + Connected Peers table)
- [ ] Security fixes: DNS compression DoS, OsRng PSK, identity key file ACL

---

## 6. Technical Constraints

| Constraint | Description |
|------------|-------------|
| Platform | Windows 10/11 (x86-64) primary; Linux deferred to v2.0 |
| Runtime | Rust stable + `x86_64-pc-windows-msvc` target |
| Dependencies | No kernel drivers, no TAP adapters, no admin required (beyond service install) |
| Max payload | 4 MiB per IPC message |
| Network ports | DNS :5354, SOCKS5 :1055, Mesh TCP :7474 (all configurable) |
| Encryption | Noise_IK_25519_ChaChaPoly_BLAKE2s (same algorithm as WireGuard) |

---

## 7. User Personas

| Persona | Needs |
|---------|-------|
| **DevOps Engineer** | Spin up microservices locally without Docker; share `.vln` hosts across dev VMs |
| **Desktop App Developer** | Isolate third-party plugins as sandboxed nodes |
| **Game Server Host** | Run multiple game instances with memory caps; share between LAN machines |
| **Enterprise IT** | Internal tooling with private namespace; multi-machine mesh for team environments |
| **Hobbyist** | Experiment with orchestration and mesh networking concepts |

---

## 8. Success Metrics

- Node spawn latency < 100 ms
- DNS resolution < 10 ms for `.vln` queries
- Mesh handshake (Noise_IK) < 50 ms on LAN
- Cross-machine `.vln` round-trip overhead < 5 ms on LAN
- Zero kernel-mode dependencies
- Cross-user pipe access blocked by SID ACL
- Session key invalidates prior connections on restart

---

## 9. Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Port conflict with system services | Medium | Configurable ports via config file |
| Job Object limits exceeded | Medium | Document limits, provide fallback mode |
| Memory-mapped registry corruption | Low | In-memory fallback, periodic flush |
| SOCKS5 proxy not supported by app | Low | DNS-only mode, environment variable hints |
| Mesh peer behind strict NAT (WAN) | Medium | LAN-only for v0.4; STUN hole-punching in v0.5 |
| Identity key file compromise | Medium | File ACL (read-only, owner-only) set at creation |

---

## 10. Timeline

| Milestone | Status |
|---|---|
| v0.1.0 (Core + SDK + Dashboard) | ✅ Released |
| v0.2.0 (Installer, Templates, Resources, Health, Logs) | ✅ Released |
| v0.3.0 (veloce-run CLI + AppContainer — Phase 1 complete) | ✅ Released |
| v0.4.0 (Multi-Machine VeloceNet P2P mesh) | 🟡 Q2 2026, PR #10 open |
| v0.5.0 (Policy Engine + STUN WAN mesh) | Q3 2026 |
| v0.6.0 (Dashboard v2 — topology canvas, heatmap, log viewer) | Q3 2026 |
| v1.0 (WireGuard-NT kernel driver + signed auto-update installer) | Q4 2026 |
| v2.0 (Linux port + unified SDK bindings) | 2027 |

---

## 11. Forward Roadmap Detail

### Phase 2 — Mesh & Automation (v0.5 – v0.6)

**Policy Engine (v0.5)**
- Tier 1 — Process RBAC: declarative rules governing which applications may request which capabilities (`SpawnNodes`, `NetRegister`, etc.)
- Tier 2 — Mesh ACLs: `ALLOW node:api.vln TO node:db.vln ON PORT 5432` style firewall rules enforced at the forwarder layer
- TOML/JSON policy files hot-reloaded by `veloce-core` without restart

**STUN WAN Mesh (v0.5)**
- Extend the v0.4 LAN mesh across NAT / internet
- Use STUN binding requests to discover each machine's external IP + port
- Peers exchange STUN-discovered endpoints over the already-encrypted Noise channel
- No manual port forwarding or VPN configuration required for typical home/office NAT

**Dashboard v2 (v0.6)**
- Drag-and-drop node wiring canvas — visually connect `.vln` services, see data flow
- Live traffic heatmap — bytes/s per mesh tunnel and per `.vln` host
- Historical resource graphs — CPU%, memory, restart counts over time
- Full log viewer panel with search/filter, replacing the current streaming overlay

### Phase 3 — Linux Engine Swap (v2.0)

- Replace named pipes → Unix domain sockets (same VELC framing, same SDK API)
- Replace Job Objects → cgroups v2 (CPU quota + memory.max) + process groups for clean tree kill
- Unprivileged user namespaces for rootless sandboxing (equivalent to AppContainer on Linux)
- Swappable backend: `veloce-core` auto-detects OS at compile time and links the right driver module
- Unified Python, Node.js, and Go SDK bindings via the existing C FFI layer

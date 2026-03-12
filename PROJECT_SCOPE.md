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
- Single-machine service mesh without kernel drivers
- Job Object-based process isolation with resource limits
- Userspace DNS + SOCKS5 proxy for `.vln` private namespace
- Named-pipe IPC with SID ACL and per-session PSK security

---

## 3. Scope Definition

### In Scope (Current v0.1 - v0.2)

| Component | Features |
|-----------|----------|
| **veloce-core** | Background Windows service, Session auth (SID ACL + PSK), Node lifecycle (spawn/kill/monitor), Job Objects (CPU/memory/lifetime limits), Shared mmap registry |
| **veloce-net** | DNS server (:5354) for `*.vln`/`*.veloce`, SOCKS5 proxy (:1055), TTL-based registration expiry |
| **veloce-ipc** | Fixed-header wire protocol (VELC magic), bincode encoding, message types with stable discriminants |
| **veloce-sdk** | Async Rust client (`VeloceClient`), C FFI (`veloce_sdk.dll`), `veloce_poll_event()` event pump |
| **apps/dashboard** | Tauri 2 desktop GUI, dark-themed UI, live node table, hostname registration |
| **apps/installer** | NSIS-based Windows installer |

### Out of Scope (Future Releases)

| Feature | Release Target |
|---------|----------------|
| Multi-machine VeloceNet | v0.4+ |
| Linux/macOS support | v0.5+ |
| Node templates | v0.3 |
| Health policies (auto-restart) | v0.3 |
| Log streaming | v0.3 |
| Dashboard v2 (historical metrics) | v0.4 |
| Policy engine | v0.5 |
| Encrypted VeloceNet | v0.6 |
| Python/Node.js/Go SDK bindings | v0.6 |
| Signed installer with auto-update | v0.4 |

---

## 4. Objectives

### Primary Objectives
1. **Provide isolated node execution** — Spawn processes as Windows Job Objects with configurable CPU %, memory caps, and lifetime limits
2. **Enable private networking** — Route `.vln` hostnames locally via built-in DNS and SOCKS5 proxy
3. **Secure local IPC** — SID-based pipe ACL + per-session PSK authentication
4. **Offer cross-language SDK** — Rust client + C FFI for Python, C++, Go, Node, Delphi

### Secondary Objectives
5. **Developer experience** — Dashboard GUI for visual node management
6. **Production readiness** — Windows service installation, auto-start capability
7. **Extensibility** — Capability-based permissions, registry for inter-node coordination

---

## 5. Deliverables

### v0.1.0 (Current)

- [x] `veloce-core.exe` — Background service with node management
- [x] `veloce-sdk` — Rust async client library
- [x] `veloce_sdk.dll` + `veloce_sdk.h` — C FFI bindings
- [x] `apps/dashboard` — Tauri 2 desktop application
- [x] Wire protocol specification (`crates/veloce-ipc`)
- [x] Examples: `net_demo`, `hello_node`

### v0.2.0

- [x] `veloce-net` — DNS + SOCKS5 for `.vln` namespace
- [x] Shared mmap registry
- [x] Push events (Started, Exited, Crashed)
- [x] Resource limit enforcement (CPU, memory, wall-clock)

### v0.3.0 (Planned)

- [ ] Node templates
- [ ] Health policies (restart-on-crash)
- [ ] Log streaming
- [ ] Installer improvements

---

## 6. Technical Constraints

| Constraint | Description |
|------------|-------------|
| Platform | Windows 10/11 (x86-64) only (v0.1-v0.3) |
| Runtime | Rust stable + `x86_64-pc-windows-msvc` target |
| Dependencies | No kernel drivers, no TAP adapters, no admin required |
| Max payload | 4 MiB per IPC message |
| Network ports | DNS :5354, SOCKS5 :1055 (configurable) |

---

## 7. User Personas

| Persona | Needs |
|---------|-------|
| **DevOps Engineer** | Spin up microservices locally without Docker |
| **Desktop App Developer** | Isolate third-party plugins as sandboxed nodes |
| **Game Server Host** | Run multiple game instances with memory caps |
| **Enterprise IT** | Internal tooling with private namespace |
| **Hobbyist** | Experiment with orchestration concepts |

---

## 8. Success Metrics

- Node spawn latency < 100ms
- DNS resolution < 10ms for `.vln` queries
- Zero kernel-mode dependencies
- Cross-user pipe access blocked by SID ACL
- Session key invalidates prior connections on restart

---

## 9. Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Port conflict with system services | Medium | Allow configurable ports via config file |
| Job Object limits exceeded | Medium | Document limits, provide fallback mode |
| Memory-mapped registry corruption | Low | In-memory fallback, periodic flush |
| SOCKS5 proxy not supported by app | Low | DNS-only mode, environment variable hints |

---

## 10. Timeline

| Milestone | Target |
|-----------|--------|
| v0.1.0 (Core + SDK + Dashboard) | Released |
| v0.2.0 (VeloceNet + Registry + Events) | Released |
| v0.3.0 (Templates + Health + Logs) | Q2 2026 |
| v0.4 (Installer v2 + Dashboard v2) | Q3 2026 |
| v0.5+ (Linux, Policy Engine) | Q4 2026+ |

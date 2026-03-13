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

### In Scope (v0.1 – v0.7)

| Component | Features shipped |
|---|---|
| **veloce-core** | Background Windows service, SID ACL + OsRng PSK, Job Objects (CPU/memory/lifetime), mmap registry, health-loop, AppContainer kernel sandbox, MeshState, mesh TCP server (:7474), PolicyEngine (TOML RBAC + mesh ACLs, hot-reload, kernel-verified `exe` field); `TrafficQuery` IPC handler; pre-auth state machine; server-authoritative capability grant; `VELOCE_SKIP_PSK` SYSTEM guard |
| **veloce-net** | DNS :5354 for `*.vln`/`*.veloce` (localhost-bind, upstream validation); SOCKS5 :1055 (VLN-only scope); TTL GC; DNS compression DoS fix; `Arc<AtomicU64>` `bytes_proxied` per `NetRecord`; `NetRegistry::traffic_snapshot()` |
| **veloce-ipc** | VELC framing, bincode encoding, message types 0x00–0x72 + `TrafficQuery (0x80)` / `TrafficStatsResult (0x81)`; `TunnelTrafficMsg`, `HostTrafficMsg`, `TrafficStatsMsg`; `PolicyRuleMsg.exe` optional field |
| **veloce-sdk** | `VeloceClient` async Rust client, C FFI (`veloce_sdk.dll`), template methods, mesh methods, `policy_get_rules()`, `policy_reload()`, `query_traffic()` |
| **veloce-mesh** | x25519 identity (Noise_IK_25519_ChaChaPoly_BLAKE2s), PeerConnection gossip (LWW CRDT), TCP forwarder, STUN WAN discovery, VM2 join codes, ACL callback; `Arc<AtomicU64>` `tx_bytes`/`rx_bytes` per `PeerConnection`; `MeshState::query_traffic_stats()`; 10-second handshake timeout |
| **apps/dashboard** | Svelte 5 + Canvas 2D — nodes + sparklines + history graphs, templates, logs (search/filter/auto-scroll/5k-cap), VeloceNet + policy panel, topology canvas (drag-and-drop, heatmap, live traffic tables) |
| **apps/installer** | Glassmorphic 5-step Tauri installer; service registration, PATH, registry |
| **apps/veloce-run** | CLI launcher (`--name`, `--hostname`, `--cpu`, `--mem`, `--restarts`, `--watch`, `--detach`) + `mesh identity/join/peers/leave` + `policy show/reload` |

### Out of Scope (Future Releases)

| Feature | Target |
|---|---|
| Security Audit 2 of 3 — second structured audit cycle | v0.8 |
| Security Audit 3 of 3 — final audit + optimisation & profiling | v0.9 |
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

### v0.4.0 ✅ Released

- [x] `crates/veloce-mesh` — Noise_IK P2P encrypted mesh (same crypto as WireGuard, zero-admin)
- [x] Multi-machine `.vln` namespace via join-code pairing + LWW gossip protocol
- [x] `veloce-run mesh identity / join / peers / leave` CLI subcommands
- [x] Dashboard mesh UI (This Machine card + Connected Peers table)
- [x] Security fixes: DNS compression DoS, OsRng PSK, identity key file ACL

### v0.5.0 ✅ Released

- [x] `crates/veloce-core/src/policy.rs` — PolicyEngine: TOML-backed RBAC + mesh ACL rules, hot-reload
- [x] `crates/veloce-mesh/src/stun.rs` — minimal RFC 5389/8489 STUN binding-request client
- [x] VM2 multi-address join code format (pub_key + n_addrs + [family+ip+port]* + timestamp)
- [x] `decode_join_code_addrs()` — transparent VM1 + VM2 backward-compatible decoding
- [x] STUN background task: upgrades join code cache from VM1 → VM2 at startup when WAN ≠ LAN
- [x] `connect_to_peer()` address racing: 250 ms stagger, first connection wins
- [x] Mesh ACL callback injected into `PeerConnection` — filters gossip entries at forwarder install
- [x] IPC message types `0x70–0x72` (PolicyGetRules, PolicyRulesResult, PolicyReload)
- [x] `ErrorCode::PolicyDenied (11)`
- [x] Capability enforcement in SpawnNode, KillNode, NetRegisterHost handlers
- [x] SDK: `policy_get_rules()`, `policy_reload()` on `VeloceClient`
- [x] `veloce-run policy show / reload` CLI subcommands
- [x] Bug fix: `pipe_security` TOKEN_USER buffer alignment (startup crash on x64)

### v0.7.0 ✅ Released

**Security Audit 1 of 3 — IPC Hardening, Mesh Hardening, Network Scope Restriction**

Nine findings (C1, C2, H1, H2, H3, H4, M1, M2, M3) identified and remediated. No new features.

- [x] **C1** — `pipe_security`: `assert_client_is_owner` returns kernel-verified Win32 exe path via `QueryFullProcessImageNameW`; `PolicyRule.exe` field matched against verified path
- [x] **C2** — `policy`: `compute_max_caps(exe_path)` grants server-authoritative capability set at handshake; per-handler `check_capability` calls removed
- [x] **H1** — `ipc_server`: pre-auth state machine drops non-`Handshake` messages before authentication completes
- [x] **H2** — `dns`: bind address changed from `0.0.0.0` to `127.0.0.1`
- [x] **H3** — `dns`: `forward_query` validates reply source against upstream address; spoofed responses discarded
- [x] **H4** — `noise`: `tokio::time::timeout(10s)` applied to both `initiator_handshake` and `responder_handshake` read steps
- [x] **M1** — `socks5`: non-VLN `CONNECT` targets rejected with `REP_UNREACHABLE`; proxy scoped to `.vln`/`.veloce` only
- [x] **M2** — `registry`: `assert!` size guards replaced with `anyhow::ensure!` for graceful error propagation
- [x] **M3** — `ipc_server`: `VELOCE_SKIP_PSK=1` silently ignored when `server_sid == "S-1-5-18"` (SYSTEM account)
- [x] `veloce-ipc`: `PolicyRuleMsg` gains `exe: Option<String>` with `#[serde(default)]`
- [x] Workspace version bumped to `0.7.0`

---

### v0.6.0 ✅ Released

**Backend — Traffic Instrumentation**
- [x] `veloce-ipc`: `TrafficQuery (0x80)`, `TrafficStatsResult (0x81)` message types; `TunnelTrafficMsg`, `HostTrafficMsg`, `TrafficStatsMsg` structs
- [x] `veloce-mesh`: `Arc<AtomicU64>` `tx_bytes`/`rx_bytes` per `PeerConnection`; incremented in writer (post-encrypt) and reader (post-read) tasks; `traffic_snapshot()`; `MeshState::query_traffic_stats()`
- [x] `veloce-net`: `Arc<AtomicU64>` `bytes_proxied` per `NetRecord`; incremented in SOCKS5 copy loop for `.vln` routes; `NetRegistry::traffic_snapshot()`; `veloce-ipc` added as dependency
- [x] `veloce-core`: `Body::TrafficQuery` IPC handler
- [x] `veloce-sdk`: `VeloceClient::query_traffic()` method
- [x] Dashboard Tauri backend: `traffic_stats`, `policy_show`, `policy_reload_cmd` commands; 2-second background push of `"traffic-update"` event

**Frontend — Svelte 5 + Canvas 2D Rewrite**
- [x] Migrated from vanilla JS to **Svelte 5** (`mount()` API); `@sveltejs/vite-plugin-svelte` Vite plugin
- [x] `stores.js` — 9 reactive stores; `topoPositions` persisted to `localStorage`
- [x] `lib/canvas.js` — `drawCircle`, `drawRect`, `drawEdge`, `drawSparkline`, `trafficColor`, `bytesPerSec`
- [x] `lib/tauri.js` — typed `invoke()` wrappers for all backend commands
- [x] `App.svelte` — shell layout, global CSS, resource polling, event listeners
- [x] `NodesTab.svelte` — inline sparklines (80×22 px) per row; click-to-expand detail panel with 120-point history graphs
- [x] `TemplatesTab.svelte` — template CRUD table + save form
- [x] `NetworkTab.svelte` — register/unregister host, mesh identity + peer connect, collapsible policy panel
- [x] `LogsTab.svelte` — live log viewer with search, stream toggles, timestamp toggle, auto-scroll, 5 000-line cap
- [x] `TopologyTab.svelte` — drag-and-drop topology canvas; edge width/colour by traffic; 60-cell heatmap; live counter tables

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
| Mesh peer behind strict NAT (WAN) | Low | STUN WAN discovery + VM2 join codes shipped in v0.5; symmetric NAT still requires manual port forward to :7474 |
| Identity key file compromise | Medium | File ACL (read-only, owner-only) set at creation |

---

## 10. Timeline

| Milestone | Status |
|---|---|
| v0.1.0 (Core + SDK + Dashboard) | ✅ Released |
| v0.2.0 (Installer, Templates, Resources, Health, Logs) | ✅ Released |
| v0.3.0 (veloce-run CLI + AppContainer — Phase 1 complete) | ✅ Released |
| v0.4.0 (Multi-Machine VeloceNet P2P mesh) | ✅ Released |
| v0.5.0 (Policy Engine + STUN WAN mesh) | ✅ Released |
| v0.6.0 (Dashboard v2 — Svelte 5, Canvas 2D, traffic instrumentation) | ✅ Released |
| v0.7.0 (Security Audit 1 of 3 — IPC, mesh, DNS/SOCKS5 hardening) | ✅ Released |
| v0.8.0 (Security Audit 2 of 3) | Q2 2026 |
| v0.9.0 (Security Audit 3 of 3 + optimisation & profiling) | Q3 2026 |
| v1.0 (WireGuard-NT kernel driver + signed auto-update installer) | Q4 2026 |
| v2.0 (Linux port + unified SDK bindings) | 2027 |

---

## 11. Forward Roadmap Detail

### Phase 2 — Mesh, Automation & Dashboard v2 (v0.5 – v0.6) ✅ Complete

**Policy Engine ✅ v0.5**
- Tier 1 — Process RBAC: declarative TOML rules governing which applications may request which capabilities (`SpawnNodes`, `NetRegister`, etc.); enforced server-side before any action is taken
- Tier 2 — Mesh ACLs: filter peer-gossiped `.vln` hostnames before forwarder installation; optionally scoped to a specific source peer
- `veloce-policy.toml` hot-reloaded via `veloce-run policy reload` — no service restart required
- Glob support (`"*"`, `"*.suffix"`) in both app names and hostnames

**STUN WAN Mesh ✅ v0.5**
- At startup, VeloceCore sends a STUN Binding Request (RFC 5389/8489) to discover the external IP
- Join code upgraded from VM1 (single LAN address) to VM2 (LAN + WAN addresses) when WAN ≠ LAN
- `connect_to_peer()` races all addresses with 250 ms stagger — LAN wins when on same network, WAN wins across NAT
- No manual port forwarding required for typical home/office NAT (symmetric NAT still requires `:7474` port forward)

**Dashboard v2 ✅ v0.6**
- Svelte 5 + Canvas 2D rewrite — zero new JS runtime dependencies
- Full backend traffic instrumentation: `Arc<AtomicU64>` byte counters on every Noise peer and every `.vln` hostname; pushed to frontend every 2 s via Tauri event
- Drag-and-drop topology canvas — edge width and colour represent live traffic rate
- 60-cell per-peer traffic heatmap (2-minute rolling window)
- Inline CPU% sparklines per node row; click-to-expand 120-point history graphs
- Logs tab with search/filter, stdout/stderr toggles, auto-scroll, 5 000-line cap
- Collapsible Policy Engine panel with App Rules and Mesh ACL tables; one-click hot-reload

---

### Phase 3 — Security Audit Cycle (v0.7 – v0.9) ✅ v0.7 Complete

This phase produces no new user-facing features. Its sole purpose is security, correctness, and performance. Three sequential structured audits are planned, each producing targeted remediations with no feature work.

**v0.7 — Security Audit 1 of 3 ✅ Complete**
- Audit scope: IPC security, mesh handshake, DNS/SOCKS5 network surface, policy engine identity model
- Nine findings identified (C1, C2, H1, H2, H3, H4, M1, M2, M3) and fully remediated
- See `RELEASE_NOTES_v0.7.0.md` for the complete finding-by-finding breakdown

**v0.8 — Security Audit 2 of 3**
- Second structured audit; scope TBD after v0.7 findings review
- Areas under consideration: mesh gossip integrity, Noise transport replay window, SOCKS5 auth options, dashboard Tauri IPC surface, installer privilege handling
- All confirmed findings remediated and shipped as v0.8.x patch releases as discovered

**v0.9 — Security Audit 3 of 3 + Optimisation**
- Final pre-1.0 security audit; full-surface review incorporating lessons from audits 1 and 2
- CPU and memory profiling under sustained load; hot paths optimised
- IPC message throughput tuned (batch encoding, buffer sizing)
- Mesh reconnect stability under network interruption and flapping
- Dashboard canvas render loop profiled; overdraw and recompute eliminated
- Documentation, inline comments, and public API surface reviewed for clarity and completeness

---

### Phase 4 — Linux Engine Swap (v2.0)

- Replace named pipes → Unix domain sockets (same VELC framing, same SDK API)
- Replace Job Objects → cgroups v2 (CPU quota + `memory.max`) + process groups for clean tree kill
- Unprivileged user namespaces for rootless sandboxing (equivalent to AppContainer on Linux)
- Swappable backend: `veloce-core` auto-detects OS at compile time and links the right driver module
- Unified Python, Node.js, and Go SDK bindings via the existing C FFI layer

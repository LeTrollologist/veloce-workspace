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

### In Scope (v0.1 – v0.9)

| Component | Features shipped |
|---|---|
| **veloce-core** | Background Windows service, SID ACL + OsRng PSK, Job Objects (CPU/memory/lifetime), mmap registry, health-loop, AppContainer kernel sandbox, MeshState, mesh TCP server (:7474), PolicyEngine (TOML RBAC + mesh ACLs, hot-reload, kernel-verified `exe` field); `TrafficQuery` IPC handler; pre-auth state machine; server-authoritative capability grant; `VELOCE_SKIP_PSK` SYSTEM guard; `MeshManage`/`PolicyAdmin` capabilities; `require_cap` on all sensitive handlers; `quote_arg` backslash escaping |
| **veloce-net** | DNS :5354 for `*.vln`/`*.veloce` (localhost-bind, upstream source + transaction-ID validation, `qdcount` cap); SOCKS5 :1055 (VLN-only scope); TTL GC; DNS compression DoS fix; `Arc<AtomicU64>` `bytes_proxied` per `NetRecord`; `NetRegistry::traffic_snapshot()`; DNS forward socket loopback-bound |
| **veloce-ipc** | VELC framing, bincode encoding, message types 0x00–0x72 + `TrafficQuery (0x80)` / `TrafficStatsResult (0x81)` + `MeshPingPeer (0x58)` / `MeshPingResult (0x59)`; `TunnelTrafficMsg`, `HostTrafficMsg`, `TrafficStatsMsg`; `PolicyRuleMsg.exe` optional field |
| **veloce-sdk** | `VeloceClient` async Rust client, C FFI (`veloce_sdk.dll`), template methods, mesh methods, `policy_get_rules()`, `policy_reload()`, `query_traffic()`, `mesh_ping_peer()` |
| **veloce-mesh** | x25519 identity (Noise_IK_25519_ChaChaPoly_BLAKE2s), PeerConnection gossip (LWW CRDT + clock-skew guard + per-peer size cap + periodic re-sync), TCP forwarder, STUN WAN discovery, VM2 join codes, VM3 join codes (TTL + one-time nonce blacklist), gossip ownership tracking, ACL callback; `Arc<AtomicU64>` `tx_bytes`/`rx_bytes` + `Arc<AtomicU32>` `latency_ms` per `PeerConnection`; `MeshState::query_traffic_stats()`; 10-second handshake timeout |
| **apps/dashboard** | Svelte 5 + Canvas 2D — nodes + sparklines + history graphs, templates, logs (search/filter/auto-scroll/5k-cap), VeloceNet + policy panel, topology canvas (drag-and-drop, heatmap, live traffic tables) |
| **apps/installer** | Glassmorphic 5-step Tauri installer; service registration, PATH, registry |
| **apps/veloce-run** | CLI launcher (`--name`, `--hostname`, `--cpu`, `--mem`, `--restarts`, `--watch`, `--detach`) + `mesh identity/join/peers/leave/status/diagnose/ping` + `policy show/reload` |

### Completed Beyond v0.9 (v1.0 – v3.0)

| Feature | Released |
|---|---|
| WireGuard-NT kernel driver (optional perf backend, one-time admin elevation) | ✅ v1.0 |
| NRPT `.vln` routing (system-wide DNS without VELOCE_DNS) | ✅ v1.0 |
| Signed installer + Tauri auto-update | ✅ v1.0 |
| GitHub Actions CI/CD pipeline | ✅ v1.0 |
| `SECURITY.md` + `VELOCE_PRINCIPALS.md` manifesto | ✅ v1.0 |
| Veloce Compose (`veloce-compose.yml`, `veloce up/down`, port publishing, env injection, health probes) | ✅ v1.1 |
| Persistence & Secrets (named volumes, bind mounts, DPAPI-backed runtime secrets) | ✅ v1.2 |
| Rolling Deployments & Desired State (reconciler, rolling/recreate strategies, `veloce status`) | ✅ v1.3 |
| Server-Signed VM3 Join Codes (`MeshGetJoinCodeV3` / `MeshJoinCodeV3Result` IPC) | ✅ v1.3.1 |
| Linux engine parity (cgroups v2, Unix domain sockets, systemd) | ✅ v2.0 |
| HTTP Ingress & Service Routing (Layer-7 reverse proxy, host/path routing) | ✅ v2.1 |
| Multi-Node Control Plane (Cluster coordinator, term tracking, replica scheduling) | ✅ v3.0 |

### Out of Scope (Future Roadmap)

| Feature | Target |
|---|---|
| Python / Node.js / Go SDK bindings | Future |
| Horizontal Process Autoscaler (HPA) | Future |
| CronJobs & DaemonSet-equivalent scheduling | Future |
| Veloce Hub — package manager (signed `.vpack` bundles, Helm-style values) | Future |

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

### v0.8.0 ✅ Released

**Security Audit 2 of 3 — IPC Capability Enforcement, Arg Injection, DNS/Gossip Hardening**

Seven findings (N1–N7) identified and remediated. No new features.

- [x] **N1** — `ipc_server`: `require_cap` added to `NetUnregisterHost`, `MeshConnect`, `MeshDisconnect`, `PolicyReload`; new `Capability::MeshManage` and `Capability::PolicyAdmin` variants
- [x] **N2** — `job`: `quote_arg` now escapes trailing backslashes per MSVC `CommandLineToArgvW` spec; prevents argument injection for node paths ending in `\`
- [x] **N3** — `dns`: `forward_query` captures outgoing transaction ID and validates it against the upstream response; mismatched IDs discarded
- [x] **N4** — `peer`: gossip entry timestamps validated against local `SystemTime` (±5 min skew); far-future entries that would permanently win LWW comparisons are discarded
- [x] **N5** — `peer`: `MAX_PEER_MSG_BYTES` guard before `serde_json::from_slice`; `RegistrySync` entry list capped at 1,000 entries per message
- [x] **N6** — `dns`: `Vec::with_capacity(qdcount)` capped at 5; prevents heap amplification from crafted DNS packets
- [x] **N7** — `dns`: ephemeral forward socket rebound from `0.0.0.0:0` to `127.0.0.1:0`
- [x] Workspace version bumped to `0.8.0`

### v0.9.0 ✅ Released

**Security Audit 3 of 3 + Mesh Improvements**

- [x] **S1 — VM3 join codes**: new `join_code_v3()` / `decode_vm3()` in `veloce-mesh::identity`; format adds `created_at[8 LE]`, `nonce[16]`, `ttl_mins[2 LE]`, `flags[1]` after the VM2 address list; `FLAG_ONE_TIME = 0x01`; `used_nonces: Mutex<HashSet<[u8;16]>>` replay blacklist in `MeshState`; `veloce-run mesh identity --ttl <mins> --one-time`
- [x] **S2 — Gossip ownership tracking**: `hostname_origins: Arc<ParkingMutex<HashMap<String, Uuid>>>` in `MeshState`; `make_owner_fn()` produces a sync `OwnerFn` closure injected into `PeerConnection`; conflicting origin attempts logged at `warn` and discarded
- [x] **O1 — Periodic gossip re-sync**: `gossip_interval_secs: u64` parameter to `PeerConnection::start()`; conditional `tokio::select!` arm ticks every 60 s and re-broadcasts the full local `NetRegistry` to the peer
- [x] **O3 — Mesh diagnostic CLI**: `MeshAction::Status`, `Diagnose`, `Ping { peer_id }` variants in `veloce-run`; `MeshPingPeer (0x58)` / `MeshPingResult (0x59)` IPC message types; `Arc<AtomicU32>` `latency_ms` per `PeerConnection`; `VeloceClient::mesh_ping_peer()` in SDK
- [x] Workspace version bumped to `0.9.0`

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
| Dependencies | No kernel drivers, no TAP adapters, no admin required (beyond service install); WireGuard-NT is an opt-in v1.0 perf backend that requires a one-time driver install — the userspace Noise_IK path is always available without it |
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
| v0.8.0 (Security Audit 2 of 3 — capability enforcement, arg injection, DNS/gossip hardening) | ✅ Released |
| v0.9.0 (Security Audit 3 of 3 + VM3 join codes, gossip ownership, mesh diagnostic CLI) | ✅ Released |
| v1.0 (WireGuard-NT + NRPT + signed installer + auto-update + CI/CD) | ✅ Released |
| v1.1 (Veloce Compose — declarative multi-service, port publishing, health probes) | ✅ Released |
| v1.2 (Persistence & Secrets — named volumes, bind mounts, DPAPI secrets) | ✅ Released |
| v1.3 (Rolling Deployments — desired-state reconciler, rolling/recreate strategies) | ✅ Released |
| v2.0 (Linux port — cgroups v2, Unix sockets, Python/Node/Go SDK bindings) | Q1–Q2 2028 |
| v2.1 (HTTP Ingress + TLS — L7 reverse proxy, ACME certs) | Q3 2028 |
| v2.2 (Autoscaling + Scheduling — HPA, CronJobs, DaemonSet-equivalent) | Q4 2028 |
| v2.3 (Veloce Hub — package manager, signed bundles, Helm-style values) | Q1 2029 |
| v3.0 (Multi-Node Control Plane — Raft, distributed desired state, StatefulSets, Namespaces) | Q2–Q3 2029 |

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

### Phase 3 — Security Audit Cycle (v0.7 – v0.9) ✅ Complete

This phase dedicated three sequential releases to security, correctness, and stability hardening. A total of 18 security findings were identified and remediated across all three audits (9 + 7 + 2 low findings closed in v0.9).

**v0.7 — Security Audit 1 of 3 ✅ Complete**
- Audit scope: IPC security, mesh handshake, DNS/SOCKS5 network surface, policy engine identity model
- Nine findings identified (C1, C2, H1, H2, H3, H4, M1, M2, M3) and fully remediated
- See `RELEASE_NOTES_v0.7.0.md` and `RELEASE_NOTES_v8-9.md` for details

**v0.8 — Security Audit 2 of 3 ✅ Complete**
- Post-v0.7 audit identified nine new findings (N1–N9); seven (N1–N7) remediated in v0.8.0
- Scope: IPC capability model completeness, node spawner argument injection, DNS transaction integrity, gossip timestamp validation, peer message size bounds, DNS allocation amplification, DNS forward socket interface scope
- Low findings N8/N9 (STUN validation) absorbed into v0.9

**v0.9 — Security Audit 3 of 3 + Mesh Improvements ✅ Complete**
- Remaining two low findings (N8 — STUN source validation, N9 — STUN magic cookie) addressed
- VM3 join codes: time-limited and one-time-use join codes with nonce replay prevention
- Gossip ownership tracking: prevents silent hostname squatting by connected peers
- Periodic gossip re-sync: 60-second ticker ensures convergence after transient failures
- New mesh diagnostic CLI commands: `status`, `diagnose`, `ping`

---

### v1.0.0 ✅ Released

**Production Release — NRPT, WireGuard-NT, Signed Installer**

- [x] NRPT `.vln` routing — `crates/veloce-core/src/nrpt.rs`; Windows Name Resolution Policy Table entry written at service install; every app, browser, and terminal resolves `.vln` without the `VELOCE_DNS` environment variable
- [x] WireGuard-NT optional kernel driver — one-time admin elevation during install; userspace Noise_IK remains the default; zero-admin promise preserved
- [x] Signed installer with Tauri auto-update delivery
- [x] GitHub Actions CI/CD pipeline (`.github/workflows/ci.yml`, `release.yml`)
- [x] `SECURITY.md` — comprehensive security policy, threat model, and trust boundaries
- [x] `VELOCE_PRINCIPALS.md` — design manifesto (zero-admin, zero kernel deps, encrypted mesh, capability-based security)
- [x] Workspace version bumped to `1.0.0`

---

### v1.1.0 ✅ Released

**Veloce Compose — Docker Compose Parity**

- [x] `veloce-compose.yml` declarative multi-service format; maps onto `SpawnNodeMsg` / `NetRegisterHost` / `RestartPolicy` IPC primitives
- [x] `veloce up` / `veloce down` CLI subcommands
- [x] Port publishing (`ports: ["8080:80"]`) — `TcpListener` in `veloce-net` forwards to node's `.vln` port
- [x] Environment injection (`environment:`, `--env`) — `Vec<(String,String)>` on `SpawnNodeMsg` fed into `CreateProcessW`
- [x] HTTP / TCP / exec health checks (`healthcheck:`) — extends health loop in `veloce-core/src/job.rs`
- [x] `depends_on:` startup ordering with `condition: service_healthy` support

### v1.2.0 ✅ Released

**Persistence & Secrets — Stateful Workloads**

- [x] Named volumes — `VolumeRegistry` maps name → NTFS path under `%PROGRAMDATA%\VeloceSolutions\volumes\`
- [x] Bind mounts — host path injection; AppContainer allowlist updated when `use_appcontainer: true`
- [x] DPAPI-backed runtime secrets — `SecretsVault` using `CryptProtectData`/`CryptUnprotectData`; injected at spawn, never plaintext on disk
- [x] `veloce secret set/rm/list` CLI; `ReadSecrets` capability enforced via policy engine

### v1.3.0 ✅ Released

**Rolling Deployments & Desired State**

- [x] Desired-state reconciler — `DesiredState` field on `CoreState`; cluster-level reconciliation loop in `veloce-core`
- [x] Rolling update strategy — drain one instance, await health check pass, drain next; zero-downtime deploys
- [x] Recreate strategy — stop all, start new version; for schema-migration workloads
- [x] `depends_on: condition: service_healthy` — blocks start until health check passes
- [x] `veloce ps` / `veloce status` — desired vs. actual replica counts, health state, last restart timestamp

---

### Phase 4 — Docker/K3s Competitive Parity (v2.0 – v3.0)

**v2.0 — Linux Engine Swap** ✅
- [x] Replace named pipes → Unix domain sockets (same VELC framing, same SDK API)
- [x] Replace Job Objects → cgroups v2 (`cpu.max` + `memory.max`) + process groups for clean tree kill
- [x] Unprivileged user namespaces for rootless sandboxing (equivalent to AppContainer on Linux)
- [x] `systemd` service registration replacing Windows service install path

**v2.1 — HTTP Ingress & Service Routing** ✅
- [x] `veloce-net/src/ingress.rs` module — Layer-7 HTTP reverse proxy listening on `:8080`
- [x] Host-based and path-based routing with longest-prefix matching (`--strip-prefix` support)
- [x] `veloce-run ingress add/rm/list` CLI subcommands + SDK methods

**v3.0 — Multi-Node Control Plane** ✅
- [x] `veloce-mesh/src/control.rs` — `ClusterCoordinator` with term tracking and leader election over Noise_IK mesh
- [x] Distributed replica assignment — `assign_replicas()` deterministically allocates service instances across active cluster nodes
- [x] Heartbeat term validation enforcing monotonicity across cluster peers

**Future Roadmap** 📋
- **v2.2 — Autoscaling & Scheduling**: Horizontal Process Autoscaler (HPA), CronJobs, and DaemonSet-equivalent scheduling
- **v2.3 — Veloce Hub Package Manager**: Signed `.vpack` bundles with Helm-style parameter templating and lifecycle hooks

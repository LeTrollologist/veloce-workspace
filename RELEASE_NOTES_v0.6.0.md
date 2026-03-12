# VeloceNetwork v0.6.0 — Release Notes

**Release:** v0.6.0
**Branch:** `feat/dashboard-v2-v0.6`
**Theme:** Dashboard v2 — Svelte + Canvas 2D + Full Backend Traffic Instrumentation

---

## Overview

v0.6.0 is the Dashboard v2 release. The frontend is rebuilt from scratch in **Svelte 5** with **raw Canvas 2D** rendering — zero new JavaScript runtime dependencies. Simultaneously, the entire Rust backend is instrumented with per-tunnel and per-host byte counters that stream live to the dashboard every 2 seconds, powering a real-time topology canvas, traffic heatmap, and per-node resource history graphs.

This release closes out **Phase 2** of the roadmap. The next sequence (v0.7–v0.9) is dedicated to hardening: bug testing, quality stabilisation, and optimisation before any new features land.

---

## What's New

### Backend — Traffic Instrumentation (Rust)

#### `veloce-ipc`
- New IPC message type `TrafficQuery` (discriminant `0x80`) — client sends to request a traffic snapshot
- New IPC message type `TrafficStatsResult` (discriminant `0x81`) — server replies with full stats
- New structs:
  - `TunnelTrafficMsg` — per-Noise-peer cumulative `tx_bytes` / `rx_bytes` + peer identity
  - `HostTrafficMsg` — per-`.vln`-hostname cumulative `bytes_proxied`
  - `TrafficStatsMsg` — envelope holding both lists + `ts_ms` timestamp for bps derivation

#### `veloce-mesh`
- `PeerConnection` now holds `Arc<AtomicU64>` `tx_bytes` and `rx_bytes` fields
- Writer task increments `tx_bytes` after every successful `noise::write_message` + `write_all`
- Reader task increments `rx_bytes` after every successful `read_exact` of an inbound cipher frame
- New `PeerConnection::traffic_snapshot() -> (u64, u64)` accessor
- New `MeshState::query_traffic_stats(host_stats: Vec<HostTrafficMsg>) -> TrafficStatsMsg` — aggregates all peer counters into a single response payload

#### `veloce-net`
- `NetRecord` gains `pub bytes_proxied: Arc<AtomicU64>` — shared via `Arc` so clones from `resolve()` point to the same counter
- SOCKS5 copy loop now increments `bytes_proxied` for `.vln`-routed connections (non-VLN passthrough traffic is not counted, as intended)
- New `NetRegistry::traffic_snapshot() -> Vec<HostTrafficMsg>` — snapshot of all registered hostname counters
- `veloce-ipc` added as a dependency of `veloce-net`

#### `veloce-core`
- `Body::TrafficQuery` handler in `ipc_server.rs`: calls `net_registry.traffic_snapshot()` and `mesh.query_traffic_stats()`, returns `Body::TrafficStatsResult`
- `Body::TrafficStatsResult` added to the server-to-client guard (returns `InvalidMessage` if sent by a client)

#### `veloce-sdk`
- New `VeloceClient::query_traffic() -> Result<TrafficStatsMsg>` method

#### Dashboard Tauri Backend (`apps/dashboard/src-tauri`)
- New Tauri command `traffic_stats` — on-demand traffic snapshot via SDK
- New Tauri command `policy_show` — returns current `PolicyRulesMsg`
- New Tauri command `policy_reload_cmd` — reloads policy from disk and returns new rules
- Background task in `connect()` pushes `"traffic-update"` Tauri event every **2 seconds**, delivering a fresh `TrafficStatsMsg` to the frontend without the UI needing to poll

---

### Frontend — Svelte 5 + Canvas 2D Rewrite

#### Migration
- Replaced vanilla JS + HTML string injection with **Svelte 5** component architecture
- Added `svelte` and `@sveltejs/vite-plugin-svelte` to the project — **zero additional runtime dependencies**
- `vite.config.js` updated with `svelte()` plugin
- `index.html` simplified to a bare `<div id="app">` mount point
- `main.js` updated to use Svelte 5's `mount()` API

#### `src/stores.js`
- Nine reactive Svelte stores: `connected`, `nodes`, `templates`, `meshInfo`, `logLines`, `resources`, `resourceHistory`, `traffic`, `trafficHistory`, `policy`
- `topoPositions` store with automatic `localStorage` persistence — drag positions survive page reloads

#### `src/lib/canvas.js`
- `drawCircle(ctx, x, y, r, fill, label)` — machine/peer node circles with centred labels
- `drawRect(ctx, x, y, w, h, fill, label)` — `.vln` host rectangles
- `drawEdge(ctx, x1, y1, x2, y2, width, color)` — tunnel edges with variable width and colour
- `drawSparkline(ctx, x, y, w, h, points, color, min, max)` — inline line graph with gradient fill
- `trafficColor(fraction)` — `hsl(120→0)` green-to-red gradient by traffic intensity
- `bytesPerSec(bytesNow, bytesPrev, tsNow, tsPrev)` — bps derivation from cumulative counter pairs

#### `src/lib/tauri.js`
- Typed `invoke()` wrappers for all 20 backend commands — single import for all components

#### `App.svelte`
- Shell layout: fixed header (brand + connection toggle), tab nav, scrollable content area
- Global CSS: dark theme (`#0d1117` background), button variants, input styles, card, table, badge classes
- Resource polling every 5 s via `setInterval` — computes `cpu_pct` delta from cumulative `cpu_ms` and populates per-node history up to 120 data points
- Event listeners: `"node-log"`, `"node-event"`, `"traffic-update"` Tauri events wired to stores
- Mesh info + policy auto-refresh every 10 s; ping heartbeat to keep connection indicator accurate

#### `NodesTab.svelte`
- Node table with inline **80×22 px Canvas 2D sparklines** showing last 30 CPU% poll points per node row
- Click any row to expand a **detail panel** with two **400×70 px Canvas 2D history graphs** (CPU% and Memory MB), animated at 60 fps via `requestAnimationFrame`
- Kill button per row; Spawn Node form with app name + executable fields

#### `TemplatesTab.svelte`
- Template list table with Spawn (▶) and Delete (✕) per row
- Save Template form: name, app name, executable, args, CPU%, RAM MB, max restarts, AppContainer toggle

#### `NetworkTab.svelte`
- Register Host / Unregister Host forms
- **This Machine** card with machine ID, join code (copyable), and listen port
- **Connect to Peer** form using VM1 or VM2 join codes
- **Connected Peers** table with name, peer ID, latency, remote hosts, Leave button
- **Policy Engine** panel — collapsible, shows App Rules table and Mesh ACL table with allow/deny badges; Reload button hot-reloads policy from disk

#### `LogsTab.svelte`
- Node selector dropdown; search/filter input; stdout / stderr / timestamp toggles; auto-scroll checkbox
- 5,000-line per-node cap (older lines purged automatically)
- Monospace log viewer with colour-coded `stdout` (light) vs `stderr` (red) lines

#### `TopologyTab.svelte`
- **Drag-and-drop Canvas 2D topology**: machine nodes as circles, `.vln` host nodes as rectangles; positions persisted to `localStorage`
- **Edge rendering**: width = `1 + log2(bps/1024)`; colour = green→red hsl gradient via `trafficColor()`; bps label on edge midpoint
- **60-cell traffic heatmap** per peer — 2-minute window at 2-second intervals; cells coloured by intensity relative to peak observed bps
- **Live traffic counter tables**: Tunnels section (peer name, TX, RX, current bps) and `.vln` Hosts section (hostname, cumulative bytes proxied)
- Hover highlight ring on nodes; drag-and-drop via `mousedown` / `mousemove` / `mouseup`

---

## IPC Discriminant Table (full)

| Discriminant | Name | Direction |
|---|---|---|
| `0x80` | `TrafficQuery` | Client → Server |
| `0x81` | `TrafficStatsResult` | Server → Client |

Previous discriminants (0x00–0x72) unchanged.

---

## Breaking Changes

None. All existing SDK methods, IPC discriminants, and CLI commands are backward compatible.

---

## Bug Fixes

None in this release. v0.7–v0.9 is the dedicated bug-fix and quality cycle.

---

## What's Next

| Version | Focus |
|---|---|
| **v0.7** | Heavy bug testing — systematic exercise of every code path; fix all discovered regressions |
| **v0.7.x** | Patch releases for individual bug fixes identified during v0.7 testing |
| **v0.8** | Feature meshing — ensure all subsystems (mesh, policy, dashboard, SDK, CLI) interoperate correctly end-to-end; fill any functional gaps |
| **v0.9** | Final optimisation pass, last-round bug fixes, performance profiling, pre-1.0 hardening |
| **v1.0** | WireGuard-NT kernel driver (perf upgrade) + signed installer with auto-update |

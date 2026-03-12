# VeloceNetwork v0.5.0 — Policy Engine + STUN WAN Mesh

This release ships **Phase 2 control plane**: a declarative policy engine for per-app capability
enforcement and mesh gossip filtering, plus automatic STUN-based WAN IP discovery that upgrades
the join code to a dual-address VM2 format — enabling cross-NAT mesh connections without manual
port forwarding.

---

## 🆕 What's New

### Policy Engine — Per-App RBAC + Mesh ACLs

VeloceCore now enforces a TOML policy file (`veloce-policy.toml` in the data directory) that
controls two independent dimensions:

**Tier 1 — Process RBAC**: which application names may request which capabilities.

```toml
# Block an untrusted sideloaded agent from spawning or killing nodes
[[rules]]
app  = "untrusted-agent"
deny = ["SpawnNodes", "KillNodes"]

# Lock a data pipeline app to read-only registry and net registration
[[rules]]
app   = "pipeline-worker"
allow = ["RegistryRead", "NetRegister"]
```

**Tier 2 — Mesh ACLs**: which hostnames received via peer gossip are installed as local
forwarders, optionally per-peer.

```toml
# Never install a forwarder for secret.vln regardless of which peer gossips it
[[mesh_acl]]
hostname = "secret.vln"
effect   = "deny"

# Reject *.internal hostnames from a specific untrusted peer
[[mesh_acl]]
hostname  = "*.internal"
from_peer = "DESKTOP-UNTRUSTED"
effect    = "deny"
```

**Behaviour:**
- File absent → allow-all (fully backward compatible — existing deployments are unaffected)
- Rules evaluated in order; first match wins; no match falls through to `default_effect` ("allow" by default)
- Hot-reloadable at runtime — no restart required
- Glob patterns: `"*"` (match any) and `"*.suffix"` (match any subdomain)

**New `veloce-run` subcommands:**

```
veloce-run policy show    → print current rules as a formatted table
veloce-run policy reload  → hot-reload veloce-policy.toml and confirm
```

**New error code:** `PolicyDenied (11)` — returned to the SDK caller when a capability check fails.

**New IPC message types:**

| Discriminant | Name | Direction |
|---|---|---|
| `0x70` | `PolicyGetRules` | Client → Core |
| `0x71` | `PolicyRulesResult` | Core → Client |
| `0x72` | `PolicyReload` | Client → Core |

**New SDK methods** on `VeloceClient`:

```rust
client.policy_get_rules().await? // → PolicyRulesMsg
client.policy_reload().await?    // → PolicyRulesMsg (rules after reload)
```

---

### STUN WAN Mesh — VM2 Multi-Address Join Codes

The v0.4 mesh only embedded the local LAN address in the join code (VM1 format). v0.5
automatically discovers the machine's WAN IP via STUN and upgrades to a VM2 join code
that carries **both** addresses — enabling peers behind NAT to connect without any manual
port forwarding configuration.

**How it works:**

1. At startup, VeloceCore binds a UDP socket and sends a STUN Binding Request (RFC 5389/8489)
   to `stun.l.google.com:19302` (falls back to `stun.cloudflare.com:3478`)
2. The XOR-MAPPED-ADDRESS attribute in the response gives the machine's external IP
3. If WAN IP ≠ LAN IP, the join code cache is upgraded from VM1 to VM2
4. `veloce-run mesh identity` shows the VM2 code and the discovered WAN address

```powershell
veloce-run mesh identity
# VM2:BBBB...==
# machine_id: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
# listening on port: 7474
# wan: 203.0.113.45  (via stun.l.google.com)

# If STUN is unreachable (offline / strict firewall):
# VM1:AAAA...==
# machine_id: ...
# listening on port: 7474
# (LAN only — STUN unreachable; WAN connections require manual port forward to :7474)
```

**VM2 join code format** (base64url-encoded payload):

```
[pub_key: 32 bytes]
[n_addrs: 1 byte]
  for each addr:
    [family: 1 byte  (4=IPv4, 6=IPv6)]
    [ip:     4|16 bytes]
    [port:   2 bytes  little-endian]
[timestamp: 8 bytes  little-endian]
```

**VM1 backward compatibility:** `decode_join_code_addrs()` transparently handles both
`VM1:…` and `VM2:…` codes — old single-address clients can still join.

**Connection racing:** when a VM2 code contains multiple addresses, `connect_to_peer()`
races them with a 250 ms stagger (LAN first, then WAN). The first successful connection
wins; the rest are dropped.

---

## 🔧 Bug Fixes

### Pipe Security: Misaligned TOKEN_USER Buffer (startup crash)

`pipe_security.rs` allocated a `[u8; 512]` stack buffer for `GetTokenInformation`, then
cast it to `*const TOKEN_USER`. On x64, `TOKEN_USER` contains a pointer field (`PSID`)
that requires 8-byte alignment, but a bare byte array is only 1-byte aligned. Depending
on stack layout, this caused a `misaligned pointer dereference` panic at startup — the
process would abort before the IPC server became ready.

**Fix:** The buffer is now wrapped in a `#[repr(align(8))]` newtype, guaranteeing the
pointer cast is always alignment-safe.

---

## 📦 Assets

| File | Description |
|---|---|
| `veloce-core.exe` | Windows background service (run elevated to install as a service) |
| `veloce-run.exe` | CLI launcher — `policy show/reload` + all previous subcommands |
| `veloce_sdk.dll` | C FFI library — exposes `policy_get_rules` / `policy_reload` |

---

## ✅ Full Feature Set (cumulative)

| Feature | Since |
|---|---|
| Windows Named-Pipe IPC + SID ACL | v0.1.0 |
| VeloceNet DNS (:5354) + SOCKS5 (:1055) | v0.1.0 |
| Job Objects (CPU / memory / lifetime) | v0.1.0 |
| Push events (Started / Exited / Crashed) | v0.1.0 |
| Shared mmap registry | v0.1.0 |
| Glassmorphic Tauri installer | v0.2.0 |
| Node Templates (save / spawn / delete) | v0.2.0 |
| Resource display (live CPU% + peak memory) | v0.2.0 |
| Health policies + exponential back-off restart | v0.2.0 |
| stdout/stderr log streaming | v0.2.0 |
| veloce-run CLI | v0.3.0 |
| AppContainer isolation | v0.3.0 |
| Multi-Machine VeloceNet (Noise_IK mesh) | v0.4.0 |
| DNS compression DoS fix | v0.4.0 |
| OsRng PSK (full 256-bit entropy) | v0.4.0 |
| Identity key file ACL | v0.4.0 |
| **Policy Engine (RBAC + mesh ACLs, TOML hot-reload)** | **v0.5.0** |
| **STUN WAN IP discovery + VM2 join code** | **v0.5.0** |
| **veloce-run policy show / reload** | **v0.5.0** |
| **SDK: policy_get_rules / policy_reload** | **v0.5.0** |
| **Pipe security alignment fix (startup crash)** | **v0.5.0** |

---

## 🗺️ What's Next — v0.6.0

- **Dashboard v2** — drag-and-drop topology canvas; live traffic heatmap (bytes/s per tunnel + per `.vln` host); historical resource graphs (CPU%, memory, restart counts); full log viewer panel with search and filter

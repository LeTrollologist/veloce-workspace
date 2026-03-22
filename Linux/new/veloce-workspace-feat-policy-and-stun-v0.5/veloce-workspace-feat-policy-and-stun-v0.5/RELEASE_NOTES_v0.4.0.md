# VeloceNetwork v0.4.0 — Multi-Machine VeloceNet

This release ships the **Phase 2 foundation**: an encrypted P2P mesh that transparently
extends the `.vln` private namespace across multiple machines — zero admin elevation,
zero VPN configuration, zero manual port rules.

---

## 🆕 What's New

### Multi-Machine VeloceNet — Noise_IK P2P Mesh

Two `veloce-core` instances on any two machines can now share their `.vln` namespaces
over a direct encrypted TCP tunnel, using the same cryptographic primitive as WireGuard.

**How it works in three commands:**

```powershell
# Machine A
veloce-run mesh identity
# VM1:AAA...==   ← share this with Machine B

# Machine B
veloce-run mesh join "VM1:AAA...=="
# ✓ connected to DESKTOP-A (peer_id=f3a1...)

# Machine A registers a service; Machine B resolves it instantly
veloce-run --hostname api.vln --port 8080 --detach -- node server.js
curl --proxy socks5://127.0.0.1:1055 http://api.vln/health   # from Machine B → 200 OK
```

**Implementation highlights:**

| Detail | Value |
|---|---|
| Cipher suite | Noise_IK_25519_ChaChaPoly_BLAKE2s |
| Key material | x25519 static keypair per machine, persisted in `veloce-identity.key` |
| Peer UUID | Derived as UUID v5 from the remote public key — stable across reconnects |
| Namespace sync | LWW (last-write-wins) gossip — no Raft, no coordination overhead |
| Transport | Plain TCP :7474 with Noise ciphertext, 2-byte length-prefixed frames |
| Forwarding | Transparent: each remote `.vln` host gets an ephemeral local port registered in `NetRegistry`; DNS and SOCKS5 are unchanged |
| Admin required | None — pure userspace, no kernel driver |

### `veloce-run mesh` Subcommands

```
veloce-run mesh identity          Print this machine's join code
veloce-run mesh join <CODE>       Connect to a remote peer
veloce-run mesh peers             List connected peers (name, latency, remote hosts)
veloce-run mesh leave <PEER_ID>   Disconnect from a peer
```

### Dashboard Mesh UI

The VeloceNet tab gains two new sections:

- **This Machine** — machine ID chip, join code (read-only field + Copy button), listen port
- **Connected Peers** — table showing peer name, latency, remote `.vln` hosts, and a Disconnect button
- Connect to a new peer by pasting a join code directly in the Dashboard

### New crate: `crates/veloce-mesh`

A self-contained P2P mesh library extracted from `veloce-core`:

```
veloce-mesh/
  identity.rs   — x25519 keypair, join-code encode/decode, file ACL
  noise.rs      — Noise_IK handshake + framed transport (initiator + responder)
  peer.rs       — PeerConnection: reader/writer tasks, LWW gossip, keepalive ping
  forward.rs    — per-hostname transparent TCP forwarder → Noise tunnel
  lib.rs        — MeshState, run_mesh_server
```

---

## 🔒 Security Fixes (shipped in this release)

### 1 — DNS Compression Pointer Loop DoS

The hand-rolled DNS parser in `veloce-net` followed compression pointer chains
without a depth limit. A malformed packet with a circular pointer (A→B→A) caused
an infinite loop in the DNS async task, blocking all `.vln` resolution.

**Fix:** Maximum 10 pointer jumps enforced; excess triggers an immediate parse error.

### 2 — PSK Entropy (full 256-bit)

The previous `generate_and_persist_psk()` XOR'd two `Uuid::new_v4()` values,
each of which fixes 6 bits (version + variant), yielding 244 effective bits of entropy.

**Fix:** Replaced with `rand::rngs::OsRng.fill_bytes(&mut psk)` — a direct OS CSPRNG
read giving the full 256 bits.

### 3 — Identity Key File ACL

`veloce-identity.key` (64-byte x25519 private key) is now created with a
`FILE_ATTRIBUTE_READONLY` + owner-only ACL on Windows and `chmod 600` on Linux,
preventing other processes running as the same user from silently overwriting it.

---

## 📦 Assets

| File | Description |
|---|---|
| `veloce-core.exe` | Windows background service (run elevated to install as a service) |
| `veloce-run.exe` | CLI launcher — wrap any exe into the mesh + mesh subcommands |
| `veloce_sdk.dll` | C FFI library for native integrations |

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
| **Multi-Machine VeloceNet (Noise_IK mesh)** | **v0.4.0** |
| **DNS compression DoS fix** | **v0.4.0** |
| **OsRng PSK (full 256-bit entropy)** | **v0.4.0** |
| **Identity key file ACL** | **v0.4.0** |

---

## 🗺️ What's Next — v0.5.0

- **Policy Engine** — process RBAC + mesh ACLs (`ALLOW node:api.vln TO node:db.vln ON PORT 5432`)
- **STUN WAN mesh** — extend the v0.4 LAN mesh across NAT / internet without manual port forwarding

# VeloceNetwork Security Policy (`SECURITY.md`)

## 1. Our Commitment

VeloceNetwork is engineered on a **Zero-Trust, Zero-Kernel, Least-Privilege** architecture. The entire runtime operates strictly in userspace without requiring kernel drivers, virtual network adapters, or elevated administrative privileges.

---

## 2. Scope & Subsystems

This policy covers all security aspects of VeloceNetwork (v1.0 – v4.2+):

* **Inter-Process Communication (IPC):** Named Pipes (Windows DACL / SID ACL) & Unix Domain Sockets (POSIX UID/GID validation) with per-session 256-bit `OsRng` PSKs.
* **Process Sandboxing:** Windows Job Objects, Windows AppContainers, Linux `cgroups v2`, and embedded zero-root WebAssembly (Wasm/WASI) sandboxing.
* **Encrypted Mesh Networking:** `Noise_IK_25519_ChaChaPoly_BLAKE2s` P2P tunnels, server-signed VM3 join codes with TTL and anti-replay nonces.
* **Userspace DNS & L7 Routing:** Private `*.vln` resolver, SOCKS5 egress filter, HTTP/HTTPS reverse proxy with automatic TLS certificate termination.
* **Enterprise Identity & ZTNA (v3.8):** OpenID Connect (OIDC) PKCE authentication, group-based RBAC, and server-side Mesh ACL bindings.
* **Kubernetes Telepresence Bridge (v4.0):** Unprivileged TCP egress interception and header-based live traffic shadowing (`X-Veloce-Intercept`, `X-Debug`).
* **Zero-Trust Team Share (v4.1):** Ephemeral VM3 share tokens (`vshare://...`) with cryptographic signatures and expiration TTLs.
* **OpenTelemetry Observability (v4.2):** W3C Trace Context propagation (`traceparent`), in-memory span ring buffers, and authenticated OTLP exporters.

---

## 3. Reporting a Vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

Please report any vulnerability or security concern directly to:

* **Security Contact:** `trollologistog@gmail.com`

**Please include:**
1. Description of the vulnerability and its potential impact.
2. Steps to reproduce or a proof-of-concept.
3. Target platform and affected version.

---

## 4. Threat Model & Trust Boundaries

| Boundary | What crosses it | Enforced Protections |
|---|---|---|
| **Named Pipe / Unix Socket (IPC)** | Client ↔ `veloce-core` | Kernel-verified SID DACL / POSIX UID check, 256-bit `OsRng` PSK, server-enforced Capability Grant |
| **Mesh TCP `:7474`** | Peer `veloce-core` ↔ `veloce-core` | Noise_IK mutual authentication, 10s handshake timeout, VM3 token TTL & one-time nonce blacklist |
| **Userspace DNS `:5354`** | Local applications ↔ VeloceNet | Localhost-only bind (`127.0.0.1`), upstream response transaction ID & source IP validation |
| **SOCKS5 Proxy `:1055`** | Local applications ↔ VeloceNet | Localhost-only bind (`127.0.0.1`), strict `.vln` / `.veloce` destination scope restriction |
| **Layer-7 Ingress `:8080` / `:8443`** | External / Local HTTP(S) | Longest-prefix matching, TLS termination with SAN validation, header stripping |
| **Wasm / WASI Runtime** | Untrusted guest code ↔ VeloceCore | Pure-Rust `wasmi` interpreter, strict linear memory caps, restricted WASI Preview 1 host calls |
| **Zero-Trust Team Share** | Peer ↔ Peer Port Tunnel | Scoped VM3 token encryption, single-use flags, dynamic DNS synthesis (`*.shared.vln`) |
| **Kubernetes Bridge** | Cloud Pod ↔ Local Host | Noise mesh encapsulation, header filter isolation, zero cluster elevation |

---

## 5. IPC Capability Model (Least Privilege)

Every client connecting to `VeloceCore` must request specific capabilities during the initial handshake. `VeloceCore` validates the client executable's image path against `veloce-policy.toml` and grants a subset of capabilities:

| Capability | Scope & Permissions | Added In |
|---|---|:---:|
| `SpawnNodes` | Launch child processes under Job Objects / cgroups | v1.0.0 |
| `KillNodes` | Terminate running child process nodes | v1.0.0 |
| `RegistryRead` | Read key-values from shared mmap registry | v1.0.0 |
| `RegistryWrite` | Write key-values to shared mmap registry | v1.0.0 |
| `NetRegister` | Register hostnames in userspace DNS (`*.vln`) and ingress | v1.0.0 |
| `NetResolve` | Resolve `.vln` domain names to local/mesh endpoints | v1.0.0 |
| `MeshManage` | Connect/disconnect P2P mesh peers, issue VM3 join codes | v0.8.0 |
| `PolicyAdmin` | Hot-reload server-side RBAC and Mesh ACL policies | v0.8.0 |
| `SecretsRead` | List registered secret names in the vault | v1.2.0 |
| `SecretsWrite` | Store or delete encrypted secrets in the vault | v1.2.0 |
| `NetPortForward` | Configure TCP port forward rules | v1.1.0 |
| `DesiredStateManage` | Apply declarative compose specs & trigger reconciler | v1.3.0 |
| `HubManage` | Manage Veloce Hub catalog & deploy pre-packaged apps | v3.4.0 |
| `MeshKvManage` | Read/write P2P replicated mesh key-value store | v3.5.0 |
| `BridgeManage` | Manage Kubernetes telepresence tunnels & intercepts | v4.0.0 |
| `ShareManage` | Create, manage, and consume ephemeral Zero-Trust team shares | v4.1.0 |
| `TraceRead` | Read distributed traces & span waterfall graphs | v4.2.0 |
| `TraceAdmin` | Configure OTLP export streaming & clear trace buffers | v4.2.0 |

---

## 6. Cryptography Standards & Implementations

* **P2P WireGuard-Grade Mesh:** `Noise_IK_25519_ChaChaPoly_BLAKE2s` with ephemeral session keys.
* **Package Signing:** `Ed25519` digital signatures with SHA-256 manifest hashing for `.vpack` archives.
* **Local Secrets Vault:** Windows DPAPI (`CryptProtectData`) on Windows; ChaCha20-Poly1305 with machine-derived key on Linux.
* **Enterprise SSO:** OpenID Connect with PKCE (RFC 7636) and SHA-256 code challenge.
* **Distributed Tracing:** W3C Trace Context Specification (v1.0) with 128-bit trace IDs and 64-bit span IDs.

# 💻 VeloceNetwork — CLI Commands Reference (`COMMANDS.md`)

The primary command-line tool for VeloceNetwork is `veloce-run`. This reference provides an exhaustive guide to every subcommand, flag, and usage pattern.

---

## 📑 Command Summary

| Command | Purpose | Key Subcommands |
|---|---|---|
| `veloce-run [FLAGS] -- <CMD>` | Launch a managed process node into the mesh | `--name`, `--hostname`, `--port`, `--cpu`, `--mem`, `--watch` |
| `veloce-run up / down / ps` | Multi-service orchestration (`veloce-compose.yml`) | `up -d`, `down`, `ps`, `status` |
| `veloce-run share` | Zero-Trust Team Share via VM3 codes | `share <PORT>`, `connect`, `list`, `revoke`, `join` |
| `veloce-run trace` | OpenTelemetry (OTel) distributed tracing | `list`, `inspect <ID>`, `export`, `clear` |
| `veloce-run bridge` | Kubernetes unprivileged telepresence bridge | `connect`, `intercept`, `list`, `disconnect` |
| `veloce-run wasm` | Sandboxed WebAssembly (Wasm/WASI) execution | `run <FILE>`, `inspect <FILE>` |
| `veloce-run auth / login` | Enterprise OIDC Single Sign-On (SSO) | `login`, `status`, `logout` |
| `veloce-run mesh` | P2P Noise_IK encrypted mesh management | `identity`, `join`, `peers`, `kv`, `status`, `ping` |
| `veloce-run ingress` | Layer-7 HTTP/HTTPS reverse proxy | `add`, `rm`, `list` |
| `veloce-run pack` | Standalone `.vpack` signed package manager | `keygen`, `init`, `build`, `verify`, `run`, `extract` |
| `veloce-run secret` | DPAPI/ChaCha encrypted runtime secrets vault | `set`, `get`, `rm`, `list` |
| `veloce-run autoscale` | Horizontal Process Autoscaler (HPA) | `set`, `get`, `rm` |
| `veloce-run cron` | Scheduled cron job task executor | `create`, `list`, `rm` |
| `veloce-run hub` | Veloce Hub application catalog & deploy | `search`, `publish`, `deploy` |
| `veloce-run portal` | Open web status portal & live console | `portal` (opens `http://localhost:9090`) |

---

## 1. Process Spawner (`veloce-run`)

Launch any executable or script into the private `.vln` mesh:

```bash
veloce-run [OPTIONS] -- <EXECUTABLE> [ARGS...]
```

### Options:
- `--name <NAME>`: Unique node name (e.g. `api-server`).
- `--hostname <HOST>`: Private `.vln` hostname (e.g. `api.vln`).
- `--port <PORT>`: Target local TCP port to map (e.g. `3000`).
- `--cpu <PERCENT>`: CPU throttling percentage (1–100%).
- `--mem <MB>`: Maximum working-set memory cap in MB.
- `--lifetime <SECONDS>`: Maximum wall-clock process runtime.
- `--restarts <COUNT>`: Maximum automatic restart attempts on crash.
- `--watch`: Stream real-time stdout/stderr to the terminal.
- `--detach`: Spawn in background, print node ID, and return immediately.
- `--use-appcontainer`: (Windows) Apply tighter Windows AppContainer kernel sandbox.

### Examples:
```bash
# Launch a Node.js server mapped to api.vln with 512MB RAM cap
veloce-run --name api --hostname api.vln --port 3000 --mem 512 -- node server.js

# Stream logs in real time
veloce-run --name worker --watch -- python worker.py
```

---

## 2. Zero-Trust Team Share (`veloce-run share` / `join`)

Publish and consume local ports securely across team members without opening firewall ports or port-forwarding:

### Subcommands:
- `veloce-run share <PORT|HOST>`: Create a share token for a local port.
  - `--name <NAME>`: Friendly share name (default: `share-<PORT>`).
  - `--ttl <DURATION>`: Expiration time (e.g. `30m`, `2h`, `1d`). Default: `1h`.
  - `--one-time`: Token is invalidated after the first connection.
- `veloce-run share connect <CODE>` or `veloce-run join <CODE>`: Connect to a share code.
- `veloce-run share list`: List all active outgoing and incoming shares.
- `veloce-run share revoke <SHARE_ID>`: Immediately revoke an active share.

### Examples:
```bash
# Developer A shares port 8080 for 2 hours
veloce-run share 8080 --name backend-api --ttl 2h

# Developer B joins using the token
veloce-run join vshare://vm3-eyJhbGciOi...
```

---

## 3. OpenTelemetry (OTel) Distributed Tracing (`veloce-run trace`)

Inspect live microservice latency waterfalls and export standard OTLP telemetry:

### Subcommands:
- `veloce-run trace list`: List recent distributed traces.
  - `--limit <N>`: Maximum traces to display (default: 20).
  - `--service <NAME>`: Filter by service name.
- `veloce-run trace inspect <TRACE_ID>`: Render ASCII waterfall latency chart.
- `veloce-run trace export`: Configure OTLP/HTTP export streaming.
  - `--endpoint <URL>`: OTLP endpoint (default: `http://localhost:4318/v1/traces`).
  - `--enable`: Turn on live streaming export.
  - `--disable`: Turn off live streaming export.
- `veloce-run trace clear`: Clear the in-memory trace ring buffer.

### Examples:
```bash
# Inspect a slow trace
veloce-run trace inspect 4bf92f3577b34da6a3ce929d0e0e4736

# Export traces to local Jaeger
veloce-run trace export --endpoint http://localhost:4318/v1/traces --enable
```

---

## 4. Kubernetes Telepresence Bridge (`veloce-run bridge`)

Connect local dev environment to a remote staging Kubernetes cluster and shadow traffic:

### Subcommands:
- `veloce-run bridge connect`: Connect to remote cluster bridge agent.
  - `--peer <ADDR>`: Mesh peer address of in-cluster bridge (e.g. `10.96.0.10:7474`).
  - `--namespace <NS>`: Target Kubernetes namespace (default: `default`).
- `veloce-run bridge intercept`: Shadow or intercept live traffic by header matching.
  - `--service <SVC>`: Cluster service name.
  - `--header <KEY:VAL>`: Header filter (e.g. `X-Debug: true` or `User-Agent: DevClient`).
  - `--target <PORT>`: Local port to receive shadowed traffic.
- `veloce-run bridge list`: List active bridges and interception rules.
- `veloce-run bridge disconnect <BRIDGE_ID>`: Disconnect active cloud bridge.

---

## 5. WebAssembly Execution (`veloce-run wasm`)

Execute sandboxed WebAssembly (WASI Preview 1) binaries:

### Subcommands:
- `veloce-run wasm run <FILE.wasm>`: Execute a Wasm module.
  - `--env <KEY=VAL>`: Pass environment variables.
  - `--arg <ARG>`: Pass CLI arguments to the Wasm module.
- `veloce-run wasm inspect <FILE.wasm>`: Display exported functions and imported host bindings.

---

## 6. Enterprise Identity & SSO (`veloce-run auth` / `login`)

Authenticate with corporate OpenID Connect (OIDC) identity providers:

### Subcommands:
- `veloce-run login`: Authenticate via browser PKCE flow.
  - `--provider <URL>`: OIDC Issuer URL (Entra ID, Okta, Keycloak).
  - `--client-id <ID>`: OAuth2 Client ID.
  - `--port <PORT>`: Local callback receiver port (default: `8989`).
- `veloce-run auth status`: View active OIDC session, email, roles, and token expiration.
- `veloce-run auth logout`: Clear tokens and revoke session.

---

## 7. P2P Mesh Management (`veloce-run mesh`)

Manage multi-machine WireGuard-grade Noise_IK encrypted mesh tunnels:

### Subcommands:
- `veloce-run mesh identity`: Generate VM3 join code.
  - `--ttl <MINUTES>`: Join code validity period (default: 60 min).
  - `--one-time`: Invalidate code after single use.
- `veloce-run mesh join <CODE>`: Connect to a remote peer machine.
- `veloce-run mesh peers`: List all connected peer nodes and transfer stats.
- `veloce-run mesh status`: Print cluster health, listen ports, and WAN address.
- `veloce-run mesh ping <PEER_ID>`: Measure round-trip ping latency to a peer.
- `veloce-run mesh leave <PEER_ID>`: Disconnect from a peer.
- `veloce-run mesh kv`: Replicated key-value store subcommands (`set`, `get`, `list`, `delete`).

---

## 8. Layer-7 HTTP/HTTPS Ingress (`veloce-run ingress`)

Configure local reverse proxy routing on port `:8080` (HTTP) and `:8443` (HTTPS):

### Subcommands:
- `veloce-run ingress add`: Add a routing rule.
  - `-H, --host <HOST>`: Match incoming Host header (e.g. `api.vln`).
  - `-p, --prefix <PATH>`: Match path prefix (e.g. `/v1`).
  - `-t, --target <PORT>`: Target local port.
  - `--strip-prefix`: Strip path prefix before forwarding.
  - `--tls`: Terminate with automatic TLS certificate.
- `veloce-run ingress list`: List all active routing rules.
- `veloce-run ingress rm <ROUTE_ID>`: Remove a routing rule.

---

## 9. Userspace `.vpack` Packager (`veloce-run pack`)

Package, cryptographically sign, and distribute standalone application archives:

### Subcommands:
- `veloce-run pack keygen --out <NAME>`: Generate an Ed25519 publisher keypair.
- `veloce-run pack init <DIR> --name <NAME>`: Create a default `vpack.toml` manifest.
- `veloce-run pack build <DIR> --out <FILE.vpack> --sign <KEY.priv>`: Build and sign a `.vpack` archive.
- `veloce-run pack inspect <FILE.vpack>`: View manifest, dependencies, and author.
- `veloce-run pack verify <FILE.vpack> --pubkey <KEY.pub>`: Verify cryptographic signature.
- `veloce-run pack extract <FILE.vpack> --dir <OUT_DIR>`: Extract archive contents.
- `veloce-run pack run <FILE.vpack> --name <NAME> --port <PORT>`: Execute archive directly into the mesh.

---

## 10. Secrets Vault (`veloce-run secret`)

Manage encrypted secrets injected securely into child nodes at spawn:

### Subcommands:
- `veloce-run secret set <NAME> <VALUE>`: Store an encrypted secret.
- `veloce-run secret get <NAME>`: Retrieve a secret.
- `veloce-run secret rm <NAME>`: Delete a secret.
- `veloce-run secret list`: List all stored secret keys.

---

## 11. Autoscaler (HPA) & CronJobs (`veloce-run autoscale` / `cron`)

- `veloce-run autoscale set <NAME> --min <MIN> --max <MAX> --cpu <TARGET_CPU>`: Configure autoscaling.
- `veloce-run cron create <NAME> -s "<CRON_EXPR>" -- <CMD>`: Schedule recurring tasks.
- `veloce-run cron list`: List scheduled cron jobs.
- `veloce-run cron rm <NAME>`: Remove a scheduled cron job.

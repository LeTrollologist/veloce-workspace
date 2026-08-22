# veloce-workspace

Monorepo containing the unified cross-platform codebase for **VeloceNetwork**.

## Repository Layout

```
veloce-workspace/
├── Windows/   ← Windows-native codebase (VeloceCore service, named-pipe IPC, Job Objects)
├── Linux/     ← Linux-native codebase (UnixSocket IPC, cgroups v2, systemd integration)
├── .github/   ← CI/CD workflows for Windows (MSVC) and Linux (Ubuntu) runners
└── Makefile   ← Sync targets to downstream production mirrors
```

## Production Mirrors

| Repo | Platform | Mirror of |
|------|----------|-----------|
| [VeloceNetwork-Windows](https://github.com/LeTrollologist/VeloceNetwork-Windows) | Windows | `Windows/` subtree |
| [VeloceNetwork-Linux](https://github.com/LeTrollologist/VeloceNetwork-Linux) | Linux | `Linux/` subtree |

## Feature Status & Roadmap Completion

| Milestone | Key Features | Status |
|---|---|:---:|
| **v1.0.0** | Core Engine, Named Pipe / Unix IPC, Job Objects, Userspace DNS/SOCKS5, Noise_IK Mesh, NRPT | ✅ Complete |
| **v1.1.0** | Veloce Compose (`veloce-compose.yml`), TCP Port Forwarding, Health Probes | ✅ Complete |
| **v1.2.0** | Stateful Workloads: Named Volumes, Bind Mounts, DPAPI Runtime Secrets Vault | ✅ Complete |
| **v1.3.0** | Rolling Deployments & Desired-State Reconciler Loop (`veloce ps`, `veloce status`) | ✅ Complete |
| **v1.3.1** | Server-Signed VM3 Join Codes (`MeshGetJoinCodeV3` / `MeshJoinCodeV3Result` IPC) | ✅ Complete |
| **v2.0.0** | Linux Engine Parity: Unix Sockets, `cgroups v2` controllers, `systemd` integration | ✅ Complete |
| **v2.1.0** | Layer-7 HTTP Ingress Reverse Proxy (`veloce-net/src/ingress.rs`, `veloce-run ingress`) | ✅ Complete |
| **v3.0.0** | Distributed Control Plane & Consensus (`ClusterCoordinator`, Term Tracking, Replica Scheduling) | ✅ Complete |

## Development Workflow

All development happens in `veloce-workspace`. Synchronize to production mirrors with:

```bash
make sync-windows   # push Windows/ subtree → VeloceNetwork-Windows
make sync-linux     # push Linux/ subtree → VeloceNetwork-Linux
make sync-prod      # sync both subtrees + pull production clones
```

## Releasing

```bash
make release-windows TAG=v3.0.0
make release-linux   TAG=v3.0.0
```

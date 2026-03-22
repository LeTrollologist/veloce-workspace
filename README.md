# veloce-workspace

Monorepo containing both platform codebases for **VeloceNetwork**.

## Repository Layout

```
veloce-workspace/
├── Windows/   ← Windows-native codebase (VeloceCore service, named-pipe IPC, Job Objects)
├── Linux/     ← Linux-native codebase (UnixSocket IPC, cgroups v2, systemd integration)
├── .github/   ← CI workflows for both platforms
└── Makefile   ← sync targets to production mirrors
```

## Production Mirrors

| Repo | Platform | Mirror of |
|------|----------|-----------|
| [VeloceNetwork-Windows](https://github.com/LeTrollologist/VeloceNetwork-Windows) | Windows | `Windows/` subtree |
| [VeloceNetwork-Linux](https://github.com/LeTrollologist/VeloceNetwork-Linux) | Linux | `Linux/` subtree |

## Development Workflow

All development happens here in `veloce-workspace`. Sync to production mirrors with:

```bash
make sync-windows   # push Windows/ subtree → VeloceNetwork-Windows
make sync-linux     # push Linux/ subtree → VeloceNetwork-Linux
make sync-prod      # sync both + pull production clones
```

## Releasing

```bash
make release-windows TAG=v2.1.0
make release-linux   TAG=v1.1.0
```

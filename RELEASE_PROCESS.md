# VeloceNetwork Release Process

Every update and release follows this standardized process. Run it locally — no CI/CD pipelines.

## Quick Reference

```bash
# Before every commit (daily dev)
make check TAG=v4.8.0

# Full release
make release TAG=v4.8.0
```

---

## Pipeline Stages

```
preflight → build → test → security → package → verify → publish
```

| Stage | Command equivalent | Must pass to continue |
|-------|-------------------|----------------------|
| **preflight** | check tools (rust, gh, WSL) | yes |
| **build** | `cargo build --release` Windows + Linux (WSL) | yes |
| **test** | `cargo test` + crypto invariants (`veloce-mesh`) | yes |
| **security** | `cargo audit` + SOC 2 + CycloneDX SBOM | no (warns, continues) |
| **package** | create archives + vpacks | yes |
| **verify** | SHA-256 all assets + `vpack t` + naming lint | yes |
| **publish** | `gh release create` (draft) → manual publish | yes |

---

## Canonical Asset Convention

Every release ships **exactly 4 assets + SHA256SUMS.txt** (5 total):

| File | Description |
|------|-------------|
| `veloce-windows-v{VER}-x86_64.zip` | Windows bundle |
| `veloce-linux-v{VER}-x86_64.tar.gz` | Linux bundle |
| `veloce-runtime-v{VER}-windows-x86_64.vpack` | Windows vpack |
| `veloce-runtime-v{VER}-linux-x86_64.vpack` | Linux vpack |
| `SHA256SUMS.txt` | SHA-256 of the 4 assets above |

**Naming rule:** `<name>-v{VER}-{platform}-{arch}` (version before platform — matches vpack-archiver convention).

Nothing else is uploaded as a release asset:
- No loose binaries
- No compliance docs (SOC 2, SBOM go in `dist/v{VER}/audit/`)
- No duplicate formats (Linux = tar.gz only, never zip)
- No alias names (no short platform names without arch)

---

## Step-by-Step

### 1. Update version

Bump `version` in all workspace `Cargo.toml` files:

```toml
# Windows/Cargo.toml, Linux/Cargo.toml
[workspace.package]
version = "4.8.0"
```

Commit:
```bash
git add */Cargo.toml */Cargo.lock
git commit -m "chore(release): bump version to v4.8.0"
```

### 2. Run the pipeline

```bash
make release TAG=v4.8.0
```

The pipeline will:
- Build Windows natively, Linux via WSL
- Run all tests including cryptographic invariant suite
- Run `cargo audit`, SOC 2 scan, generate CycloneDX SBOM
- Create canonical archives and vpacks in `dist/v4.8.0/`
- Verify SHA-256 and vpack integrity
- Create a **draft** GitHub release with all 5 assets

### 3. Review and publish

1. Open the draft release URL printed by the pipeline
2. Fill in the highlights section of the release body
3. Click **Publish release**

### 4. Tag and push

```bash
git tag v4.8.0
git push origin main --tags
```

---

## Resuming a Failed Pipeline

If a stage fails, fix the issue and resume from that stage:

```bash
# Example: package stage failed — fix it then resume
make check TAG=v4.8.0            # re-run build/test/security
python scripts/pipeline.py v4.8.0 --from package
```

---

## Outputs in `dist/v{VER}/`

```
dist/v4.8.0/
  windows/                    ← staged Windows binaries (not committed)
  linux/                      ← staged Linux binaries (not committed)
  veloce-windows-v4.8.0-x86_64.zip
  veloce-linux-v4.8.0-x86_64.tar.gz
  veloce-runtime-v4.8.0-windows-x86_64.vpack
  veloce-runtime-v4.8.0-linux-x86_64.vpack
  SHA256SUMS.txt
  release_body.md             ← GitHub release body (auto-generated)
  audit/
    soc2-audit.txt
    veloce-sbom-cyclonedx.json
    cargo-audit.txt
```

Binary staging dirs (`windows/`, `linux/`) are gitignored — only the final archives and audit outputs are tracked.

---

## Scripts Reference

| Script | Purpose |
|--------|---------|
| `scripts/pipeline.py` | Master orchestrator — use this for everything |
| `scripts/check.py` | Standalone build + test + security (no package/publish) |
| `scripts/release.py` | Standalone release-only driver (for repackaging existing binaries) |
| `Makefile` | Shortcuts — wraps `pipeline.py` |

---

## `.gitignore` entries (add if missing)

```
dist/v*/windows/
dist/v*/linux/
dist/v*/staging/
```

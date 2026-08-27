#!/usr/bin/env python3
"""
veloce-pipeline — standardized build, verify, security & release orchestrator.

Usage:
    python scripts/pipeline.py <tag> [options]

Options:
    --dry-run          Preview all actions, touch nothing
    --from <stage>     Start from a specific stage (skip earlier ones)
    --skip <s1,s2>     Skip named stages entirely
    --no-publish       Run everything except the GitHub upload

Stages (in order):
    preflight  verify tools are available
    build      cargo build --release for Windows (native) + Linux (WSL)
    test       cargo test + veloce-mesh crypto invariants
    security   cargo audit, SOC 2 scan, CycloneDX SBOM
    package    assemble archives (.zip / .tar.gz) + vpacks
    verify     sha256sum + vpack t integrity + naming lint
    publish    gh release create (draft) + upload 7 canonical assets

Examples:
    python scripts/pipeline.py v4.8.0
    python scripts/pipeline.py v4.8.0 --dry-run
    python scripts/pipeline.py v4.8.0 --from package
    python scripts/pipeline.py v4.8.0 --skip security --no-publish
"""

import argparse
import hashlib
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path
from datetime import datetime, timezone

# ── repo layout ───────────────────────────────────────────────────────────────
REPO_ROOT  = Path(__file__).parent.parent.resolve()
DIST_ROOT  = REPO_ROOT / "dist"
REPO_GH    = "LeTrollologist/veloce-workspace"
VPACK_ARCHIVER_RELEASE = "https://github.com/LeTrollologist/vpack-archiver/releases/download/v1.1.0/vpack-archiver-v1.1.0-windows-x86_64.zip"

IGNORED_ADVISORIES = ["RUSTSEC-2020-0071", "RUSTSEC-2020-0159"]

WORKSPACES = {
    "windows": REPO_ROOT / "Windows",
    "linux":   REPO_ROOT / "Linux",
}

PLATFORM_BINS = {
    "windows": ["veloce-core.exe", "veloce-run.exe", "veloce-launcher.exe",
                "veloce-shell.exe", "veloce_sdk.dll",
                "vpack-archiver.exe", "vpack-installer.exe"],
    "linux":   ["veloce-core", "veloce-run", "veloce-launcher",
                "veloce-shell", "libveloce_sdk.so",
                "vpack-archiver", "vpack-installer"],
}

ALL_STAGES = ["preflight", "build", "test", "security", "package", "verify", "publish"]

# ── helpers ───────────────────────────────────────────────────────────────────

class Pipeline:
    def __init__(self, tag: str, dry_run: bool, out: Path):
        self.tag     = tag
        self.dry_run = dry_run
        self.out     = out          # dist/v{VER}/
        self.errors: list[str] = []

    def run(self, cmd: list, cwd: Path | None = None,
            via_wsl: bool = False, check: bool = True, label: str = "") -> subprocess.CompletedProcess:
        if via_wsl:
            # Convert Windows path to WSL path for cwd
            wsl_cwd = str(cwd).replace("\\", "/").replace("C:", "/mnt/c") if cwd else None
            cmd = ["wsl", "--", "bash", "-c",
                   f"cd '{wsl_cwd}' && " + " ".join(str(c) for c in cmd)]
        display = label or " ".join(str(c) for c in cmd)
        print(f"    $ {display}")
        if self.dry_run:
            return subprocess.CompletedProcess(cmd, 0)
        r = subprocess.run([str(c) for c in cmd], cwd=(None if via_wsl else cwd))
        if check and r.returncode != 0:
            raise RuntimeError(f"Command failed (exit {r.returncode}): {display}")
        return r

    def copy(self, src: Path, dst: Path):
        if not self.dry_run:
            dst.parent.mkdir(parents=True, exist_ok=True)
            if src.exists():
                shutil.copy2(src, dst)
            else:
                print(f"    ~ not found (skipping): {src.name}")


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def header(title: str):
    bar = "─" * (len(title) + 4)
    print(f"\n┌{bar}┐")
    print(f"│  {title}  │")
    print(f"└{bar}┘")


# ── stage 1: preflight ────────────────────────────────────────────────────────

def stage_preflight(p: Pipeline):
    header("PREFLIGHT — Checking tools")
    ok = True

    def need(cmd, name, install=""):
        nonlocal ok
        found = shutil.which(cmd) is not None
        status = "OK " if found else "MISSING"
        print(f"    [{status}] {name}" + (f"  →  {install}" if not found and install else ""))
        if not found:
            ok = False

    need("cargo",         "Rust / Cargo",       "https://rustup.rs")
    need("gh",            "GitHub CLI",          "https://cli.github.com")

    # WSL check
    wsl = shutil.which("wsl") is not None
    print(f"    [{'OK ' if wsl else 'MISSING'}] WSL (for Linux builds)" +
          ("" if wsl else "  →  Enable WSL in Windows Features"))
    if not wsl:
        ok = False

    # vpack-archiver
    vpa = shutil.which("vpack-archiver") is not None or shutil.which("vpack") is not None
    print(f"    [{'OK ' if vpa else 'MISSING'}] vpack-archiver  →  {VPACK_ARCHIVER_RELEASE}" if not vpa else
          f"    [OK ] vpack-archiver")
    if not vpa:
        print("           vpack-archiver is optional for verify stage but needed for vpack t checks")

    # gh auth
    r = subprocess.run(["gh", "auth", "status"], capture_output=True, text=True)
    authed = r.returncode == 0
    for line in r.stdout.splitlines():
        if "Logged in" in line:
            print(f"    [OK ] gh auth: {line.strip()}")
            break
    if not authed:
        print("    [WARN] gh not authenticated — publish stage will fail")

    if not ok:
        raise RuntimeError("Preflight failed — install missing tools above and retry.")
    print("    All required tools present.")


# ── stage 2: build ────────────────────────────────────────────────────────────

def stage_build(p: Pipeline):
    header("BUILD — cargo build --release")

    for plat, ws in WORKSPACES.items():
        print(f"\n  [{plat.upper()}]")
        via_wsl = (plat == "linux")
        p.run(["cargo", "build", "--release", "--workspace"],
              cwd=ws, via_wsl=via_wsl,
              label=f"cargo build --release --workspace  ({ws.name}/)" +
                    (" [via WSL]" if via_wsl else ""))

        # Copy compiled binaries into dist/v{VER}/{plat}/
        staging = p.out / plat
        if not p.dry_run:
            staging.mkdir(parents=True, exist_ok=True)

        if plat == "windows":
            rel_dir = ws / "target" / "release"
        else:
            # WSL builds into the WSL filesystem; the target/ dir is mapped
            rel_dir = ws / "target" / "release"

        for bin_name in PLATFORM_BINS[plat]:
            p.copy(rel_dir / bin_name, staging / bin_name)

        # Docs
        for doc in ["README.md", "SECURITY.md", "veloce_sdk.h"]:
            p.copy(ws / doc, staging / doc)

        print(f"    Binaries staged in dist/{p.tag}/{plat}/")


# ── stage 3: test ─────────────────────────────────────────────────────────────

def stage_test(p: Pipeline):
    header("TEST — cargo test")

    for plat, ws in WORKSPACES.items():
        via_wsl = (plat == "linux")
        print(f"\n  [{plat.upper()}] cargo test --workspace")
        p.run(["cargo", "test", "--workspace"],
              cwd=ws, via_wsl=via_wsl,
              label=f"cargo test --workspace  ({ws.name}/)" +
                    (" [via WSL]" if via_wsl else ""))

    # Crypto invariant suite always on Linux
    print(f"\n  [CRYPTO INVARIANTS] cargo test -p veloce-mesh")
    p.run(["cargo", "test", "-p", "veloce-mesh", "--", "--nocapture"],
          cwd=WORKSPACES["linux"], via_wsl=True,
          label="cargo test -p veloce-mesh -- --nocapture  [via WSL]")


# ── stage 4: security ─────────────────────────────────────────────────────────

def stage_security(p: Pipeline):
    header("SECURITY — audit, SOC 2, SBOM")
    audit_dir = p.out / "audit"
    if not p.dry_run:
        audit_dir.mkdir(parents=True, exist_ok=True)

    # cargo audit
    print("\n  [ADVISORY] cargo audit")
    ignore_flags = " ".join(f"--ignore {a}" for a in IGNORED_ADVISORIES)
    lock = WORKSPACES["linux"] / "Cargo.lock"
    rc = subprocess.run(
        f"cargo audit --file {lock} {ignore_flags}",
        shell=True, text=True,
        capture_output=False,
    ).returncode if not p.dry_run else 0
    if rc != 0:
        print("    Advisory scan found issues — review above output.")

    # veloce-run security audit (SOC 2)
    runner = p.out / "linux" / "veloce-run"
    if not p.dry_run and not runner.exists():
        print("    [SKIP] veloce-run not staged yet — run build stage first")
    else:
        print("\n  [SOC 2] veloce-run security audit")
        soc2_out = audit_dir / "soc2-audit.txt"
        if not p.dry_run:
            r = subprocess.run([str(runner), "security", "audit"],
                               capture_output=True, text=True)
            soc2_out.write_text(r.stdout + r.stderr, encoding="utf-8")
            print(f"    Written: {soc2_out.name}")
        else:
            print(f"    [dry-run] Would write: {soc2_out.name}")

    # veloce-run security sbom (CycloneDX)
    print("\n  [SBOM] veloce-run security sbom")
    sbom_out = audit_dir / "veloce-sbom-cyclonedx.json"
    if not p.dry_run and runner.exists():
        r = subprocess.run([str(runner), "security", "sbom",
                            "--output", str(sbom_out)],
                           capture_output=True, text=True)
        if sbom_out.exists():
            print(f"    Written: {sbom_out.name}  ({sbom_out.stat().st_size:,} bytes)")
    else:
        print(f"    [dry-run] Would write: {sbom_out.name}")


# ── stage 5: package ──────────────────────────────────────────────────────────

def stage_package(p: Pipeline):
    header("PACKAGE — archives + vpacks")

    tag = p.tag

    for plat in ["windows", "linux"]:
        staging = p.out / plat
        print(f"\n  [{plat.upper()}]")

        if plat == "windows":
            archive_name = f"veloce-windows-{tag}-x86_64.zip"
            archive_path = p.out / archive_name
            print(f"    Creating {archive_name}")
            if not p.dry_run:
                with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED) as zf:
                    for f in staging.rglob("*"):
                        if f.is_file():
                            zf.write(f, f.relative_to(staging))
        else:
            archive_name = f"veloce-linux-{tag}-x86_64.tar.gz"
            archive_path = p.out / archive_name
            print(f"    Creating {archive_name}")
            if not p.dry_run:
                with tarfile.open(archive_path, "w:gz") as tf:
                    tf.add(staging, arcname=".")

        print(f"    archive -> {archive_name}" +
              ("" if p.dry_run else f"  ({archive_path.stat().st_size:,} bytes)"))

        # vpack
        suffix   = "windows-x86_64" if plat == "windows" else "linux-x86_64"
        vpack_name = f"veloce-runtime-{tag}-{suffix}.vpack"
        vpack_path = p.out / vpack_name
        runner_bin = "veloce-run.exe" if plat == "windows" else "veloce-run"
        runner     = staging / runner_bin

        print(f"    Building {vpack_name}")
        if not p.dry_run:
            if runner.exists():
                subprocess.run([str(runner), "pack", "init", str(staging),
                                "-n", "veloce-runtime"], check=False)
                subprocess.run([str(runner), "pack", "build", str(staging),
                                "-o", str(vpack_path)], check=False)
                if vpack_path.exists():
                    print(f"    vpack  -> {vpack_name}  ({vpack_path.stat().st_size:,} bytes)")
                else:
                    print(f"    [warn] vpack not produced — veloce-run pack build failed")
            else:
                print(f"    [skip] runner not found: {runner}")


# ── stage 6: verify ───────────────────────────────────────────────────────────

import re

ASSET_PATTERNS = [
    re.compile(r"^veloce-(windows|linux)-v\d+\.\d+\.\d+-(x86_64)\.(zip|tar\.gz)$"),
    re.compile(r"^veloce-runtime-v\d+\.\d+\.\d+-(windows|linux)-(x86_64)\.vpack$"),
    re.compile(r"^SHA256SUMS\.txt$"),
]

def stage_verify(p: Pipeline):
    header("VERIFY — sha256, vpack integrity, naming lint")
    tag = p.tag

    canonical_names = [
        f"veloce-windows-{tag}-x86_64.zip",
        f"veloce-linux-{tag}-x86_64.tar.gz",
        f"veloce-runtime-{tag}-windows-x86_64.vpack",
        f"veloce-runtime-{tag}-linux-x86_64.vpack",
    ]

    sha_lines = []
    all_ok = True

    print("\n  [SHA256]")
    for name in canonical_names:
        path = p.out / name
        if not p.dry_run:
            if path.exists():
                digest = sha256_file(path)
                sha_lines.append(f"{digest}  {name}")
                print(f"    {digest}  {name}")
            else:
                print(f"    [MISSING] {name}")
                all_ok = False
        else:
            print(f"    [dry-run] would hash: {name}")

    # Write SHA256SUMS.txt
    sha_path = p.out / "SHA256SUMS.txt"
    if not p.dry_run and sha_lines:
        sha_path.write_text("\n".join(sha_lines) + "\n", encoding="utf-8")
        print(f"\n    SHA256SUMS.txt written ({len(sha_lines)} entries)")

    # vpack integrity
    print("\n  [VPACK INTEGRITY]")
    vpa = shutil.which("vpack-archiver") or shutil.which("vpack")
    for name in [n for n in canonical_names if n.endswith(".vpack")]:
        path = p.out / name
        if p.dry_run:
            print(f"    [dry-run] would: vpack t {name}")
        elif vpa and path.exists():
            r = subprocess.run([vpa, "t", str(path)], capture_output=True, text=True)
            status = "OK " if r.returncode == 0 else "FAIL"
            print(f"    [{status}] {name}")
            if r.returncode != 0:
                all_ok = False
        else:
            print(f"    [skip] vpack-archiver not on PATH or file missing")

    # Naming lint
    print("\n  [NAMING LINT]")
    for name in canonical_names + ["SHA256SUMS.txt"]:
        matched = any(pat.match(name) for pat in ASSET_PATTERNS)
        print(f"    [{'OK ' if matched else 'FAIL'}] {name}")
        if not matched:
            all_ok = False

    if not all_ok:
        raise RuntimeError("Verify stage failed — see issues above.")
    print("\n    All assets verified.")


# ── stage 7: publish ──────────────────────────────────────────────────────────

def stage_publish(p: Pipeline):
    header("PUBLISH — GitHub release")
    tag = p.tag

    sha_path = p.out / "SHA256SUMS.txt"
    sha_content = sha_path.read_text(encoding="utf-8") if (not p.dry_run and sha_path.exists()) else "(checksums here)"

    body = f"""\
# VeloceNetwork {tag}

> Fill in highlights before publishing this draft.

---

### Platform Release Packages

| Platform | Archive | VPack |
| :--- | :--- | :--- |
| **Windows** x86\\_64 | `veloce-windows-{tag}-x86_64.zip` | `veloce-runtime-{tag}-windows-x86_64.vpack` |
| **Linux** x86\\_64   | `veloce-linux-{tag}-x86_64.tar.gz` | `veloce-runtime-{tag}-linux-x86_64.vpack` |

---

### Installation via VPack Archiver (recommended)

**1. Download [VPack Archiver v1.1.0](https://github.com/LeTrollologist/vpack-archiver/releases/tag/v1.1.0)**
and place the binary on your `PATH`.

**2. Verify**
```bash
vpack t veloce-runtime-{tag}-windows-x86_64.vpack   # Windows
vpack t veloce-runtime-{tag}-linux-x86_64.vpack     # Linux
```

**3. Extract**
```powershell
# Windows
vpack x veloce-runtime-{tag}-windows-x86_64.vpack -o $env:LOCALAPPDATA\\Veloce
[Environment]::SetEnvironmentVariable("PATH", "$env:LOCALAPPDATA\\Veloce;" + $env:PATH, "User")
```
```bash
# Linux
vpack x veloce-runtime-{tag}-linux-x86_64.vpack -o ~/.local/veloce
echo 'export PATH="$HOME/.local/veloce:$PATH"' >> ~/.bashrc && source ~/.bashrc
```

**4. Confirm**
```bash
veloce os status
```

> Tip: `vpack l veloce-runtime-{tag}-windows-x86_64.vpack` to preview contents before extracting.

---

### Manual Extraction

```powershell
# Windows
Expand-Archive veloce-windows-{tag}-x86_64.zip -DestinationPath .\\veloce
```
```bash
# Linux
tar -xzf veloce-linux-{tag}-x86_64.tar.gz -C ~/.local/veloce
```

---

### SHA-256 Checksums

```bash
sha256sum -c SHA256SUMS.txt
```

```
{sha_content.strip()}
```
"""

    body_path = p.out / "release_body.md"
    if not p.dry_run:
        body_path.write_text(body, encoding="utf-8")

    upload_assets = [
        p.out / f"veloce-windows-{tag}-x86_64.zip",
        p.out / f"veloce-linux-{tag}-x86_64.tar.gz",
        p.out / f"veloce-runtime-{tag}-windows-x86_64.vpack",
        p.out / f"veloce-runtime-{tag}-linux-x86_64.vpack",
        p.out / "SHA256SUMS.txt",
    ]

    if p.dry_run:
        print(f"\n  [dry-run] Would create draft release {tag} and upload:")
        for a in upload_assets:
            print(f"    {a.name}")
        return

    print(f"\n  Creating draft release {tag} …")
    subprocess.run(["gh", "release", "create", tag,
                    "--repo", REPO_GH,
                    "--title", f"VeloceNetwork {tag}",
                    "--notes-file", str(body_path),
                    "--draft"], check=False)

    for asset in upload_assets:
        if asset.exists():
            print(f"  Uploading {asset.name} …")
            subprocess.run(["gh", "release", "upload", tag, str(asset),
                            "--clobber", "--repo", REPO_GH], check=True)
        else:
            print(f"  [skip] {asset.name} not found")

    print(f"\n  Draft release ready:")
    print(f"    https://github.com/{REPO_GH}/releases/tag/{tag}")
    print("  Publish from the GitHub UI when ready.")


# ── main ──────────────────────────────────────────────────────────────────────

STAGE_FNS = {
    "preflight": stage_preflight,
    "build":     stage_build,
    "test":      stage_test,
    "security":  stage_security,
    "package":   stage_package,
    "verify":    stage_verify,
    "publish":   stage_publish,
}

def main():
    parser = argparse.ArgumentParser(
        description="VeloceNetwork standardized build + release pipeline.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    parser.add_argument("tag", help="Release tag, e.g. v4.8.0")
    parser.add_argument("--dry-run",    action="store_true")
    parser.add_argument("--from",       dest="from_stage", metavar="STAGE",
                        choices=ALL_STAGES, help="Start from this stage")
    parser.add_argument("--skip",       metavar="s1,s2",
                        help="Comma-separated stages to skip")
    parser.add_argument("--no-publish", action="store_true",
                        help="Skip publish stage")
    args = parser.parse_args()

    tag = args.tag
    if not tag.startswith("v"):
        sys.exit(f"ERROR: tag must start with 'v' (got '{tag}')")

    skip = set(args.skip.split(",") if args.skip else [])
    if args.no_publish:
        skip.add("publish")

    # Determine which stages to run
    stages = ALL_STAGES
    if args.from_stage:
        stages = stages[stages.index(args.from_stage):]
    stages = [s for s in stages if s not in skip]

    out = DIST_ROOT / tag
    if not args.dry_run:
        out.mkdir(parents=True, exist_ok=True)

    p = Pipeline(tag, args.dry_run, out)

    print(f"\n{'='*62}")
    print(f"  VeloceNetwork Pipeline  tag={tag}  dry-run={args.dry_run}")
    print(f"  Stages: {' → '.join(stages)}")
    print(f"  Output: dist/{tag}/")
    print(f"{'='*62}")

    start = datetime.now(timezone.utc)
    failed = None

    for stage in stages:
        try:
            STAGE_FNS[stage](p)
        except Exception as e:
            print(f"\n  ✗ Stage '{stage}' FAILED: {e}", file=sys.stderr)
            failed = stage
            break

    elapsed = (datetime.now(timezone.utc) - start).total_seconds()
    print(f"\n{'='*62}")
    if failed:
        print(f"  PIPELINE FAILED at stage '{failed}'  ({elapsed:.1f}s)")
        print(f"  Fix the issue then resume with: python scripts/pipeline.py {tag} --from {failed}")
        sys.exit(1)
    else:
        print(f"  PIPELINE COMPLETE  ({elapsed:.1f}s)")
        print(f"  Artifacts in: dist/{tag}/")
    print(f"{'='*62}\n")


if __name__ == "__main__":
    main()

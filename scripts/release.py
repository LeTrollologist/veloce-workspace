#!/usr/bin/env python3
"""
veloce-release — local release packaging & publishing script.

Usage:
    python scripts/release.py <version-tag> [--dry-run] [--skip-build]

Examples:
    python scripts/release.py v4.8.0
    python scripts/release.py v4.8.0 --dry-run       # preview without touching GitHub
    python scripts/release.py v4.8.0 --skip-build    # package pre-built binaries only

Workflow
--------
1. Validates the tag and gh CLI authentication.
2. Builds the workspace on the current platform (unless --skip-build).
3. Assembles the platform bundle into dist/<platform>-staging/.
4. Creates the canonical archive (zip for Windows, tar.gz for Linux/macOS).
5. Builds the .vpack package (if veloce-run is available).
6. Generates SHA256SUMS.txt covering exactly the 6 canonical assets.
7. Creates the GitHub release (draft) and uploads the 7 assets.

Canonical asset naming  (matches vpack-archiver convention: name-vVER-platform-arch)
---------------------
  veloce-windows-v{VER}-x86_64.zip
  veloce-linux-v{VER}-x86_64.tar.gz
  veloce-macos-v{VER}-universal.tar.gz
  veloce-runtime-v{VER}-windows-x86_64.vpack
  veloce-runtime-v{VER}-linux-x86_64.vpack
  veloce-runtime-v{VER}-macos-universal.vpack
  SHA256SUMS.txt

Prerequisites
-------------
  - Rust toolchain installed (for builds)
  - gh CLI authenticated with repo write access  (gh auth status)
  - On macOS: both aarch64-apple-darwin and x86_64-apple-darwin targets installed
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

# ── repo layout ──────────────────────────────────────────────────────────────
REPO_ROOT = Path(__file__).parent.parent.resolve()
DIST      = REPO_ROOT / "dist"
REPO_GH   = "LeTrollologist/veloce-workspace"

# Binaries produced per platform workspace
PLATFORM_BINARIES = {
    "windows": {
        "workspace": REPO_ROOT / "Windows",
        "build_target": "x86_64-pc-windows-msvc",
        "release_dir": "target/release",
        "bins": [
            "veloce-core.exe", "veloce-run.exe",
            "veloce-launcher.exe", "veloce-shell.exe",
        ],
        "libs": ["veloce_sdk.dll"],
        "sdk_header": "veloce_sdk.h",
    },
    "linux": {
        "workspace": REPO_ROOT / "Linux",
        "build_target": "x86_64-unknown-linux-gnu",
        "release_dir": "target/release",
        "bins": [
            "veloce-core", "veloce-run",
            "veloce-launcher", "veloce-shell",
        ],
        "libs": ["libveloce_sdk.so"],
        "sdk_header": "veloce_sdk.h",
    },
    "macos": {
        "workspace": REPO_ROOT / "macOS",
        "build_targets": [
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
        ],
        "bins": [
            "veloce-core", "veloce-run",
            "veloce-launcher", "veloce-shell",
        ],
        "libs": [],
        "sdk_header": "veloce_sdk.h",
    },
}

# ── helpers ───────────────────────────────────────────────────────────────────

def run(cmd: list[str], cwd: Path | None = None, check: bool = True) -> subprocess.CompletedProcess:
    print(f"  $ {' '.join(str(c) for c in cmd)}")
    return subprocess.run(cmd, cwd=cwd, check=check)


def gh(*args, check: bool = True) -> subprocess.CompletedProcess:
    cmd = ["gh"] + [str(a) for a in args]
    print(f"  $ {' '.join(cmd)}")
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.stdout.strip():
        print(f"    {r.stdout.strip()[:300]}")
    if check and r.returncode != 0:
        print(f"    STDERR: {r.stderr.strip()}", file=sys.stderr)
        raise subprocess.CalledProcessError(r.returncode, cmd)
    return r


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def detect_platform() -> str:
    s = platform.system().lower()
    if s == "windows":
        return "windows"
    if s == "darwin":
        return "macos"
    return "linux"


def require_gh_auth():
    r = subprocess.run(["gh", "auth", "status"], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("ERROR: gh CLI is not authenticated. Run: gh auth login")
    for line in r.stdout.splitlines():
        if "Logged in to" in line:
            print(f"  gh: {line.strip()}")
            break


# ── build step ────────────────────────────────────────────────────────────────

def build(plat: str, dry_run: bool):
    if dry_run:
        print(f"[dry-run] Would build {plat} workspace")
        return

    cfg = PLATFORM_BINARIES[plat]

    if plat == "macos":
        for target in cfg["build_targets"]:
            run(["cargo", "build", "--release", "--workspace", "--target", target],
                cwd=cfg["workspace"])
    else:
        run(["cargo", "build", "--release", "--workspace", "--target", cfg["build_target"]],
            cwd=cfg["workspace"])


# ── assembly step ─────────────────────────────────────────────────────────────

def assemble(plat: str, tag: str, staging: Path, dry_run: bool) -> Path:
    """Copy binaries into staging/, return path to the staging dir."""
    cfg = PLATFORM_BINARIES[plat]

    if not dry_run:
        staging.mkdir(parents=True, exist_ok=True)

    def copy_if(src: Path, name: str | None = None):
        dst = staging / (name or src.name)
        if src.exists():
            if not dry_run:
                shutil.copy2(src, dst)
            print(f"    + {dst.name}")
        else:
            print(f"    ~ (skipped, not found) {src.name}")

    print(f"\n[assemble] {plat}")

    if plat == "macos":
        # Build universal binaries with lipo
        arm_dir  = cfg["workspace"] / "target" / "aarch64-apple-darwin" / "release"
        x86_dir  = cfg["workspace"] / "target" / "x86_64-apple-darwin"  / "release"
        for bin_name in cfg["bins"]:
            arm = arm_dir / bin_name
            x86 = x86_dir / bin_name
            dst = staging / bin_name
            if arm.exists() and x86.exists():
                if not dry_run:
                    run(["lipo", "-create", "-output", str(dst), str(arm), str(x86)])
                print(f"    + {bin_name} (universal lipo)")
            elif arm.exists():
                copy_if(arm, bin_name)
            else:
                print(f"    ~ (skipped) {bin_name}")
    else:
        rel_dir = cfg["workspace"] / cfg["release_dir"]
        for bin_name in cfg["bins"]:
            copy_if(rel_dir / bin_name)
        for lib in cfg["libs"]:
            copy_if(rel_dir / lib)

    # SDK header
    header = cfg["workspace"] / cfg["sdk_header"]
    copy_if(header)

    # Docs
    for doc in ["README.md", "SECURITY.md"]:
        copy_if(cfg["workspace"] / doc)

    return staging


# ── pack (.vpack) step ────────────────────────────────────────────────────────

def build_vpack(plat: str, tag: str, staging: Path, out_dir: Path, dry_run: bool) -> Path | None:
    """Run veloce-run pack build to produce the .vpack file. Returns path or None."""
    suffix = {"windows": "windows-x86_64", "linux": "linux-x86_64", "macos": "macos-universal"}[plat]
    vpack_name = f"veloce-runtime-{tag}-{suffix}.vpack"
    vpack_path = out_dir / vpack_name

    cfg = PLATFORM_BINARIES[plat]
    if plat == "macos":
        runner = staging / "veloce-run"
    else:
        rel_dir = cfg["workspace"] / cfg["release_dir"]
        runner = rel_dir / ("veloce-run.exe" if plat == "windows" else "veloce-run")

    if not runner.exists():
        print(f"  [vpack] veloce-run not found at {runner} — skipping vpack")
        return None

    print(f"\n[vpack] {vpack_name}")
    if not dry_run:
        run([str(runner), "pack", "init", str(staging), "-n", "veloce-runtime"],
            check=False)
        run([str(runner), "pack", "build", str(staging), "-o", str(vpack_path)],
            check=False)

    return vpack_path if (dry_run or vpack_path.exists()) else None


# ── archive step ──────────────────────────────────────────────────────────────

def create_archive(plat: str, tag: str, staging: Path, out_dir: Path, dry_run: bool) -> Path:
    suffix = {"windows": "x86_64", "linux": "x86_64", "macos": "universal"}[plat]
    if plat == "windows":
        archive_name = f"veloce-windows-{tag}-{suffix}.zip"
        archive_path = out_dir / archive_name
        print(f"\n[archive] {archive_name}")
        if not dry_run:
            with zipfile.ZipFile(archive_path, "w", zipfile.ZIP_DEFLATED) as zf:
                for f in staging.rglob("*"):
                    if f.is_file():
                        zf.write(f, f.relative_to(staging))
    else:
        ext = "tar.gz"
        platform_label = {"linux": "linux", "macos": "macos"}[plat]
        archive_name = f"veloce-{platform_label}-{tag}-{suffix}.{ext}"
        archive_path = out_dir / archive_name
        print(f"\n[archive] {archive_name}")
        if not dry_run:
            with tarfile.open(archive_path, "w:gz") as tf:
                tf.add(staging, arcname=".")

    return archive_path


# ── checksum step ─────────────────────────────────────────────────────────────

def generate_sha256sums(assets: list[Path], out_dir: Path, dry_run: bool) -> Path:
    sha_path = out_dir / "SHA256SUMS.txt"
    lines = []
    for p in assets:
        if p and p.exists():
            digest = sha256_file(p)
            lines.append(f"{digest}  {p.name}")
            print(f"  {digest}  {p.name}")
        else:
            print(f"  (skipped missing) {p}")

    print(f"\n[checksums] writing {sha_path.name}")
    if not dry_run:
        sha_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return sha_path


# ── publish step ──────────────────────────────────────────────────────────────

def publish(tag: str, assets: list[Path], notes_path: Path, dry_run: bool):
    upload_files = [p for p in assets if p and (dry_run or p.exists())]

    if dry_run:
        print(f"\n[dry-run] Would create release {tag} and upload:")
        for p in upload_files:
            print(f"  {p.name}")
        return

    print(f"\n[publish] Creating GitHub release {tag}")
    gh("release", "create", tag,
       "--repo", REPO_GH,
       "--title", f"VeloceNetwork {tag}",
       "--notes-file", str(notes_path),
       "--draft",
       check=False)

    print(f"\n[publish] Uploading {len(upload_files)} assets")
    for p in upload_files:
        print(f"  uploading {p.name} …")
        gh("release", "upload", tag, str(p), "--clobber", "--repo", REPO_GH)

    print(f"\n[publish] Done. Review draft at:")
    print(f"  https://github.com/{REPO_GH}/releases/tag/{tag}")
    print("  Publish it from the GitHub UI when ready.")


# ── release notes template ────────────────────────────────────────────────────

def write_release_notes(tag: str, out_dir: Path) -> Path:
    ver = tag.lstrip("v")  # noqa: F841  (kept for future use in highlights)

    # Note: this is a Python f-string; literal braces in bash snippets are doubled.
    notes = (
        f"# VeloceNetwork {tag}\n"
        "\n"
        "> Review this draft and fill in the highlights before publishing.\n"
        "\n"
        "---\n"
        "\n"
        "### Platform Release Packages\n"
        "\n"
        "| Platform | Archive | VPack |\n"
        "| :--- | :--- | :--- |\n"
        f"| **Windows** x86\_64 | `veloce-windows-{tag}-x86_64.zip` | `veloce-runtime-{tag}-windows-x86_64.vpack` |\n"
        f"| **Linux** x86\_64   | `veloce-linux-{tag}-x86_64.tar.gz` | `veloce-runtime-{tag}-linux-x86_64.vpack` |\n"
        f"| **macOS** universal | `veloce-macos-{tag}-universal.tar.gz` | `veloce-runtime-{tag}-macos-universal.vpack` |\n"
        "\n"
        "---\n"
        "\n"
        "### Installation\n"
        "\n"
        "#### Option A — VPack Archiver (recommended)\n"
        "\n"
        "The `.vpack` packages are installed using"
        " [VPack Archiver](https://github.com/LeTrollologist/vpack-archiver)"
        " — an ultra-fast O(1)-seek archive tool with built-in CRC-32 integrity"
        " and Ed25519 signature verification.\n"
        "\n"
        "**1. Download VPack Archiver**\n"
        "\n"
        "| Platform | Download |\n"
        "| :--- | :--- |\n"
        "| Windows | [`vpack-archiver-v1.1.0-windows-x86_64.zip`](https://github.com/LeTrollologist/vpack-archiver/releases/download/v1.1.0/vpack-archiver-v1.1.0-windows-x86_64.zip) |\n"
        "\n"
        "Unzip and add `vpack-archiver.exe` (Windows) or `vpack-archiver` (Linux/macOS) to your `PATH`.\n"
        "\n"
        "**2. Verify the runtime package integrity**\n"
        "\n"
        "```bash\n"
        f"# Checks CRC-32 of every entry and the Ed25519 publisher signature\n"
        f"vpack t veloce-runtime-{tag}-windows-x86_64.vpack   # Windows\n"
        f"vpack t veloce-runtime-{tag}-linux-x86_64.vpack     # Linux\n"
        f"vpack t veloce-runtime-{tag}-macos-universal.vpack  # macOS\n"
        "```\n"
        "\n"
        "> Tip: preview contents before extracting: `vpack l veloce-runtime-" + tag + "-windows-x86_64.vpack`\n"
        "\n"
        "**3. Extract the runtime**\n"
        "\n"
        "```powershell\n"
        "# Windows (PowerShell)\n"
        f"vpack x veloce-runtime-{tag}-windows-x86_64.vpack -o $env:LOCALAPPDATA\\Veloce\n"
        "```\n"
        "\n"
        "```bash\n"
        "# Linux\n"
        f"vpack x veloce-runtime-{tag}-linux-x86_64.vpack -o ~/.local/veloce\n"
        "\n"
        "# macOS\n"
        f"vpack x veloce-runtime-{tag}-macos-universal.vpack -o ~/.local/veloce\n"
        "```\n"
        "\n"
        "**4. Add to PATH and verify**\n"
        "\n"
        "```powershell\n"
        "# Windows — add to user PATH (one-time)\n"
        "[Environment]::SetEnvironmentVariable(\"PATH\", \"$env:LOCALAPPDATA\\Veloce;\" + $env:PATH, \"User\")\n"
        "```\n"
        "\n"
        "```bash\n"
        "# Linux / macOS — add to shell profile\n"
        "echo 'export PATH=\"$HOME/.local/veloce:$PATH\"' >> ~/.bashrc   # or ~/.zshrc\n"
        "source ~/.bashrc\n"
        "```\n"
        "\n"
        "```bash\n"
        "# Verify the installation\n"
        "veloce os status\n"
        "```\n"
        "\n"
        "---\n"
        "\n"
        "#### Option B — Manual archive extraction\n"
        "\n"
        "```powershell\n"
        "# Windows\n"
        f"Expand-Archive veloce-windows-{tag}-x86_64.zip -DestinationPath .\\veloce\n"
        "```\n"
        "\n"
        "```bash\n"
        "# Linux\n"
        f"mkdir -p ~/.local/veloce && tar -xzf veloce-linux-{tag}-x86_64.tar.gz -C ~/.local/veloce\n"
        "\n"
        "# macOS\n"
        f"mkdir -p ~/.local/veloce && tar -xzf veloce-macos-{tag}-universal.tar.gz -C ~/.local/veloce\n"
        "```\n"
        "\n"
        "---\n"
        "\n"
        "### Cryptographic Checksums (SHA-256)\n"
        "\n"
        "```bash\n"
        "sha256sum -c SHA256SUMS.txt          # Linux / macOS\n"
        "Get-FileHash * | Format-Table -Auto  # Windows PowerShell\n"
        "```\n"
        "\n"
        "```\n"
        "# See SHA256SUMS.txt attached to this release\n"
        "```\n"
    )

    path = out_dir / "release_notes.md"
    path.write_text(notes, encoding="utf-8")
    return path


# ── main ──────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description="Package and publish a VeloceNetwork release.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    parser.add_argument("tag", help="Release tag, e.g. v4.8.0")
    parser.add_argument("--dry-run", action="store_true",
                        help="Preview actions without building, uploading, or modifying GitHub")
    parser.add_argument("--skip-build", action="store_true",
                        help="Skip cargo build — package whatever is already in target/release")
    parser.add_argument("--platform", choices=["windows", "linux", "macos"],
                        default=detect_platform(),
                        help="Platform to package (default: auto-detected)")
    args = parser.parse_args()

    tag  = args.tag
    plat = args.platform

    if not tag.startswith("v"):
        sys.exit(f"ERROR: tag must start with 'v' (got '{tag}')")

    print(f"\n{'='*60}")
    print(f"  veloce-release  tag={tag}  platform={plat}  dry-run={args.dry_run}")
    print(f"{'='*60}\n")

    # 1. Verify gh auth
    print("[preflight] Checking gh CLI auth …")
    require_gh_auth()

    # 2. Output directory for this release
    out_dir = DIST / tag
    staging = out_dir / "staging"
    if not args.dry_run:
        out_dir.mkdir(parents=True, exist_ok=True)

    # 3. Build
    if not args.skip_build:
        print(f"\n[build] Building {plat} workspace …")
        build(plat, args.dry_run)
    else:
        print(f"\n[build] --skip-build: using pre-built binaries")

    # 4. Assemble staging directory
    assemble(plat, tag, staging, args.dry_run)

    # 5. Create canonical archive
    archive = create_archive(plat, tag, staging, out_dir, args.dry_run)

    # 6. Build .vpack
    vpack = build_vpack(plat, tag, staging, out_dir, args.dry_run)

    # 7. Generate SHA256SUMS.txt
    #    (covers archive + vpack for this platform; merge across platforms before publishing)
    print(f"\n[checksums]")
    canonical = [a for a in [archive, vpack] if a]
    sha_path = generate_sha256sums(canonical, out_dir, args.dry_run)

    # 8. Write release notes template
    notes_path = write_release_notes(tag, out_dir)

    # 9. Publish to GitHub
    all_assets = canonical + [sha_path]
    publish(tag, all_assets, notes_path, args.dry_run)

    print(f"\n[done] Artifacts written to: {out_dir}")


if __name__ == "__main__":
    main()

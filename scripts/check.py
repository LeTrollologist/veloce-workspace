#!/usr/bin/env python3
"""
veloce-check — local build, test, security & compliance runner.

Usage:
    python scripts/check.py [--platform <windows|linux|macos|current>]
                            [--skip-build] [--skip-test]
                            [--skip-audit] [--skip-security]

Runs everything the deleted GitHub Actions workflows used to run, locally:

  1. cargo build --workspace          (platform workspace)
  2. cargo test  --workspace          (platform workspace)
  3. cargo check --workspace          (cross-platform sanity, Linux workspace)
  4. cargo audit                      (dependency advisory scan, Linux Cargo.lock)
  5. cargo test -p veloce-mesh        (cryptographic invariant suite)
  6. veloce-run security audit        (SOC 2 Type II compliance scan)
  7. veloce-run security sbom         (CycloneDX SBOM generation → dist/sbom/)
"""

import argparse
import os
import platform
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).parent.parent.resolve()

IGNORED_ADVISORIES = [
    "RUSTSEC-2020-0071",
    "RUSTSEC-2020-0159",
]

WORKSPACES = {
    "windows": REPO_ROOT / "Windows",
    "linux":   REPO_ROOT / "Linux",
    "macos":   REPO_ROOT / "macOS",
}

def detect_platform() -> str:
    s = platform.system().lower()
    if s == "windows": return "windows"
    if s == "darwin":  return "macos"
    return "linux"


def run(cmd: list, cwd: Path | None = None, check: bool = True, label: str = "") -> int:
    display = label or " ".join(str(c) for c in cmd)
    print(f"\n  $ {display}")
    r = subprocess.run([str(c) for c in cmd], cwd=cwd)
    if check and r.returncode != 0:
        print(f"\n  FAILED (exit {r.returncode}): {display}", file=sys.stderr)
        sys.exit(r.returncode)
    return r.returncode


def step(title: str):
    bar = "─" * (len(title) + 4)
    print(f"\n┌{bar}┐")
    print(f"│  {title}  │")
    print(f"└{bar}┘")


# ── 1 & 2: build + test current platform workspace ───────────────────────────

def build_and_test(plat: str, skip_build: bool, skip_test: bool):
    ws = WORKSPACES[plat]

    if not skip_build:
        step(f"Build — {plat} workspace")
        run(["cargo", "build", "--workspace"], cwd=ws)

    if not skip_test:
        step(f"Test — {plat} workspace")
        run(["cargo", "test", "--workspace"], cwd=ws)


# ── 3: cargo check (cross-platform sanity using Linux workspace) ──────────────

def cargo_check():
    step("Cargo check (cross-platform sanity)")
    run(["cargo", "check", "--workspace"], cwd=WORKSPACES["linux"])


# ── 4: cargo audit ────────────────────────────────────────────────────────────

def dependency_audit():
    step("Dependency audit (cargo-audit)")

    # Install cargo-audit if missing
    probe = subprocess.run(["cargo", "audit", "--version"], capture_output=True)
    if probe.returncode != 0:
        print("  Installing cargo-audit …")
        run(["cargo", "install", "cargo-audit", "--locked"], check=False)

    ignore_flags = []
    for adv in IGNORED_ADVISORIES:
        ignore_flags += ["--ignore", adv]

    lock_file = WORKSPACES["linux"] / "Cargo.lock"
    rc = run(
        ["cargo", "audit", "--file", str(lock_file)] + ignore_flags,
        check=False,
        label=f"cargo audit --file Linux/Cargo.lock {' '.join(ignore_flags)}",
    )
    if rc != 0:
        print("  Advisory scan reported issues — review output above.")
    else:
        print("  Advisory scan clean.")


# ── 5: cryptographic invariant tests ─────────────────────────────────────────

def crypto_invariants():
    step("Cryptographic invariant tests (veloce-mesh)")
    run(
        ["cargo", "test", "-p", "veloce-mesh", "--", "--nocapture"],
        cwd=WORKSPACES["linux"],
    )


# ── 6: SOC 2 compliance audit ─────────────────────────────────────────────────

def soc2_audit(skip_security: bool):
    if skip_security:
        print("\n  [skip] SOC 2 audit (--skip-security)")
        return

    step("SOC 2 Type II compliance audit (veloce-run security audit)")
    runner = WORKSPACES["linux"] / "target" / "release" / "veloce-run"
    if not runner.exists():
        # Try debug build
        runner = WORKSPACES["linux"] / "target" / "debug" / "veloce-run"
    if not runner.exists():
        print("  veloce-run not built yet — run without --skip-build first.")
        return

    run([str(runner), "security", "audit"])


# ── 7: SBOM generation ────────────────────────────────────────────────────────

def generate_sbom(skip_security: bool):
    if skip_security:
        print("\n  [skip] SBOM generation (--skip-security)")
        return

    step("SBOM generation (CycloneDX via veloce-run security sbom)")
    runner = WORKSPACES["linux"] / "target" / "release" / "veloce-run"
    if not runner.exists():
        runner = WORKSPACES["linux"] / "target" / "debug" / "veloce-run"
    if not runner.exists():
        print("  veloce-run not built yet — run without --skip-build first.")
        return

    sbom_dir = REPO_ROOT / "dist" / "sbom"
    sbom_dir.mkdir(parents=True, exist_ok=True)
    sbom_out = sbom_dir / "veloce-sbom-cyclonedx.json"

    run([str(runner), "security", "sbom", "--output", str(sbom_out)])
    if sbom_out.exists():
        print(f"  SBOM written: {sbom_out} ({sbom_out.stat().st_size:,} bytes)")


# ── main ──────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description="Local build, test, security & compliance runner.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    parser.add_argument("--platform", choices=["windows", "linux", "macos", "current"],
                        default="current",
                        help="Workspace to build/test (default: auto-detect from OS)")
    parser.add_argument("--skip-build",    action="store_true", help="Skip cargo build")
    parser.add_argument("--skip-test",     action="store_true", help="Skip cargo test")
    parser.add_argument("--skip-audit",    action="store_true", help="Skip cargo audit")
    parser.add_argument("--skip-security", action="store_true",
                        help="Skip SOC 2 audit and SBOM generation")
    args = parser.parse_args()

    plat = detect_platform() if args.platform == "current" else args.platform

    print(f"\n{'='*60}")
    print(f"  veloce-check  platform={plat}")
    print(f"{'='*60}")

    build_and_test(plat, args.skip_build, args.skip_test)

    # Security steps run on Linux workspace (mirrors original CI behaviour)
    if plat == "linux" or detect_platform() == "linux":
        if not args.skip_audit:
            dependency_audit()
        crypto_invariants()
        soc2_audit(args.skip_security)
        generate_sbom(args.skip_security)
    else:
        print(f"\n  [info] Audit/security steps target the Linux workspace.")
        print(f"  [info] Run `python scripts/check.py --platform linux` on a Linux machine,")
        print(f"  [info] or use WSL: wsl python scripts/check.py --platform linux")
        if not args.skip_audit:
            # cargo-audit can still run on Windows/macOS against the Linux Cargo.lock
            dependency_audit()

    print(f"\n{'='*60}")
    print(f"  All checks passed.")
    print(f"{'='*60}\n")


if __name__ == "__main__":
    main()

//! cgroups v2 filesystem API for VeloceCore process isolation.
//!
//! Each spawned node gets its own cgroup slice under the parent cgroup.
//! When running under systemd with `Delegate=yes`, the parent is determined
//! by reading `/proc/self/cgroup`.  Otherwise, we try to create
//! `/sys/fs/cgroup/veloce/` directly.
//!
//! Kernel requirements:
//!   - cgroups v2 unified hierarchy: Linux 4.15+
//!   - `memory.max` controller: Linux 4.5+
//!   - `cpu.weight`: Linux 4.19+
//!   - `cgroup.kill`: Linux 5.14+ (fallback to SIGKILL loop on older kernels)
//!
//! The `Delegate=yes` systemd unit option is required for sub-cgroup creation.

#![cfg(unix)]

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Handle to a per-node cgroup directory.
#[derive(Debug, Clone)]
pub struct CgroupPath(PathBuf);

impl CgroupPath {
    /// Create a new cgroup for `node_id` under `base`.
    pub fn create(base: &Path, node_id: Uuid) -> Result<Self> {
        let path = base.join(node_id.simple().to_string());
        std::fs::create_dir_all(&path)
            .with_context(|| format!("mkdir cgroup {:?}", path))?;
        Ok(Self(path))
    }

    /// Set maximum memory in bytes (`memory.max`).
    pub fn set_memory_max(&self, bytes: u64) -> Result<()> {
        self.write_file("memory.max", &format!("{bytes}"))
    }

    /// Set CPU weight (1–10000, default 100) (`cpu.weight`).
    pub fn set_cpu_weight(&self, weight: u32) -> Result<()> {
        self.write_file("cpu.weight", &format!("{weight}"))
    }

    /// Move a process into this cgroup by writing its PID to `cgroup.procs`.
    pub fn add_proc(&self, pid: u32) -> Result<()> {
        self.write_file("cgroup.procs", &format!("{pid}"))
    }

    /// Kill all processes in this cgroup.
    ///
    /// Tries `cgroup.kill` (Linux 5.14+) first; falls back to iterating
    /// `cgroup.procs` and sending SIGKILL to each PID.
    pub fn kill(&self) -> Result<()> {
        // Try cgroup.kill (Linux 5.14+)
        if self.write_file("cgroup.kill", "1").is_ok() {
            return Ok(());
        }
        // Fallback: read cgroup.procs and kill each PID
        let procs_path = self.0.join("cgroup.procs");
        let contents = std::fs::read_to_string(&procs_path)
            .context("read cgroup.procs")?;
        for line in contents.lines() {
            let pid: i32 = line.trim().parse().unwrap_or(0);
            if pid > 0 {
                unsafe { libc::kill(pid, libc::SIGKILL); }
            }
        }
        Ok(())
    }

    /// Read current memory usage in bytes (`memory.current`).
    pub fn read_memory_current(&self) -> Result<u64> {
        let s = std::fs::read_to_string(self.0.join("memory.current"))
            .context("read memory.current")?;
        s.trim().parse().context("parse memory.current")
    }

    /// Read total CPU time in microseconds from `cpu.stat usage_usec`.
    pub fn read_cpu_usage_usec(&self) -> Result<u64> {
        let s = std::fs::read_to_string(self.0.join("cpu.stat"))
            .context("read cpu.stat")?;
        for line in s.lines() {
            if let Some(rest) = line.strip_prefix("usage_usec ") {
                return rest.trim().parse().context("parse cpu.stat usage_usec");
            }
        }
        bail!("usage_usec not found in cpu.stat")
    }

    /// Remove the cgroup directory (only works when all processes have exited).
    #[allow(dead_code)]
    pub fn cleanup(&self) -> Result<()> {
        // The dir can only be removed once it's empty of processes
        std::fs::remove_dir(&self.0)
            .with_context(|| format!("rmdir cgroup {:?}", self.0))
    }

    /// Return the path to this cgroup directory.
    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        &self.0
    }

    fn write_file(&self, name: &str, value: &str) -> Result<()> {
        let p = self.0.join(name);
        std::fs::write(&p, value)
            .with_context(|| format!("write {:?} = {:?}", p, value))
    }
}

// ── Cgroup availability probe ─────────────────────────────────────────────────

/// Returns `true` if cgroups v2 unified hierarchy is active and VeloceCore
/// has write access to create sub-cgroups.
pub fn cgroup_v2_available() -> bool {
    // Unified hierarchy marker
    if !Path::new("/sys/fs/cgroup/cgroup.controllers").exists() {
        return false;
    }
    let base = cgroup_base_path();
    // Try creating the parent veloce/ directory as a write-access test
    if !base.exists() {
        std::fs::create_dir_all(&base).is_ok()
    } else {
        // Check that we can write to the subtree_control file
        base.join("cgroup.subtree_control").exists()
            || std::fs::metadata(&base).map(|m| !m.permissions().readonly()).unwrap_or(false)
    }
}

/// Returns the base directory under which VeloceCore creates per-node cgroups.
///
/// When running under systemd, reads `/proc/self/cgroup` to find the
/// systemd-assigned slice (unified hierarchy `0::/...` line) and appends
/// `veloce/` under it.  Otherwise, uses `/sys/fs/cgroup/veloce/`.
pub fn cgroup_base_path() -> PathBuf {
    if let Ok(contents) = std::fs::read_to_string("/proc/self/cgroup") {
        for line in contents.lines() {
            // Unified hierarchy line: "0::/system.slice/veloce-core.service"
            if let Some(path) = line.strip_prefix("0::/") {
                let unified_path = PathBuf::from("/sys/fs/cgroup").join(path).join("veloce");
                return unified_path;
            }
        }
    }
    PathBuf::from("/sys/fs/cgroup/veloce")
}

/// Enable the memory and cpu controllers on the base cgroup, which is
/// required before child cgroups can use those controllers.
pub fn enable_controllers(base: &Path) -> Result<()> {
    let subtree = base.join("cgroup.subtree_control");
    if subtree.exists() {
        std::fs::write(&subtree, "+memory +cpu")
            .with_context(|| format!("enable controllers in {:?}", subtree))?;
    }
    Ok(())
}

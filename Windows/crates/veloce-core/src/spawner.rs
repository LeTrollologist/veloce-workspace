//! Linux process spawner for VeloceCore.
//!
//! Replaces the Windows Job Object spawner (`job.rs` Windows code) for the
//! Linux port.  Uses `tokio::process::Command` (fork+execve) with optional
//! cgroups v2 resource isolation.

#![cfg(unix)]

use anyhow::{Context, Result};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use uuid::Uuid;

use veloce_ipc::message::SpawnNodeMsg;

use crate::cgroup::{self, CgroupPath};

/// All Linux-specific fields added to `NodeHandle`.
pub struct LinuxHandle {
    pub child:  tokio::process::Child,
    pub cgroup: Option<CgroupPath>,
}

/// Spawn a child process for the given `SpawnNodeMsg`.
///
/// Returns:
/// - The `tokio::process::Child` handle (for `try_wait`)
/// - The optional `CgroupPath` (if cgroups v2 is available)
/// - The child PID
///
/// Log readers must be wired up by the caller after this returns
/// (stdout/stderr are left as `piped` for the caller to take).
pub async fn spawn_process(
    msg:        &SpawnNodeMsg,
    node_id:    Uuid,
    env_extras: &[(String, String)],
) -> Result<(tokio::process::Child, Option<CgroupPath>, u32)> {
    let mut cmd = build_command(msg, node_id, env_extras)?;

    let mut child = cmd.spawn().context("spawn child process")?;
    let pid = child.id().context("child has no PID")?;

    // ── cgroup assignment ─────────────────────────────────────────────────
    let cgroup = if cgroup::cgroup_v2_available() {
        let base = cgroup::cgroup_base_path();
        // Ensure controllers are enabled on the parent
        let _ = cgroup::enable_controllers(&base);

        match CgroupPath::create(&base, node_id) {
            Ok(cg) => {
                // Apply resource limits if specified
                if let Some(ref limits) = msg.limits {
                    if let Some(mem_mb) = limits.mem_mb {
                        let _ = cg.set_memory_max(mem_mb * 1024 * 1024);
                    }
                    if let Some(cpu_pct) = limits.cpu_pct {
                        // cpu.weight: 1–10000; 100 = default. Scale: 1% = weight 10.
                        let weight = cpu_pct.clamp(1, 100) * 10;
                        let _ = cg.set_cpu_weight(weight);
                    }
                }
                if let Err(e) = cg.add_proc(pid) {
                    tracing::warn!(%node_id, "cgroup add_proc failed: {e}");
                }
                Some(cg)
            }
            Err(e) => {
                tracing::warn!(%node_id, "cgroup create failed (continuing without): {e}");
                None
            }
        }
    } else {
        tracing::debug!("cgroups v2 unavailable — running without resource limits");
        None
    };

    Ok((child, cgroup, pid))
}

/// Kill a process, preferring cgroup kill, then SIGTERM → wait → SIGKILL.
pub fn terminate_process(
    child:  &mut tokio::process::Child,
    cgroup: Option<&CgroupPath>,
    pid:    u32,
) {
    // Try cgroup kill first (kills entire subtree)
    if let Some(cg) = cgroup {
        if cg.kill().is_ok() {
            return;
        }
    }

    // Fall back to SIGTERM then SIGKILL
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }

    // Give it 5 seconds, then SIGKILL
    let pid_i = pid as i32;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return, // exited
            _ => {}
        }
        if start.elapsed().as_secs() >= 5 {
            unsafe { libc::kill(pid_i, libc::SIGKILL); }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Non-blocking check whether the child is still alive.
pub fn is_alive(child: &mut tokio::process::Child) -> bool {
    matches!(child.try_wait(), Ok(None))
}

// ── Internals ────────────────────────────────────────────────────────────────

fn build_command(
    msg:        &SpawnNodeMsg,
    node_id:    Uuid,
    env_extras: &[(String, String)],
) -> Result<Command> {
    let mut cmd = Command::new(&msg.executable);

    if !msg.args.is_empty() {
        cmd.args(&msg.args);
    }

    // Environment: start with inherited, add explicit vars, add VeloceCore vars
    cmd.env_clear();

    // Inherit PATH and other essentials
    for (k, v) in std::env::vars() {
        cmd.env(&k, &v);
    }

    // User-supplied environment
    for (k, v) in &msg.env {
        cmd.env(k, v);
    }

    // VeloceCore injected vars
    let socket_path = resolve_core_socket();
    cmd.env("VELOCE_SOCKET",  &socket_path);
    cmd.env("VELOCE_NODE_ID", node_id.to_string());

    // Injected extras (volumes, secrets already resolved by job.rs)
    for (k, v) in env_extras {
        cmd.env(k, v);
    }

    // Pipe stdout/stderr so VeloceCore can capture logs
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.stdin(Stdio::null());

    // Place child in its own process group so kill(-pgid, sig) reaches all descendants
    unsafe {
        cmd.pre_exec(|| {
            libc::setpgid(0, 0);
            Ok(())
        });
    }

    Ok(cmd)
}

fn resolve_core_socket() -> String {
    if std::path::Path::new("/run/veloce/core.sock").exists() {
        "/run/veloce/core.sock".to_string()
    } else if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        format!("{xdg}/veloce/core.sock")
    } else {
        "/run/veloce/core.sock".to_string()
    }
}

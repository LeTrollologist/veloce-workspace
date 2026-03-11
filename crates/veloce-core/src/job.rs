/*!
# Job Object Node Spawner

Each sideloaded node runs inside its own Windows Job Object.

Security properties enforced:
- `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: node dies when VeloceCore exits.
- `JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION`: crash doesn't linger.
- Optional CPU rate / memory limits via `JOBOBJECT_CPU_RATE_CONTROL_INFORMATION`
  and `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`.
- `PROCESS_CREATION_MITIGATION_POLICY_BLOCK_NON_MICROSOFT_BINARIES_ALWAYS_ON` (future).

The spawned process receives its VeloceCore pipe name as `VELOCE_PIPE` env var.
*/

use anyhow::{bail, Context, Result};
use parking_lot::Mutex;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::watch;
use uuid::Uuid;

use veloce_ipc::message::{NodeEvent, NodeStatus, SpawnNodeMsg, NodeLimits};

use crate::state::CoreState;

// Windows API surface
#[cfg(windows)]
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{CloseHandle, HANDLE, BOOL},
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW,
                JobObjectExtendedLimitInformation, JobObjectCpuRateControlInformation,
                QueryInformationJobObject, SetInformationJobObject,
                JOBOBJECT_BASIC_LIMIT_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOBOBJECT_CPU_RATE_CONTROL_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
                JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOB_OBJECT_CPU_RATE_CONTROL_ENABLE,
                JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
            },
            Threading::{
                CreateProcessW, OpenProcess, TerminateProcess, WaitForSingleObject,
                PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTUPINFOW,
                CREATE_UNICODE_ENVIRONMENT, INFINITE,
                PROCESS_TERMINATE, PROCESS_QUERY_INFORMATION,
            },
        },
    },
};

// ── NODE HANDLE ───────────────────────────────────────────────────────────────

/// A live node managed by VeloceCore.
pub struct NodeHandle {
    pub node_id:   Uuid,
    pub slot_idx:  usize,
    pub pid:       u32,
    pub app_name:  String,
    pub pipe_path: String,
    #[cfg(windows)]
    job_handle:    SafeHandle,
    #[cfg(windows)]
    proc_handle:   SafeHandle,
    pub event_tx:  tokio::sync::broadcast::Sender<NodeEventMsg>,
}

impl NodeHandle {
    /// Raw Win32 HANDLE value as isize — safe to copy for health-check snapshots.
    /// Always 0 on non-Windows builds.
    #[cfg(windows)]
    pub fn proc_handle_raw(&self) -> isize {
        self.proc_handle.0 .0 as isize
    }
    #[cfg(not(windows))]
    pub fn proc_handle_raw(&self) -> isize { 0 }
}

#[derive(Debug, Clone)]
pub struct NodeEventMsg {
    pub node_id: Uuid,
    pub event:   NodeEvent,
}

/// RAII wrapper around a Win32 HANDLE.
#[cfg(windows)]
struct SafeHandle(HANDLE);

#[cfg(windows)]
impl Drop for SafeHandle {
    fn drop(&mut self) {
        unsafe { let _ = CloseHandle(self.0); }
    }
}

#[cfg(windows)]
unsafe impl Send for SafeHandle {}
#[cfg(windows)]
unsafe impl Sync for SafeHandle {}

// ── SPAWNER ───────────────────────────────────────────────────────────────────

/// Spawn a node under a new Job Object.
#[cfg(windows)]
pub async fn spawn_node(
    msg:       &SpawnNodeMsg,
    node_id:   Uuid,
    slot_idx:  usize,
    core_pipe: &str,
) -> Result<NodeHandle> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let (event_tx, _) = tokio::sync::broadcast::channel(16);
    let pipe_path = format!(r"\\.\pipe\VeloceNode-{}", node_id.simple());

    // ── Create Job Object ────────────────────────────────────────────────────
    let job_name = format!("VeloceNode-{}\0", node_id.simple());
    let job_name_w: Vec<u16> = OsStr::new(&job_name).encode_wide().collect();

    let job_handle = unsafe {
        let h = CreateJobObjectW(None, PCWSTR(job_name_w.as_ptr()))
            .context("CreateJobObject")?;
        SafeHandle(h)
    };

    // Apply limits
    apply_job_limits(job_handle.0, msg.limits.as_ref())?;

    // ── Build environment block ──────────────────────────────────────────────
    // Start with current environment, then merge caller's overrides
    let mut env_map: HashMap<String, String> = std::env::vars().collect();
    for (k, v) in &msg.env {
        env_map.insert(k.clone(), v.clone());
    }
    // Inject VeloceCore pipe path so the node can self-connect
    env_map.insert("VELOCE_PIPE".into(), core_pipe.into());
    env_map.insert("VELOCE_NODE_ID".into(), node_id.to_string());
    env_map.insert("VELOCE_NODE_PIPE".into(), pipe_path.clone());

    let env_block = build_env_block(&env_map);

    // ── CreateProcess ────────────────────────────────────────────────────────
    let cmdline = build_cmdline(&msg.executable, &msg.args);
    let cmdline_w: Vec<u16> = OsStr::new(&cmdline).encode_wide().chain(Some(0)).collect();

    let mut si = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut pi = PROCESS_INFORMATION::default();

    unsafe {
        CreateProcessW(
            PCWSTR::null(),
            windows::core::PWSTR(cmdline_w.as_ptr() as *mut u16),
            None,
            None,
            BOOL(0),
            CREATE_UNICODE_ENVIRONMENT,
            Some(env_block.as_ptr() as *const _),
            PCWSTR::null(),
            &si,
            &mut pi,
        ).context("CreateProcess")?;
    }

    let pid = pi.dwProcessId;
    let proc_handle = SafeHandle(pi.hProcess);
    // Close thread handle — we don't need it
    unsafe { let _ = CloseHandle(pi.hThread); }

    // ── Assign to Job Object ─────────────────────────────────────────────────
    unsafe {
        AssignProcessToJobObject(job_handle.0, pi.hProcess)
            .context("AssignProcessToJobObject")?;
    }

    tracing::info!(%node_id, pid, app_name = %msg.app_name, "node spawned");

    Ok(NodeHandle {
        node_id,
        slot_idx,
        pid,
        app_name:  msg.app_name.clone(),
        pipe_path,
        job_handle,
        proc_handle,
        event_tx,
    })
}

/// Terminate a node by closing its Job Object (which triggers KILL_ON_JOB_CLOSE).
#[cfg(windows)]
pub fn terminate_node(handle: &NodeHandle, exit_code: u32) -> Result<()> {
    unsafe {
        TerminateProcess(handle.proc_handle.0, exit_code)
            .context("TerminateProcess")?;
    }
    tracing::info!(node_id = %handle.node_id, "node terminated");
    Ok(())
}

// ── HEALTH MONITOR ────────────────────────────────────────────────────────────

/// Background task: poll live nodes every 2 seconds, update registry on exit.
pub async fn health_loop(state: Arc<CoreState>) {
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        check_nodes(&state).await;
    }
}

#[cfg(windows)]
async fn check_nodes(state: &Arc<CoreState>) {
    use windows::Win32::Foundation::{HANDLE, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::GetExitCodeProcess;

    let nodes = state.node_table().list_live();
    for info in nodes {
        if info.proc_handle_raw == 0 { continue; }

        // SAFETY: proc_handle_raw is a snapshot of a valid HANDLE owned by
        // the NodeHandle still in the NodeTable.  We only read status here.
        let raw_handle = HANDLE(info.proc_handle_raw);
        let still_running = unsafe {
            WaitForSingleObject(raw_handle, 0) == WAIT_TIMEOUT
        };
        if !still_running {
            let mut exit_code = 0u32;
            unsafe { let _ = GetExitCodeProcess(raw_handle, &mut exit_code); }

            tracing::info!(node_id = %info.node_id, exit_code, "node exited");
            let _ = state.registry().set_node_status(
                info.slot_idx,
                crate::registry::NodeStatus::Stopped,
                exit_code,
            );
            let _ = info.event_tx.send(NodeEventMsg {
                node_id: info.node_id,
                event:   NodeEvent::Exited { exit_code },
            });
            state.node_table().remove(info.node_id);
        }
    }
}

#[cfg(not(windows))]
async fn check_nodes(_state: &Arc<CoreState>) {}

// ── WIN32 HELPERS ─────────────────────────────────────────────────────────────

#[cfg(windows)]
fn apply_job_limits(job: HANDLE, limits: Option<&NodeLimits>) -> Result<()> {
    // Always apply: kill-on-close and die-on-unhandled-exception
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;

    if let Some(l) = limits {
        if let Some(mem_mb) = l.mem_mb {
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
            info.ProcessMemoryLimit = (mem_mb * 1024 * 1024) as usize;
        }
    }

    unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ).context("SetInformationJobObject extended")?;
    }

    // CPU rate control
    if let Some(l) = limits {
        if let Some(cpu_pct) = l.cpu_pct {
            let cpu_pct = cpu_pct.clamp(1, 100);
            let rate_info = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION {
                ControlFlags: JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
                Anonymous: windows::Win32::System::JobObjects::JOBOBJECT_CPU_RATE_CONTROL_INFORMATION_0 {
                    CpuRate: (cpu_pct * 100) as u32, // in units of 1/10000
                },
            };
            unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectCpuRateControlInformation,
                    &rate_info as *const _ as *const _,
                    std::mem::size_of_val(&rate_info) as u32,
                ).context("SetInformationJobObject CPU rate")?;
            }
        }
    }

    Ok(())
}

#[cfg(windows)]
fn build_env_block(env: &HashMap<String, String>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let mut block = Vec::new();
    for (k, v) in env {
        let entry = format!("{k}={v}");
        block.extend(std::ffi::OsStr::new(&entry).encode_wide());
        block.push(0u16); // null-terminate each string
    }
    block.push(0u16); // double-null termination
    block
}

#[cfg(windows)]
fn build_cmdline(exe: &str, args: &[String]) -> String {
    let mut parts = vec![quote_arg(exe)];
    parts.extend(args.iter().map(|a| quote_arg(a)));
    parts.join(" ")
}

#[cfg(windows)]
fn quote_arg(s: &str) -> String {
    if s.contains(' ') || s.contains('"') {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

// ── STUBS for non-Windows builds ──────────────────────────────────────────────

#[cfg(not(windows))]
pub async fn spawn_node(
    _msg: &SpawnNodeMsg, _node_id: Uuid, _slot_idx: usize, _core_pipe: &str,
) -> Result<NodeHandle> {
    bail!("Job Object spawning only supported on Windows")
}

#[cfg(not(windows))]
pub fn terminate_node(_handle: &NodeHandle, _exit_code: u32) -> Result<()> {
    bail!("Job Object termination only supported on Windows")
}
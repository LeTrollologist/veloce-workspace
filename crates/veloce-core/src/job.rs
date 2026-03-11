/*!
# Job Object Node Spawner

Each sideloaded node runs inside its own Windows Job Object.

Security properties enforced:
- `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: node dies when VeloceCore exits.
- `JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION`: crash doesn't linger.
- Optional CPU rate / memory limits via `JOBOBJECT_CPU_RATE_CONTROL_INFORMATION`
  and `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`.

stdout and stderr are captured via anonymous pipes and forwarded to subscribers
as `NodeLogChunk` push messages.  Crashed nodes are automatically restarted
according to the `RestartPolicy` stored on the original `SpawnNodeMsg`.
*/

use anyhow::{bail, Context, Result};
use std::{collections::HashMap, sync::Arc, time::Duration};
use uuid::Uuid;

use veloce_ipc::message::{
    LogStream, NodeEvent, NodeLimits, NodeLogChunkMsg, RestartPolicy, SpawnNodeMsg,
};

use crate::state::CoreState;

// Windows API surface
#[cfg(windows)]
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{CloseHandle, BOOL, HANDLE},
        Security::SECURITY_ATTRIBUTES,
        Storage::FileSystem::ReadFile,
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW,
                JobObjectCpuRateControlInformation, JobObjectExtendedLimitInformation,
                SetInformationJobObject, JOBOBJECT_CPU_RATE_CONTROL_INFORMATION,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_CPU_RATE_CONTROL_ENABLE,
                JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
            },
            Pipes::CreatePipe,
            Threading::{
                CreateProcessW, GetExitCodeProcess, TerminateProcess, WaitForSingleObject,
                CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
                STARTUPINFOW,
            },
        },
    },
};

// ── NODE HANDLE ───────────────────────────────────────────────────────────────

/// A live node managed by VeloceCore.
pub struct NodeHandle {
    pub node_id:        Uuid,
    pub slot_idx:       usize,
    pub pid:            u32,
    pub app_name:       String,
    pub pipe_path:      String,
    /// Original spawn message — kept so the health loop can restart the node.
    pub spawn_msg:      SpawnNodeMsg,
    /// Optional restart-on-crash policy.
    pub restart_policy: Option<RestartPolicy>,
    /// How many times this node has already been restarted.
    pub restart_count:  u32,
    #[cfg(windows)]
    job_handle:  SafeHandle,
    #[cfg(windows)]
    proc_handle: SafeHandle,
    /// Broadcast channel for node lifecycle events.
    pub event_tx: tokio::sync::broadcast::Sender<NodeEventMsg>,
    /// Broadcast channel for captured stdout/stderr chunks.
    pub log_tx: tokio::sync::broadcast::Sender<NodeLogChunkMsg>,
}

impl NodeHandle {
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
///
/// Pass `existing_event_tx` / `existing_log_tx` to reuse existing broadcast
/// channels on restart so current subscribers keep receiving events automatically.
#[cfg(windows)]
pub async fn spawn_node(
    msg:               &SpawnNodeMsg,
    node_id:           Uuid,
    slot_idx:          usize,
    core_pipe:         &str,
    existing_event_tx: Option<tokio::sync::broadcast::Sender<NodeEventMsg>>,
    existing_log_tx:   Option<tokio::sync::broadcast::Sender<NodeLogChunkMsg>>,
) -> Result<NodeHandle> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let event_tx = existing_event_tx
        .unwrap_or_else(|| tokio::sync::broadcast::channel(32).0);
    let log_tx = existing_log_tx
        .unwrap_or_else(|| tokio::sync::broadcast::channel(256).0);

    let pipe_path = format!(r"\\.\pipe\VeloceNode-{}", node_id.simple());

    // ── Create Job Object ────────────────────────────────────────────────────
    let job_name   = format!("VeloceNode-{}\0", node_id.simple());
    let job_name_w: Vec<u16> = OsStr::new(&job_name).encode_wide().collect();

    let job_handle = unsafe {
        let h = CreateJobObjectW(None, PCWSTR(job_name_w.as_ptr()))
            .context("CreateJobObject")?;
        SafeHandle(h)
    };

    apply_job_limits(job_handle.0, msg.limits.as_ref())?;

    // ── Build environment block ──────────────────────────────────────────────
    let mut env_map: HashMap<String, String> = std::env::vars().collect();
    for (k, v) in &msg.env {
        env_map.insert(k.clone(), v.clone());
    }
    env_map.insert("VELOCE_PIPE".into(), core_pipe.into());
    env_map.insert("VELOCE_NODE_ID".into(), node_id.to_string());
    env_map.insert("VELOCE_NODE_PIPE".into(), pipe_path.clone());

    let env_block = build_env_block(&env_map);

    // ── Create stdout / stderr capture pipes ─────────────────────────────────
    // Child inherits the write ends; parent reads the read ends.
    let sa = SECURITY_ATTRIBUTES {
        nLength:              std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle:       BOOL(1),
    };

    let (mut stdout_read, mut stdout_write) = (HANDLE::default(), HANDLE::default());
    let (mut stderr_read, mut stderr_write) = (HANDLE::default(), HANDLE::default());

    unsafe {
        CreatePipe(&mut stdout_read, &mut stdout_write, Some(&sa), 0)
            .context("CreatePipe stdout")?;
        CreatePipe(&mut stderr_read, &mut stderr_write, Some(&sa), 0)
            .context("CreatePipe stderr")?;
    }

    // ── CreateProcess ────────────────────────────────────────────────────────
    let cmdline   = build_cmdline(&msg.executable, &msg.args);
    let cmdline_w: Vec<u16> = OsStr::new(&cmdline).encode_wide().chain(Some(0)).collect();

    let mut si = STARTUPINFOW {
        cb:         std::mem::size_of::<STARTUPINFOW>() as u32,
        dwFlags:    STARTF_USESTDHANDLES,
        hStdOutput: stdout_write,
        hStdError:  stderr_write,
        ..Default::default()
    };
    let mut pi = PROCESS_INFORMATION::default();

    unsafe {
        CreateProcessW(
            PCWSTR::null(),
            windows::core::PWSTR(cmdline_w.as_ptr() as *mut u16),
            None,
            None,
            BOOL(1), // bInheritHandles = TRUE so child gets the write pipe ends
            CREATE_UNICODE_ENVIRONMENT,
            Some(env_block.as_ptr() as *const _),
            PCWSTR::null(),
            &si,
            &mut pi,
        ).context("CreateProcess")?;
    }

    let pid = pi.dwProcessId;
    let proc_handle = SafeHandle(pi.hProcess);
    unsafe { let _ = CloseHandle(pi.hThread); }

    // Close the write ends in the parent — the child owns them now.
    // Keeping them open would prevent ReadFile from returning EOF.
    unsafe {
        let _ = CloseHandle(stdout_write);
        let _ = CloseHandle(stderr_write);
    }

    // ── Assign to Job Object ─────────────────────────────────────────────────
    unsafe {
        AssignProcessToJobObject(job_handle.0, pi.hProcess)
            .context("AssignProcessToJobObject")?;
    }

    tracing::info!(%node_id, pid, app_name = %msg.app_name, "node spawned");

    // ── Spawn log reader tasks ───────────────────────────────────────────────
    spawn_log_reader(stdout_read.0 as isize, node_id, LogStream::Stdout, log_tx.clone());
    spawn_log_reader(stderr_read.0 as isize, node_id, LogStream::Stderr, log_tx.clone());

    Ok(NodeHandle {
        node_id,
        slot_idx,
        pid,
        app_name:       msg.app_name.clone(),
        pipe_path,
        spawn_msg:      msg.clone(),
        restart_policy: msg.restart_policy.clone(),
        restart_count:  0,
        job_handle,
        proc_handle,
        event_tx,
        log_tx,
    })
}

/// Terminate a node via TerminateProcess.
#[cfg(windows)]
pub fn terminate_node(handle: &NodeHandle, exit_code: u32) -> Result<()> {
    unsafe {
        TerminateProcess(handle.proc_handle.0, exit_code)
            .context("TerminateProcess")?;
    }
    tracing::info!(node_id = %handle.node_id, "node terminated");
    Ok(())
}

// ── LOG READER ────────────────────────────────────────────────────────────────

/// Spawn a blocking thread that reads from a pipe handle and sends chunks to
/// `log_tx`.  Exits when the pipe read end returns EOF.
#[cfg(windows)]
fn spawn_log_reader(
    handle_raw: isize,
    node_id:    Uuid,
    stream:     LogStream,
    log_tx:     tokio::sync::broadcast::Sender<NodeLogChunkMsg>,
) {
    tokio::task::spawn_blocking(move || {
        let handle = HANDLE(handle_raw);
        let mut buf = [0u8; 4096];
        loop {
            let mut bytes_read = 0u32;
            let result = unsafe {
                ReadFile(handle, Some(&mut buf), Some(&mut bytes_read), None)
            };
            if result.is_err() || bytes_read == 0 {
                break;
            }
            let data = buf[..bytes_read as usize].to_vec();
            let _ = log_tx.send(NodeLogChunkMsg { node_id, stream, data });
        }
        unsafe { let _ = CloseHandle(handle); }
    });
}

// ── HEALTH MONITOR ────────────────────────────────────────────────────────────

/// Background task: poll live nodes every 2 s, emit events, apply restart policy.
pub async fn health_loop(state: Arc<CoreState>) {
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        check_nodes(&state).await;
    }
}

#[cfg(windows)]
async fn check_nodes(state: &Arc<CoreState>) {
    use windows::Win32::Foundation::WAIT_TIMEOUT;

    let nodes = state.node_table().list_live();
    for info in nodes {
        if info.proc_handle_raw == 0 { continue; }

        let raw_handle = HANDLE(info.proc_handle_raw);
        let still_running = unsafe {
            WaitForSingleObject(raw_handle, 0) == WAIT_TIMEOUT
        };
        if still_running { continue; }

        let mut exit_code = 0u32;
        unsafe { let _ = GetExitCodeProcess(raw_handle, &mut exit_code); }

        tracing::info!(node_id = %info.node_id, exit_code, "node exited");

        let _ = state.registry().set_node_status(
            info.slot_idx,
            crate::registry::NodeStatus::Stopped,
            exit_code,
        );

        // Take full ownership of the handle for restart inspection
        if let Some(handle) = state.node_table().remove(info.node_id) {
            let _ = handle.event_tx.send(NodeEventMsg {
                node_id: info.node_id,
                event:   NodeEvent::Exited { exit_code },
            });

            // ── Restart logic ─────────────────────────────────────────────
            if let Some(ref policy) = handle.restart_policy {
                if handle.restart_count < policy.max_restarts {
                    let delay   = back_off_secs(policy, handle.restart_count);
                    let attempt = handle.restart_count + 1;

                    let _ = handle.event_tx.send(NodeEventMsg {
                        node_id: info.node_id,
                        event:   NodeEvent::Restarting { attempt, delay_secs: delay },
                    });

                    // Clone everything the restart task needs; keep broadcast
                    // channels so existing subscribers receive the new events.
                    let event_tx  = handle.event_tx.clone();
                    let log_tx    = handle.log_tx.clone();
                    let spawn_msg = handle.spawn_msg.clone();
                    let node_id   = handle.node_id;
                    let slot_idx  = handle.slot_idx;
                    let state2    = state.clone();

                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        match spawn_node(
                            &spawn_msg, node_id, slot_idx,
                            veloce_ipc::PIPE_NAME,
                            Some(event_tx.clone()), Some(log_tx),
                        ).await {
                            Ok(mut new_handle) => {
                                new_handle.restart_count  = attempt;
                                new_handle.restart_policy = spawn_msg.restart_policy.clone();
                                let pid = new_handle.pid;
                                state2.node_table().insert(new_handle);
                                let _ = event_tx.send(NodeEventMsg {
                                    node_id,
                                    event: NodeEvent::Started { pid },
                                });
                                tracing::info!(%node_id, pid, attempt, "node restarted");
                            }
                            Err(e) => {
                                tracing::error!(%node_id, "restart attempt {attempt} failed: {e:#}");
                            }
                        }
                    });
                } else {
                    tracing::warn!(
                        node_id = %info.node_id,
                        max = policy.max_restarts,
                        "node exhausted restart budget"
                    );
                }
            }
        }
    }
}

/// Exponential back-off: `min(base * 2^count, max_delay)`.
fn back_off_secs(policy: &RestartPolicy, count: u32) -> u64 {
    let delay = policy.base_delay_secs.saturating_mul(1u64 << count.min(10));
    delay.min(policy.max_delay_secs)
}

#[cfg(not(windows))]
async fn check_nodes(_state: &Arc<CoreState>) {}

// ── WIN32 HELPERS ─────────────────────────────────────────────────────────────

#[cfg(windows)]
fn apply_job_limits(job: HANDLE, limits: Option<&NodeLimits>) -> Result<()> {
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

    if let Some(l) = limits {
        if let Some(cpu_pct) = l.cpu_pct {
            let cpu_pct = cpu_pct.clamp(1, 100);
            let rate_info = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION {
                ControlFlags: JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
                Anonymous: windows::Win32::System::JobObjects::JOBOBJECT_CPU_RATE_CONTROL_INFORMATION_0 {
                    CpuRate: (cpu_pct * 100) as u32,
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
        block.push(0u16);
    }
    block.push(0u16);
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
    _msg:               &SpawnNodeMsg,
    _node_id:           Uuid,
    _slot_idx:          usize,
    _core_pipe:         &str,
    _existing_event_tx: Option<tokio::sync::broadcast::Sender<NodeEventMsg>>,
    _existing_log_tx:   Option<tokio::sync::broadcast::Sender<NodeLogChunkMsg>>,
) -> Result<NodeHandle> {
    bail!("Job Object spawning only supported on Windows")
}

#[cfg(not(windows))]
pub fn terminate_node(_handle: &NodeHandle, _exit_code: u32) -> Result<()> {
    bail!("Job Object termination only supported on Windows")
}

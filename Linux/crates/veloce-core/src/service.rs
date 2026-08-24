/*!
Windows Service lifecycle management.

Handles:
- SCM dispatch table entry
- Service installation / uninstall via SCM API
- SERVICE_STATUS updates (START_PENDING → RUNNING → STOP_PENDING → STOPPED)
- Recovery actions: restart on crash x3, then alert
*/

#[cfg(windows)]
use anyhow::Context;
#[cfg(windows)]
use std::ffi::OsString;
#[cfg(windows)]
use std::time::Duration;
#[cfg(windows)]
use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl,
        ServiceExitCode, ServiceInfo, ServiceStartType, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

#[cfg(windows)]
pub const SERVICE_NAME: &str = "VeloceCore";
#[cfg(windows)]
pub const SERVICE_DISPLAY: &str = "VeloceCore Runtime";
#[cfg(windows)]
pub const SERVICE_DESC: &str =
    "VeloceSolutions privileged runtime — local node orchestration and private network layer.";

// ── SCM DISPATCH ──────────────────────────────────────────────────────────────

#[cfg(windows)]
define_windows_service!(ffi_service_main, service_main);

#[cfg(windows)]
pub fn dispatch() -> anyhow::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .context("service dispatcher failed")?;
    Ok(())
}

#[cfg(windows)]
fn service_main(args: Vec<OsString>) {
    if let Err(e) = run_service(args) {
        tracing::error!("Service fatal error: {e:#}");
    }
}

#[cfg(windows)]
fn run_service(_args: Vec<OsString>) -> anyhow::Result<()> {
    // Channel for the stop signal from SCM → async runtime
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

    // Register our control handler with the SCM
    let status_handle = service_control_handler::register(SERVICE_NAME, move |ctrl| {
        match ctrl {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = stop_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    })?;

    // Report: START_PENDING
    status_handle.set_service_status(ServiceStatus {
        service_type:             ServiceType::OWN_PROCESS,
        current_state:            ServiceState::StartPending,
        controls_accepted:        ServiceControlAccept::empty(),
        exit_code:                ServiceExitCode::Win32(0),
        checkpoint:               0,
        wait_hint:                Duration::from_secs(5),
        process_id:               None,
    })?;

    // Boot async runtime on a background thread so we don't block the SCM thread
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("veloce-worker")
        .build()
        .context("tokio runtime")?;

    let (core_done_tx, core_done_rx) = std::sync::mpsc::channel::<anyhow::Result<()>>();

    std::thread::spawn(move || {
        let result = rt.block_on(crate::run_core());
        let _ = core_done_tx.send(result);
    });

    // Report: RUNNING
    status_handle.set_service_status(ServiceStatus {
        service_type:      ServiceType::OWN_PROCESS,
        current_state:     ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code:         ServiceExitCode::Win32(0),
        checkpoint:        0,
        wait_hint:         Duration::ZERO,
        process_id:        None,
    })?;

    // Wait for SCM stop signal or the runtime to exit on its own
    loop {
        // Poll for stop signal (1-second timeout so we can also check core_done)
        if stop_rx.recv_timeout(Duration::from_secs(1)).is_ok() {
            tracing::info!("SCM stop received");
            break;
        }
        if core_done_rx.try_recv().is_ok() {
            tracing::info!("Core runtime exited");
            break;
        }
    }

    // Report: STOP_PENDING
    status_handle.set_service_status(ServiceStatus {
        service_type:      ServiceType::OWN_PROCESS,
        current_state:     ServiceState::StopPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code:         ServiceExitCode::Win32(0),
        checkpoint:        0,
        wait_hint:         Duration::from_secs(5),
        process_id:        None,
    })?;

    // Report: STOPPED
    status_handle.set_service_status(ServiceStatus {
        service_type:      ServiceType::OWN_PROCESS,
        current_state:     ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code:         ServiceExitCode::Win32(0),
        checkpoint:        0,
        wait_hint:         Duration::ZERO,
        process_id:        None,
    })?;

    Ok(())
}

// ── INSTALL (Windows) ─────────────────────────────────────────────────────────

#[cfg(windows)]
pub fn install() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE)
        .context("open SCM (run as Administrator)")?;

    let exe_path = std::env::current_exe().context("current exe path")?;

    let info = ServiceInfo {
        name:              SERVICE_NAME.into(),
        display_name:      SERVICE_DISPLAY.into(),
        service_type:      ServiceType::OWN_PROCESS,
        start_type:        ServiceStartType::AutoStart,
        error_control:     ServiceErrorControl::Normal,
        executable_path:   exe_path,
        launch_arguments:  vec![],
        dependencies:      vec![],
        account_name:      None, // LocalSystem
        account_password:  None,
    };

    let service = manager
        .create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)
        .context("create service")?;

    service.set_description(SERVICE_DESC).context("set description")?;

    // Set recovery actions: restart 3x (with 5s delay), then alert
    set_recovery_actions(&service)?;

    // Install NRPT rule so *.vln resolves system-wide (best-effort)
    if let Err(e) = crate::nrpt::install(crate::nrpt::VLN_DNS_ADDR) {
        tracing::warn!("NRPT rule install failed (non-fatal): {e:#}");
    }

    println!("VeloceCore service installed. Run: sc start {SERVICE_NAME}");
    Ok(())
}

#[cfg(windows)]
pub fn uninstall() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("open SCM")?;

    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::DELETE | ServiceAccess::STOP)
        .context("open service (not installed?)")?;

    // Stop first if running
    let _ = service.stop();
    std::thread::sleep(Duration::from_secs(2));

    service.delete().context("delete service")?;

    // Remove NRPT rule (best-effort)
    let _ = crate::nrpt::uninstall();

    println!("VeloceCore service removed.");
    Ok(())
}

#[cfg(windows)]
pub fn start_service() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("open SCM")?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::START)
        .context("open service")?;
    service.start(&[] as &[&std::ffi::OsStr]).context("start service")?;
    println!("VeloceCore started.");
    Ok(())
}

#[cfg(windows)]
pub fn stop_service() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("open SCM")?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::STOP)
        .context("open service")?;
    service.stop().context("stop service")?;
    println!("VeloceCore stopped.");
    Ok(())
}

// ── RECOVERY ACTIONS (Windows) ────────────────────────────────────────────────

/// Set recovery policy: restart on first 3 failures (5 s delay each), then log event.
#[cfg(windows)]
fn set_recovery_actions(service: &windows_service::service::Service) -> anyhow::Result<()> {
    use windows_service::service::{
        ServiceActionType, ServiceFailureActions, ServiceFailureResetPeriod,
        ServiceAction,
    };

    let actions = ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86400)),
        reboot_msg:   None,
        command:      None,
        actions:      Some(vec![
            ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(5) },
            ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(5) },
            ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(5) },
            ServiceAction { action_type: ServiceActionType::None,    delay: Duration::ZERO },
        ]),
    };

    service.update_failure_actions(actions).context("set recovery actions")?;
    Ok(())
}

// ── Linux / Unix service management ──────────────────────────────────────────

/// Install the VeloceCore systemd unit.
#[cfg(unix)]
pub fn install() -> anyhow::Result<()> {
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("/usr/bin/veloce-core"));
    crate::systemd::install(&exe)
}

/// Remove the VeloceCore systemd unit.
#[cfg(unix)]
pub fn uninstall() -> anyhow::Result<()> {
    crate::systemd::uninstall()
}

/// Start VeloceCore via systemctl.
#[cfg(unix)]
pub fn start_service() -> anyhow::Result<()> {
    crate::systemd::start()
}

/// Stop VeloceCore via systemctl.
#[cfg(unix)]
pub fn stop_service() -> anyhow::Result<()> {
    crate::systemd::stop()
}

/// On Linux there is no SCM dispatch — systemd invokes the binary directly.
/// Call `notify_ready()` and then run the core loop.
#[cfg(unix)]
pub fn dispatch(run_fn: impl FnOnce() -> anyhow::Result<()>) -> anyhow::Result<()> {
    crate::systemd::notify_ready();
    run_fn()
}

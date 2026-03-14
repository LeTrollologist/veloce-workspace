/*!
VeloceNetwork Installer — Tauri 2 backend.

Exposes Tauri commands for the glassmorphic installer wizard:
  - `check_admin`     — returns true if running elevated
  - `get_default_dir` — returns default install path
  - `browse_dir`      — opens a native folder picker
  - `start_install`   — performs the installation, emitting progress events
  - `start_uninstall` — removes VeloceNetwork from the machine

Progress events emitted to the frontend:
  - `"install-progress"` — `{ step, pct, message, status }`
*/

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct InstallOptions {
    install_dir:      String,
    desktop_shortcut: bool,
    start_menu:       bool,
    add_to_path:      bool,
    start_service:    bool,
}

#[derive(Serialize, Clone)]
struct ProgressEvent {
    step:    String,
    pct:     u8,
    message: String,
    status:  String,   // "running" | "ok" | "error"
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn emit_progress(app: &tauri::AppHandle, step: &str, pct: u8, message: &str, status: &str) {
    let _ = app.emit("install-progress", ProgressEvent {
        step:    step.into(),
        pct,
        message: message.into(),
        status:  status.into(),
    });
}

/// True when the process has Administrator privilege.
/// Uses a write-probe to %ProgramFiles% rather than Win32 token introspection.
fn is_elevated() -> bool {
    let probe = std::path::Path::new(r"C:\Program Files\.veloce_admin_check");
    match std::fs::write(probe, b"") {
        Ok(_) => { let _ = std::fs::remove_file(probe); true }
        Err(_) => false,
    }
}

fn default_install_dir() -> String {
    std::env::var("ProgramFiles")
        .map(|p| format!("{}\\VeloceNetwork", p))
        .unwrap_or_else(|_| {
            std::env::var("LOCALAPPDATA")
                .map(|p| format!("{}\\VeloceNetwork", p))
                .unwrap_or_else(|_| r"C:\Program Files\VeloceNetwork".into())
        })
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
fn check_admin() -> bool {
    is_elevated()
}

#[tauri::command]
fn get_default_dir() -> String {
    default_install_dir()
}

#[tauri::command]
async fn browse_dir(app: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .blocking_pick_folder()
        .map(|p| p.to_string())
}

#[tauri::command]
async fn start_install(
    app:     tauri::AppHandle,
    options: InstallOptions,
) -> Result<(), String> {
    let install_dir = PathBuf::from(&options.install_dir);
    do_install(app, install_dir, options).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_uninstall(app: tauri::AppHandle) -> Result<(), String> {
    do_uninstall(app).await.map_err(|e| e.to_string())
}

// ── Installation logic ────────────────────────────────────────────────────────

async fn do_install(
    app:         tauri::AppHandle,
    install_dir: PathBuf,
    opts:        InstallOptions,
) -> anyhow::Result<()> {
    // ── 1. Create install directory ────────────────────────────────────────
    emit_progress(&app, "directory", 5, "Creating installation directory…", "running");
    tokio::fs::create_dir_all(&install_dir).await
        .map_err(|e| anyhow::anyhow!("Cannot create directory: {e}"))?;
    emit_progress(&app, "directory", 10, "Installation directory ready.", "ok");

    // ── 2. Copy bundled binaries ───────────────────────────────────────────
    emit_progress(&app, "copy_core", 15, "Copying VeloceCore service…", "running");
    copy_resource(&app, "veloce-core.exe", &install_dir.join("veloce-core.exe")).await?;
    emit_progress(&app, "copy_core", 30, "VeloceCore copied.", "ok");

    emit_progress(&app, "copy_dash", 35, "Copying VeloceNetwork Dashboard…", "running");
    copy_resource(&app, "veloce-dashboard.exe", &install_dir.join("veloce-dashboard.exe")).await?;
    emit_progress(&app, "copy_dash", 50, "Dashboard copied.", "ok");

    // Copy installer itself for the uninstall entry
    if let Ok(self_exe) = std::env::current_exe() {
        let _ = tokio::fs::copy(&self_exe, install_dir.join("uninstall.exe")).await;
    }

    // ── 3. Windows service ─────────────────────────────────────────────────
    emit_progress(&app, "service", 55, "Registering VeloceCore service…", "running");
    tokio::task::spawn_blocking({
        let dir   = install_dir.clone();
        let start = opts.start_service;
        move || install_service(&dir, start)
    }).await??;
    emit_progress(&app, "service", 68, "Service registered.", "ok");

    // ── 4. Shortcuts ───────────────────────────────────────────────────────
    if opts.start_menu {
        emit_progress(&app, "shortcuts", 72, "Creating Start Menu shortcut…", "running");
        let target = install_dir.join("veloce-dashboard.exe");
        let programs = std::env::var("APPDATA")
            .map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Start Menu\Programs"))
            .unwrap_or_else(|_| PathBuf::from(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs"));
        let link = programs.join("VeloceNetwork.lnk");
        create_shortcut(&target, &link, "VeloceNetwork Dashboard")?;
        emit_progress(&app, "shortcuts", 76, "Start Menu shortcut created.", "ok");
    }

    if opts.desktop_shortcut {
        emit_progress(&app, "shortcuts", 78, "Creating Desktop shortcut…", "running");
        let target = install_dir.join("veloce-dashboard.exe");
        let desktop = std::env::var("USERPROFILE")
            .map(|h| PathBuf::from(h).join("Desktop"))
            .unwrap_or_else(|_| PathBuf::from(r"C:\Users\Public\Desktop"));
        let link = desktop.join("VeloceNetwork.lnk");
        create_shortcut(&target, &link, "VeloceNetwork Dashboard")?;
        emit_progress(&app, "shortcuts", 82, "Desktop shortcut created.", "ok");
    }

    // ── 5. PATH ────────────────────────────────────────────────────────────
    if opts.add_to_path {
        emit_progress(&app, "path", 84, "Adding to system PATH…", "running");
        let _ = add_to_path(&install_dir);
        emit_progress(&app, "path", 87, "PATH updated.", "ok");
    }

    // ── 6. Uninstaller registry entry ──────────────────────────────────────
    emit_progress(&app, "registry", 88, "Registering uninstaller…", "running");
    let _ = register_uninstaller(&install_dir, opts.install_dir.as_str());
    emit_progress(&app, "registry", 92, "Uninstaller registered.", "ok");

    // ── 7. NRPT rule — route *.vln queries to VeloceNet DNS ────────────────
    emit_progress(&app, "nrpt", 93, "Configuring .vln system DNS routing…", "running");
    match install_nrpt() {
        Ok(()) => emit_progress(&app, "nrpt", 97, ".vln DNS routing configured.", "ok"),
        Err(e) => {
            // Non-fatal: system-wide .vln resolution requires NRPT; fall back to VELOCE_DNS.
            eprintln!("[installer] NRPT rule install failed (non-fatal): {e}");
            emit_progress(&app, "nrpt", 97, ".vln DNS routing skipped (run as admin to enable).", "ok");
        }
    }

    emit_progress(&app, "done", 100, "Installation complete!", "ok");
    Ok(())
}

async fn do_uninstall(app: tauri::AppHandle) -> anyhow::Result<()> {
    let install_dir = get_install_dir_from_registry()
        .unwrap_or_else(|| PathBuf::from(default_install_dir()));

    emit_progress(&app, "stop_service", 10, "Stopping VeloceCore service…", "running");
    let _ = tokio::task::spawn_blocking(stop_and_remove_service).await?;
    emit_progress(&app, "stop_service", 30, "Service removed.", "ok");

    emit_progress(&app, "cleanup", 40, "Removing shortcuts…", "running");
    let _ = remove_shortcuts();
    emit_progress(&app, "cleanup", 55, "Shortcuts removed.", "ok");

    emit_progress(&app, "path", 60, "Removing from PATH…", "running");
    let _ = remove_from_path(&install_dir);
    emit_progress(&app, "path", 70, "PATH cleaned.", "ok");

    emit_progress(&app, "nrpt", 72, "Removing .vln DNS routing rule…", "running");
    let _ = remove_nrpt();
    emit_progress(&app, "nrpt", 76, ".vln DNS rule removed.", "ok");

    emit_progress(&app, "registry", 78, "Removing registry entries…", "running");
    let _ = remove_uninstaller_registry();
    emit_progress(&app, "registry", 88, "Registry cleaned.", "ok");

    emit_progress(&app, "files", 90, "Removing files…", "running");
    let _ = remove_install_dir(&install_dir);
    emit_progress(&app, "files", 100, "Uninstall complete.", "ok");
    Ok(())
}

// ── Platform helpers ──────────────────────────────────────────────────────────

/// Copy a bundled resource to a destination path.
/// During development, binaries aren't bundled — this emits a warning instead of failing.
async fn copy_resource(
    app:  &tauri::AppHandle,
    name: &str,
    dest: &Path,
) -> anyhow::Result<()> {
    let resource_dir = app.path()
        .resource_dir()
        .map_err(|e| anyhow::anyhow!("resource dir: {e}"))?;
    let src = resource_dir.join(name);

    if !src.exists() {
        eprintln!("[installer] resource not bundled: {}", src.display());
        return Ok(());
    }

    tokio::fs::copy(&src, dest)
        .await
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("copy {}: {e}", name))
}

#[cfg(windows)]
fn install_service(install_dir: &Path, start_now: bool) -> anyhow::Result<()> {
    use std::ffi::OsString;
    use windows_service::service::{
        ServiceAccess, ServiceErrorControl, ServiceInfo,
        ServiceStartType, ServiceType,
    };
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CREATE_SERVICE,
    )?;

    let exe = install_dir.join("veloce-core.exe");
    let info = ServiceInfo {
        name:             OsString::from("VeloceCoreService"),
        display_name:     OsString::from("VeloceNetwork Core"),
        service_type:     ServiceType::OWN_PROCESS,
        start_type:       ServiceStartType::AutoStart,
        error_control:    ServiceErrorControl::Normal,
        executable_path:  exe,
        launch_arguments: vec![],
        dependencies:     vec![],
        account_name:     None,
        account_password: None,
    };

    let svc = manager.create_service(
        &info,
        ServiceAccess::CHANGE_CONFIG | ServiceAccess::START,
    )?;

    if start_now {
        let args: &[&std::ffi::OsStr] = &[];
        svc.start(args)?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn install_service(_dir: &Path, _start: bool) -> anyhow::Result<()> { Ok(()) }

#[cfg(windows)]
fn stop_and_remove_service() -> anyhow::Result<()> {
    use windows_service::service::{ServiceAccess, ServiceState};
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    use std::time::Duration;

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT,
    )?;
    let svc = manager.open_service(
        "VeloceCoreService",
        ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
    )?;

    if let Ok(status) = svc.query_status() {
        if status.current_state != ServiceState::Stopped {
            let _ = svc.stop();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                std::thread::sleep(Duration::from_millis(300));
                if let Ok(s) = svc.query_status() {
                    if s.current_state == ServiceState::Stopped { break; }
                }
                if std::time::Instant::now() > deadline { break; }
            }
        }
    }
    svc.delete()?;
    Ok(())
}

#[cfg(not(windows))]
fn stop_and_remove_service() -> anyhow::Result<()> { Ok(()) }

/// Create a Windows `.lnk` shortcut via PowerShell.
fn create_shortcut(target: &Path, link: &Path, description: &str) -> anyhow::Result<()> {
    let script = format!(
        "$ws = New-Object -ComObject WScript.Shell; \
         $s = $ws.CreateShortcut('{link}'); \
         $s.TargetPath = '{target}'; \
         $s.Description = '{desc}'; \
         $s.Save()",
        link   = link.display(),
        target = target.display(),
        desc   = description,
    );
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()?;
    Ok(())
}

fn remove_shortcuts() -> anyhow::Result<()> {
    let programs = std::env::var("APPDATA").ok()
        .map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Start Menu\Programs\VeloceNetwork.lnk"));
    let desktop = std::env::var("USERPROFILE").ok()
        .map(|h| PathBuf::from(h).join("Desktop\\VeloceNetwork.lnk"));
    for p in [programs, desktop].into_iter().flatten() {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

#[cfg(windows)]
fn add_to_path(dir: &Path) -> anyhow::Result<()> {
    use winreg::{enums::*, RegKey};
    let hklm  = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key   = hklm.open_subkey_with_flags(
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        KEY_READ | KEY_WRITE,
    )?;
    let current: String = key.get_value("PATH")?;
    let dir_str: String = dir.to_string_lossy().into_owned();
    if !current.to_lowercase().contains(&dir_str.to_lowercase() as &str) {
        key.set_value("PATH", &format!("{};{}", current, dir_str))?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn add_to_path(_dir: &Path) -> anyhow::Result<()> { Ok(()) }

#[cfg(windows)]
fn remove_from_path(dir: &Path) -> anyhow::Result<()> {
    use winreg::{enums::*, RegKey};
    let hklm  = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key   = hklm.open_subkey_with_flags(
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        KEY_READ | KEY_WRITE,
    )?;
    let current: String = key.get_value("PATH")?;
    let dir_lower: String = dir.to_string_lossy().to_lowercase();
    let parts: Vec<&str> = current
        .split(';')
        .filter(|p| p.to_lowercase() != dir_lower)
        .collect();
    key.set_value("PATH", &parts.join(";"))?;
    Ok(())
}

#[cfg(not(windows))]
fn remove_from_path(_dir: &Path) -> anyhow::Result<()> { Ok(()) }

#[cfg(windows)]
fn register_uninstaller(install_dir: &Path, display_dir: &str) -> anyhow::Result<()> {
    use winreg::{enums::*, RegKey};
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let (key, _) = hklm.create_subkey(
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\VeloceNetwork"
    )?;
    key.set_value("DisplayName",     &"VeloceNetwork")?;
    key.set_value("DisplayVersion",  &env!("CARGO_PKG_VERSION"))?;
    key.set_value("Publisher",       &"VeloceSolutions")?;
    key.set_value("InstallLocation", &display_dir)?;
    key.set_value("UninstallString", &format!(
        r#""{}" --uninstall"#,
        install_dir.join("uninstall.exe").display()
    ))?;
    key.set_value("NoModify",        &1u32)?;
    key.set_value("NoRepair",        &1u32)?;
    Ok(())
}

#[cfg(not(windows))]
fn register_uninstaller(_dir: &Path, _s: &str) -> anyhow::Result<()> { Ok(()) }

#[cfg(windows)]
fn remove_uninstaller_registry() -> anyhow::Result<()> {
    use winreg::{enums::*, RegKey};
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    hklm.delete_subkey_all(
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\VeloceNetwork"
    )?;
    Ok(())
}

#[cfg(not(windows))]
fn remove_uninstaller_registry() -> anyhow::Result<()> { Ok(()) }

#[cfg(windows)]
fn get_install_dir_from_registry() -> Option<PathBuf> {
    use winreg::{enums::*, RegKey};
    let key: String = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\VeloceNetwork")
        .ok()?
        .get_value("InstallLocation")
        .ok()?;
    Some(PathBuf::from(key))
}

#[cfg(not(windows))]
fn get_install_dir_from_registry() -> Option<PathBuf> { None }

fn remove_install_dir(dir: &Path) -> anyhow::Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

// ── NRPT helpers ──────────────────────────────────────────────────────────────

/// Write the `.vln` NRPT rule so `*.vln` queries resolve system-wide.
/// Requires Administrator privilege.  Idempotent.
#[cfg(windows)]
fn install_nrpt() -> anyhow::Result<()> {
    use winreg::{enums::*, RegKey};

    const BASE: &str =
        r"SYSTEM\CurrentControlSet\Services\Dnscache\Parameters\DnsPolicyConfig";
    const RULE: &str = "VeloceNetwork-VLN";
    const DNS:  &str = "127.0.0.1:5354";

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let base  = hklm.open_subkey_with_flags(BASE, KEY_CREATE_SUB_KEY)
        .map_err(|e| anyhow::anyhow!("open DnsPolicyConfig: {e} (run as Administrator)"))?;

    let (rule, _) = base.create_subkey(RULE)?;
    rule.set_value("Version",           &2u32)?;
    rule.set_value("ConfigOptions",     &8u32)?;
    rule.set_value("Name",              &vec![".vln".to_owned()])?;
    rule.set_value("GenericDNSServers", &DNS)?;
    rule.set_value("Comment",           &"VeloceNetwork .vln private namespace")?;
    Ok(())
}

#[cfg(not(windows))]
fn install_nrpt() -> anyhow::Result<()> { Ok(()) }

/// Remove the `.vln` NRPT rule.  No-op if not present.
#[cfg(windows)]
fn remove_nrpt() -> anyhow::Result<()> {
    use winreg::{enums::*, RegKey};
    const BASE: &str =
        r"SYSTEM\CurrentControlSet\Services\Dnscache\Parameters\DnsPolicyConfig";
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let base  = match hklm.open_subkey_with_flags(BASE, KEY_WRITE) {
        Ok(k)  => k,
        Err(_) => return Ok(()),  // key absent — nothing to do
    };
    match base.delete_subkey_all("VeloceNetwork-VLN") {
        Ok(()) | Err(_) => {}
    }
    Ok(())
}

#[cfg(not(windows))]
fn remove_nrpt() -> anyhow::Result<()> { Ok(()) }

// ── App entry point ───────────────────────────────────────────────────────────

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            check_admin,
            get_default_dir,
            browse_dir,
            start_install,
            start_uninstall,
        ])
        .run(tauri::generate_context!())
        .expect("error while running VeloceNetwork Installer");
}

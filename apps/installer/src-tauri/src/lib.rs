/*!
VeloceNetwork Installer — Tauri 2 backend.

Exposes Tauri commands for the glassmorphic installer wizard:
  - `check_admin`     — returns true if running elevated (informational only)
  - `get_default_dir` — returns default install path (LOCALAPPDATA, no admin needed)
  - `browse_dir`      — opens a native folder picker
  - `start_install`   — performs the installation, emitting progress events
  - `start_uninstall` — removes VeloceNetwork from the machine

No-Admin design:
  - Service registered as a per-user Task Scheduler task (schtasks, no SCM needed)
  - PATH written to HKCU\Environment (no HKLM write needed)
  - Uninstaller registered in HKCU (no HKLM write needed)
  - Shortcuts created via IShellLinkW COM (no PowerShell subprocess)
  - Default install directory is %LOCALAPPDATA%\VeloceNetwork

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
/// Uses Win32 token introspection — informational only.
/// The installer does not require elevation in normal operation.
#[cfg(windows)]
fn is_elevated() -> bool {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        ).is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

#[cfg(not(windows))]
fn is_elevated() -> bool { false }

/// Default install directory — prefers LOCALAPPDATA (no admin required).
fn default_install_dir() -> String {
    std::env::var("LOCALAPPDATA")
        .map(|p| format!("{}\\VeloceNetwork", p))
        .unwrap_or_else(|_| {
            std::env::var("ProgramFiles")
                .map(|p| format!("{}\\VeloceNetwork", p))
                .unwrap_or_else(|_| r"C:\Users\Public\VeloceNetwork".into())
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
    emit_progress(&app, "copy_core", 15, "Copying VeloceCore…", "running");
    copy_resource(&app, "veloce-core.exe", &install_dir.join("veloce-core.exe")).await?;
    emit_progress(&app, "copy_core", 30, "VeloceCore copied.", "ok");

    emit_progress(&app, "copy_dash", 35, "Copying VeloceNetwork Dashboard…", "running");
    copy_resource(&app, "veloce-dashboard.exe", &install_dir.join("veloce-dashboard.exe")).await?;
    emit_progress(&app, "copy_dash", 50, "Dashboard copied.", "ok");

    // Copy installer itself for the uninstall entry
    if let Ok(self_exe) = std::env::current_exe() {
        let _ = tokio::fs::copy(&self_exe, install_dir.join("uninstall.exe")).await;
    }

    // ── 3. Schedule startup task (no SCM / no admin required) ─────────────
    emit_progress(&app, "service", 55, "Scheduling startup task…", "running");
    tokio::task::spawn_blocking({
        let dir   = install_dir.clone();
        let start = opts.start_service;
        move || schedule_startup_task(&dir, start)
    }).await??;
    emit_progress(&app, "service", 68, "Startup task scheduled.", "ok");

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

    // ── 5. PATH (HKCU — no admin required) ────────────────────────────────
    if opts.add_to_path {
        emit_progress(&app, "path", 84, "Adding to user PATH…", "running");
        let _ = add_to_path(&install_dir);
        emit_progress(&app, "path", 87, "PATH updated.", "ok");
    }

    // ── 6. Uninstaller registry entry (HKCU — no admin required) ──────────
    emit_progress(&app, "registry", 90, "Registering uninstaller…", "running");
    let _ = register_uninstaller(&install_dir, opts.install_dir.as_str());
    emit_progress(&app, "registry", 95, "Uninstaller registered.", "ok");

    emit_progress(&app, "done", 100, "Installation complete!", "ok");
    Ok(())
}

async fn do_uninstall(app: tauri::AppHandle) -> anyhow::Result<()> {
    let install_dir = get_install_dir_from_registry()
        .unwrap_or_else(|| PathBuf::from(default_install_dir()));

    emit_progress(&app, "stop_service", 10, "Stopping VeloceCore task…", "running");
    let _ = tokio::task::spawn_blocking(remove_startup_task).await?;
    emit_progress(&app, "stop_service", 30, "Task removed.", "ok");

    emit_progress(&app, "cleanup", 40, "Removing shortcuts…", "running");
    let _ = remove_shortcuts();
    emit_progress(&app, "cleanup", 55, "Shortcuts removed.", "ok");

    emit_progress(&app, "path", 60, "Removing from PATH…", "running");
    let _ = remove_from_path(&install_dir);
    emit_progress(&app, "path", 70, "PATH cleaned.", "ok");

    emit_progress(&app, "registry", 75, "Removing registry entries…", "running");
    let _ = remove_uninstaller_registry();
    emit_progress(&app, "registry", 85, "Registry cleaned.", "ok");

    emit_progress(&app, "files", 88, "Removing files…", "running");
    let _ = remove_install_dir(&install_dir);
    emit_progress(&app, "files", 100, "Uninstall complete.", "ok");
    Ok(())
}

// ── Platform helpers ──────────────────────────────────────────────────────────

/// Copy a bundled resource to a destination path.
/// Returns an error if the resource is not bundled — installation cannot proceed
/// without the required binaries.
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
        anyhow::bail!(
            "Required binary '{}' is not bundled in this installer. \
             Please use an official release build.",
            name
        );
    }

    tokio::fs::copy(&src, dest)
        .await
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("copy {}: {e}", name))
}

/// Register VeloceCore as a per-user Task Scheduler task.
/// Runs as the current user with LIMITED rights — no SCM or admin required.
/// VeloceCore is launched with the `run` subcommand (foreground/daemon mode).
#[cfg(windows)]
fn schedule_startup_task(install_dir: &Path, start_now: bool) -> anyhow::Result<()> {
    use std::process::Command;

    let exe = install_dir.join("veloce-core.exe");
    // Wrap path in quotes so spaces in directory names are handled correctly.
    let exe_cmd = format!("\"{}\" run", exe.to_string_lossy());

    let status = Command::new("schtasks")
        .args([
            "/Create", "/F",
            "/SC",    "ONLOGON",
            "/RL",    "LIMITED",
            "/TN",    r"VeloceNetwork\VeloceCore",
            "/TR",    &exe_cmd,
            "/DELAY", "0000:30",  // 30 s grace period after logon
        ])
        .status()
        .map_err(|e| anyhow::anyhow!("schtasks unavailable: {e}"))?;

    if !status.success() {
        anyhow::bail!(
            "Failed to register startup task (schtasks exited {}). \
             Ensure the Task Scheduler service is running.",
            status
        );
    }

    if start_now {
        let _ = Command::new("schtasks")
            .args(["/Run", "/TN", r"VeloceNetwork\VeloceCore"])
            .status();
    }

    Ok(())
}

#[cfg(not(windows))]
fn schedule_startup_task(_dir: &Path, _start: bool) -> anyhow::Result<()> { Ok(()) }

/// End and delete the VeloceCore Task Scheduler task.
#[cfg(windows)]
fn remove_startup_task() -> anyhow::Result<()> {
    use std::process::Command;

    // Stop the task if running (ignore errors — may already be stopped)
    let _ = Command::new("schtasks")
        .args(["/End", "/TN", r"VeloceNetwork\VeloceCore"])
        .status();

    let status = Command::new("schtasks")
        .args(["/Delete", "/F", "/TN", r"VeloceNetwork\VeloceCore"])
        .status()
        .map_err(|e| anyhow::anyhow!("schtasks unavailable: {e}"))?;

    if !status.success() {
        anyhow::bail!("Failed to delete startup task (schtasks exited {})", status);
    }

    Ok(())
}

#[cfg(not(windows))]
fn remove_startup_task() -> anyhow::Result<()> { Ok(()) }

/// Create a Windows `.lnk` shortcut using the IShellLinkW COM interface.
///
/// Previously used a PowerShell subprocess with unsanitised path strings
/// interpolated into a single-quoted PS script — vulnerable to injection via
/// paths containing single quotes (e.g. `C:\Users\O'Brien\Desktop`).
/// The COM approach has no injection surface and requires no external process.
#[cfg(windows)]
fn create_shortcut(target: &Path, link: &Path, description: &str) -> anyhow::Result<()> {
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::BOOL,
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile,
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
            },
            UI::Shell::{IShellLinkW, ShellLink},
        },
    };

    fn to_wide_nul(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0u16)).collect()
    }

    unsafe {
        // S_OK (0) = we initialised; S_FALSE (1) = already init'd on this thread.
        // Both are success. Only call CoUninitialize if we did the initialisation.
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() {
            anyhow::bail!("CoInitializeEx failed: {:?}", hr);
        }
        let we_initialized = hr.0 == 0;

        let result = (|| -> anyhow::Result<()> {
            let sl: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| anyhow::anyhow!("CoCreateInstance(IShellLinkW): {e}"))?;

            let target_w = to_wide_nul(&target.to_string_lossy());
            sl.SetPath(PCWSTR(target_w.as_ptr()))
                .map_err(|e| anyhow::anyhow!("IShellLinkW::SetPath: {e}"))?;

            let desc_w = to_wide_nul(description);
            sl.SetDescription(PCWSTR(desc_w.as_ptr()))
                .map_err(|e| anyhow::anyhow!("IShellLinkW::SetDescription: {e}"))?;

            // Ensure the parent directory exists before saving the .lnk file
            if let Some(parent) = link.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let persist: IPersistFile = sl.cast()
                .map_err(|e| anyhow::anyhow!("cast to IPersistFile: {e}"))?;

            let link_w = to_wide_nul(&link.to_string_lossy());
            persist.Save(PCWSTR(link_w.as_ptr()), BOOL(1))
                .map_err(|e| anyhow::anyhow!("IPersistFile::Save: {e}"))?;

            Ok(())
        })();

        if we_initialized {
            CoUninitialize();
        }

        result
    }
}

#[cfg(not(windows))]
fn create_shortcut(_target: &Path, _link: &Path, _description: &str) -> anyhow::Result<()> {
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

/// Add the install directory to HKCU\Environment PATH (no admin required).
/// Broadcasts WM_SETTINGCHANGE so running shells pick up the change immediately.
#[cfg(windows)]
fn add_to_path(dir: &Path) -> anyhow::Result<()> {
    use winreg::{enums::*, RegKey};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(r"Environment")?;
    let current: String = key.get_value("PATH").unwrap_or_default();
    let dir_str = dir.to_string_lossy().into_owned();

    if !current.to_lowercase().contains(&dir_str.to_lowercase() as &str) {
        let new_path = if current.is_empty() {
            dir_str
        } else {
            format!("{};{}", current, dir_str)
        };
        key.set_value("PATH", &new_path)?;
        broadcast_env_change();
    }
    Ok(())
}

#[cfg(not(windows))]
fn add_to_path(_dir: &Path) -> anyhow::Result<()> { Ok(()) }

/// Remove the install directory from HKCU\Environment PATH.
#[cfg(windows)]
fn remove_from_path(dir: &Path) -> anyhow::Result<()> {
    use winreg::{enums::*, RegKey};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(r"Environment", KEY_READ | KEY_WRITE) {
        let current: String = key.get_value("PATH").unwrap_or_default();
        let dir_lower = dir.to_string_lossy().to_lowercase();
        let parts: Vec<&str> = current
            .split(';')
            .filter(|p| p.to_lowercase() != dir_lower)
            .collect();
        key.set_value("PATH", &parts.join(";"))?;
        broadcast_env_change();
    }
    Ok(())
}

#[cfg(not(windows))]
fn remove_from_path(_dir: &Path) -> anyhow::Result<()> { Ok(()) }

/// Broadcast WM_SETTINGCHANGE("Environment") so Explorer and open terminals
/// pick up the updated HKCU PATH without requiring a logoff/logon cycle.
#[cfg(windows)]
fn broadcast_env_change() {
    use windows::Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::WindowsAndMessaging::{SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE},
    };
    let env: Vec<u16> = "Environment\0".encode_utf16().collect();
    unsafe {
        SendMessageTimeoutW(
            HWND(0xffff_isize), // HWND_BROADCAST
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(env.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            5000,
            None,
        );
    }
}

/// Register uninstall entry in HKCU (no admin required).
/// Visible to the current user in Settings > Apps & Features.
#[cfg(windows)]
fn register_uninstaller(install_dir: &Path, display_dir: &str) -> anyhow::Result<()> {
    use winreg::{enums::*, RegKey};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(
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
    key.set_value("NoModify", &1u32)?;
    key.set_value("NoRepair", &1u32)?;
    Ok(())
}

#[cfg(not(windows))]
fn register_uninstaller(_dir: &Path, _s: &str) -> anyhow::Result<()> { Ok(()) }

#[cfg(windows)]
fn remove_uninstaller_registry() -> anyhow::Result<()> {
    use winreg::{enums::*, RegKey};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.delete_subkey_all(
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\VeloceNetwork"
    )?;
    Ok(())
}

#[cfg(not(windows))]
fn remove_uninstaller_registry() -> anyhow::Result<()> { Ok(()) }

#[cfg(windows)]
fn get_install_dir_from_registry() -> Option<PathBuf> {
    use winreg::{enums::*, RegKey};
    let key: String = RegKey::predef(HKEY_CURRENT_USER)
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

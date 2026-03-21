/*!
VeloceNetwork Launcher — startup menu.

  1  Start Service          systemctl start veloce-core  (Linux)
                            sc start VeloceCoreService   (Windows)
  2  VeloceNet Shell        opens veloce-shell in a new console window
  3  Dashboard              opens veloce-dashboard detached
  4  Shell + Dashboard      both of the above
  5  Stop Service           systemctl stop veloce-core   (Linux)
                            sc stop VeloceCoreService    (Windows)
  Q  Quit
*/

use std::path::PathBuf;
use std::process::Command;
use std::io::{self, Write};

// ── Install-dir resolution ────────────────────────────────────────────────────

#[cfg(windows)]
fn install_dir() -> PathBuf {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
    if let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\VeloceNetwork")
    {
        if let Ok(dir) = key.get_value::<String, _>("InstallLocation") {
            return PathBuf::from(dir);
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files\VeloceNetwork"))
}

#[cfg(unix)]
fn install_dir() -> PathBuf {
    if let Ok(home) = std::env::var("VELOCE_HOME") {
        return PathBuf::from(home);
    }
    // Standard Linux FHS installation path
    PathBuf::from("/opt/veloce")
}

#[cfg(not(any(windows, unix)))]
fn install_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[cfg(windows)]
fn service_status() -> &'static str {
    let out = Command::new("sc")
        .args(["query", "VeloceCoreService"])
        .output()
        .unwrap_or_else(|_| std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: b"unknown".to_vec(),
            stderr: vec![],
        });
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    if text.contains("running") { "RUNNING" }
    else if text.contains("stopped") { "STOPPED" }
    else if text.contains("start_pending") { "STARTING" }
    else if text.contains("stop_pending") { "STOPPING" }
    else { "UNKNOWN" }
}

#[cfg(unix)]
fn service_status() -> &'static str {
    let out = Command::new("systemctl")
        .args(["is-active", "veloce-core"])
        .output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout).trim().to_lowercase();
            if text == "active" { "RUNNING" }
            else if text == "activating" { "STARTING" }
            else if text == "deactivating" { "STOPPING" }
            else if text == "inactive" || text == "failed" { "STOPPED" }
            else { "UNKNOWN" }
        }
        Err(_) => "UNKNOWN",
    }
}

#[cfg(not(any(windows, unix)))]
fn service_status() -> &'static str { "UNKNOWN" }

#[cfg(windows)]
fn service_ctl(verb: &str) -> String {
    let out = Command::new("sc")
        .args([verb, "VeloceCoreService"])
        .output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            text.lines()
                .find(|l| l.contains("STATE") || l.contains("successfully"))
                .unwrap_or(if o.status.success() { "OK" } else { "Command ran" })
                .trim()
                .to_owned()
        }
        Err(e) => format!("Error: {e}"),
    }
}

#[cfg(unix)]
fn service_ctl(verb: &str) -> String {
    // verb is "start" or "stop" — map directly to systemctl
    let systemctl_verb = match verb {
        "start" => "start",
        "stop"  => "stop",
        other   => other,
    };
    let out = Command::new("systemctl")
        .args([systemctl_verb, "veloce-core"])
        .output();
    match out {
        Ok(o) if o.status.success() => "OK".to_owned(),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let msg = stderr.lines().next().unwrap_or("command failed").trim().to_owned();
            if msg.is_empty() { "Failed (no output)".to_owned() } else { msg }
        }
        Err(e) => format!("Error: {e}"),
    }
}

#[cfg(not(any(windows, unix)))]
fn service_ctl(_verb: &str) -> String { "Not supported on this platform".to_owned() }

#[cfg(windows)]
fn open_shell_window(shell_exe: &std::path::Path) {
    use std::os::windows::process::CommandExt;
    let quoted = format!(r#""{}""#, shell_exe.display());
    let _ = Command::new("cmd")
        .args(["/c", "start", "VeloceNet Shell", &quoted])
        .creation_flags(0x00000008 /* DETACHED_PROCESS */)
        .spawn();
}

#[cfg(not(windows))]
fn open_shell_window(shell_exe: &std::path::Path) {
    let _ = Command::new("xterm")
        .args(["-title", "VeloceNet Shell", "-e", &shell_exe.to_string_lossy()])
        .spawn();
}

#[cfg(windows)]
fn launch_detached(exe: &std::path::Path) {
    use std::os::windows::process::CommandExt;
    let _ = Command::new(exe)
        .creation_flags(0x00000008 /* DETACHED_PROCESS */)
        .spawn();
}

#[cfg(not(windows))]
fn launch_detached(exe: &std::path::Path) {
    let _ = Command::new(exe).spawn();
}

fn clear() {
    #[cfg(windows)]
    { let _ = Command::new("cmd").args(["/c", "cls"]).status(); }
    #[cfg(not(windows))]
    { print!("\x1b[2J\x1b[H"); }
}

// ── Menu ─────────────────────────────────────────────────────────────────────

fn print_menu(status: &str) {
    println!();
    println!("  \u{2554}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2557}");
    println!("  \u{2551}        V e l o c e N e t w o r k        \u{2551}");
    println!("  \u{2560}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2563}");
    println!("  \u{2551}                                          \u{2551}");
    println!("  \u{2551}   Service: {status:<32}\u{2551}");
    println!("  \u{2551}                                          \u{2551}");
    println!("  \u{2551}   1 \u{203a}  Start Service                     \u{2551}");
    println!("  \u{2551}   2 \u{203a}  VeloceNet Shell                   \u{2551}");
    println!("  \u{2551}   3 \u{203a}  Dashboard                         \u{2551}");
    println!("  \u{2551}   4 \u{203a}  Shell + Dashboard                 \u{2551}");
    println!("  \u{2551}   5 \u{203a}  Stop Service                      \u{2551}");
    println!("  \u{2551}                                          \u{2551}");
    println!("  \u{2551}   Q \u{203a}  Quit                              \u{2551}");
    println!("  \u{2551}                                          \u{2551}");
    println!("  \u{255a}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}");
    println!();
    print!("  Choose [1-5 / Q]: ");
    let _ = io::stdout().flush();
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let dir = install_dir();

    #[cfg(windows)]
    let shell_bin = dir.join("veloce-shell.exe");
    #[cfg(unix)]
    let shell_bin = dir.join("veloce-shell");
    #[cfg(not(any(windows, unix)))]
    let shell_bin = dir.join("veloce-shell");

    #[cfg(windows)]
    let dash_bin = dir.join("veloce-dashboard.exe");
    #[cfg(unix)]
    let dash_bin = dir.join("veloce-dashboard");
    #[cfg(not(any(windows, unix)))]
    let dash_bin = dir.join("veloce-dashboard");

    loop {
        clear();
        let status = service_status();
        print_menu(status);

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() { break; }

        println!();
        match input.trim().to_uppercase().as_str() {
            "1" => {
                println!("  Starting VeloceCore service…");
                let result = service_ctl("start");
                println!("  {result}");
            }
            "2" => {
                open_shell_window(&shell_bin);
                println!("  \u{2713}  VeloceNet Shell opened.");
            }
            "3" => {
                launch_detached(&dash_bin);
                println!("  \u{2713}  Dashboard launched.");
            }
            "4" => {
                open_shell_window(&shell_bin);
                launch_detached(&dash_bin);
                println!("  \u{2713}  Shell + Dashboard launched.");
            }
            "5" => {
                println!("  Stopping VeloceCore service…");
                let result = service_ctl("stop");
                println!("  {result}");
            }
            "Q" | "QUIT" | "EXIT" => break,
            _ => {
                println!("  Invalid choice.");
            }
        }

        println!();
        println!("  Press Enter to return to the menu…");
        let _ = io::stdin().read_line(&mut String::new());
    }
}

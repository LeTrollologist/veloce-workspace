/*!
VeloceNetwork Launcher — interactive startup menu.

Presents five launch modes when run from a terminal or double-clicked:

  1  Terminal Interface          veloce-run in a new console window
  2  Dashboard (GUI)             veloce-dashboard.exe
  3  Terminal + Dashboard        both of the above
  4  Start Service + Dashboard   starts VeloceCoreService, then opens the Dashboard
  5  Developer CLI               new console with veloce-run --help and a held prompt
*/

use std::path::{Path, PathBuf};
use std::process::Command;
use std::io::{self, Write};

// ── Install-dir resolution ────────────────────────────────────────────────────

fn install_dir() -> PathBuf {
    #[cfg(windows)]
    {
        use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
        if let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\VeloceNetwork")
        {
            if let Ok(dir) = key.get_value::<String, _>("InstallLocation") {
                return PathBuf::from(dir);
            }
        }
    }
    // Fallback: assume binaries live next to this launcher
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files\VeloceNetwork"))
}

// ── Process launchers ─────────────────────────────────────────────────────────

/// Open a new visible console window running `cmd /k <command>`.
/// The window stays open after the command so the user can read output.
#[cfg(windows)]
fn open_terminal(title: &str, command: &str) {
    use std::os::windows::process::CommandExt;
    let _ = Command::new("cmd")
        .args(["/c", "start", title, "cmd", "/k", command])
        .creation_flags(0x00000008 /* DETACHED_PROCESS */)
        .spawn();
}

#[cfg(not(windows))]
fn open_terminal(_title: &str, command: &str) {
    // Best-effort on non-Windows (not a real target, but keeps it compiling)
    let _ = Command::new("sh").args(["-c", &format!("xterm -e '{command}'")]).spawn();
}

/// Launch a GUI binary detached (no console window needed).
#[cfg(windows)]
fn launch_detached(exe: &Path) {
    use std::os::windows::process::CommandExt;
    let _ = Command::new(exe)
        .creation_flags(0x00000008 /* DETACHED_PROCESS */)
        .spawn();
}

#[cfg(not(windows))]
fn launch_detached(exe: &Path) {
    let _ = Command::new(exe).spawn();
}

fn service_ctl(verb: &str) {
    let _ = Command::new("sc").args([verb, "VeloceCoreService"]).output();
}

// ── Menu rendering ────────────────────────────────────────────────────────────

fn clear() {
    #[cfg(windows)]
    { let _ = Command::new("cmd").args(["/c", "cls"]).status(); }
    #[cfg(not(windows))]
    { print!("\x1b[2J\x1b[H"); }
}

fn print_menu() {
    println!();
    println!("  ╔══════════════════════════════════════════╗");
    println!("  ║        V e l o c e N e t w o r k        ║");
    println!("  ╠══════════════════════════════════════════╣");
    println!("  ║                                          ║");
    println!("  ║   1 ›  Terminal Interface                ║");
    println!("  ║   2 ›  Dashboard  (GUI)                  ║");
    println!("  ║   3 ›  Terminal Interface + Dashboard    ║");
    println!("  ║   4 ›  Start Service + Dashboard         ║");
    println!("  ║   5 ›  Developer CLI                     ║");
    println!("  ║                                          ║");
    println!("  ║   Q ›  Quit                              ║");
    println!("  ║                                          ║");
    println!("  ╚══════════════════════════════════════════╝");
    println!();
    print!("  Choose [1-5 / Q]: ");
    let _ = io::stdout().flush();
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let dir = install_dir();
    let run_exe  = dir.join("veloce-run.exe");
    let dash_exe = dir.join("veloce-dashboard.exe");

    // Quote the paths for use in cmd /k strings
    let run_cmd  = format!("\"{}\"", run_exe.display());
    let _dash_cmd = format!("\"{}\"", dash_exe.display());

    loop {
        clear();
        print_menu();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            break;
        }

        match input.trim().to_uppercase().as_str() {
            "1" => {
                // Open a new terminal running veloce-run interactively
                open_terminal("VeloceNetwork — Terminal Interface", &run_cmd);
                println!("\n  ✓  Terminal opened.");
            }
            "2" => {
                launch_detached(&dash_exe);
                println!("\n  ✓  Dashboard launched.");
            }
            "3" => {
                open_terminal("VeloceNetwork — Terminal Interface", &run_cmd);
                launch_detached(&dash_exe);
                println!("\n  ✓  Terminal + Dashboard launched.");
            }
            "4" => {
                println!("\n  Starting VeloceCore service…");
                service_ctl("start");
                // Brief pause so the service has a moment to bind its pipe
                std::thread::sleep(std::time::Duration::from_secs(2));
                launch_detached(&dash_exe);
                println!("  ✓  Service started + Dashboard launched.");
            }
            "5" => {
                // Open a developer console: show veloce-run help, then stay open
                let dev_cmd = format!("{run_cmd} --help & echo. & echo Type a veloce-run command above, or close this window.");
                open_terminal("VeloceNetwork — Developer CLI", &dev_cmd);
                println!("\n  ✓  Developer CLI opened.");
            }
            "Q" | "QUIT" | "EXIT" => break,
            _ => {
                println!("\n  Invalid choice — press Enter to try again.");
                let _ = io::stdin().read_line(&mut String::new());
                continue;
            }
        }

        // Brief pause so the user sees the confirmation, then loop back
        println!("  Press Enter to return to the menu…");
        let _ = io::stdin().read_line(&mut String::new());
    }
}

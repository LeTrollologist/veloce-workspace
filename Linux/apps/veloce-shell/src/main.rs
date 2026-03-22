/*!
VeloceNet Shell — interactive REPL for VeloceNetwork.

Opens a persistent prompt `[VeloceNet]> ` and dispatches short commands
to veloce-run subprocesses so stdout/stderr flow directly to the terminal.

Commands:
  mesh <sub> [args]   veloce-run mesh <sub> [args]
  run <exe> [args]    veloce-run <exe> [args]
  nrpt <sub>          veloce-run nrpt <sub>
  policy <sub>        veloce-run policy <sub>
  service start       sc start VeloceCoreService
  service stop        sc stop  VeloceCoreService
  service status      sc query VeloceCoreService
  dashboard           launch veloce-dashboard.exe detached
  logs                tail %TEMP%\veloce-core.log (Ctrl-C to stop)
  version             print VeloceNetwork version
  help                print this table
  exit / quit         leave the shell
*/

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    // Fallback: same directory as this binary
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files\VeloceNetwork"))
}

// ── Service status ────────────────────────────────────────────────────────────

fn service_status_line() -> String {
    let out = Command::new("sc")
        .args(["query", "VeloceCoreService"])
        .output()
        .unwrap_or_else(|_| std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: b"unknown".to_vec(),
            stderr: vec![],
        });
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    if text.contains("running") {
        "RUNNING".to_string()
    } else if text.contains("stopped") {
        "STOPPED".to_string()
    } else if text.contains("start_pending") {
        "STARTING...".to_string()
    } else if text.contains("stop_pending") {
        "STOPPING...".to_string()
    } else {
        "UNKNOWN (is service installed?)".to_string()
    }
}

// ── Banner ────────────────────────────────────────────────────────────────────

fn print_banner(version: &str) {
    let status = service_status_line();
    println!();
    println!("  ==========================================");
    println!("   VeloceNetwork Shell  v{version}");
    println!("  ==========================================");
    println!();
    println!("   Service: {status}");
    println!();
    println!("   Commands:");
    println!("     mesh status       \u{2014} show peers & traffic");
    println!("     mesh identity     \u{2014} get your join code");
    println!("     mesh join <code>  \u{2014} connect to a peer");
    println!("     nrpt enable       \u{2014} activate .vln DNS (admin)");
    println!("     run <exe>         \u{2014} launch process into mesh");
    println!("     service start     \u{2014} start VeloceCore service");
    println!("     service stop      \u{2014} stop VeloceCore service");
    println!("     logs              \u{2014} tail VeloceCore log (Ctrl-C to stop)");
    println!("     dashboard         \u{2014} open Dashboard");
    println!("     help              \u{2014} full command list");
    println!("     exit              \u{2014} close the shell");
    println!("  ==========================================");
    println!();
}

fn print_help() {
    println!();
    println!("  \u{2500}\u{2500} VeloceNet Shell Commands \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
    println!("   mesh status              Peers, RTT, traffic summary");
    println!("   mesh identity            Print this machine's join code");
    println!("   mesh join <code>         Connect to a remote peer");
    println!("   mesh peers               List connected peers");
    println!("   mesh diagnose            Run self-diagnosis");
    println!("   mesh ping <peer-id>      Show RTT to a peer");
    println!("   mesh leave <peer-id>     Disconnect from a peer");
    println!("   nrpt enable              Install .vln DNS routing (admin)");
    println!("   nrpt disable             Remove .vln DNS routing (admin)");
    println!("   nrpt status              Show NRPT rule state");
    println!("   policy show              Print active policy rules");
    println!("   policy reload            Hot-reload veloce-policy.toml");
    println!("   run <exe> [args]         Launch an executable into the mesh");
    println!("   service start            Start VeloceCoreService");
    println!("   service stop             Stop VeloceCoreService");
    println!("   service status           Show VeloceCoreService state");
    println!("   dashboard                Open the Dashboard GUI");
    println!("   logs                     Tail VeloceCore log (Ctrl-C to stop)");
    println!("   version                  Print version");
    println!("   help                     Show this table");
    println!("   exit / quit              Close the shell");
    println!("  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
    println!();
}

// ── Command dispatch ──────────────────────────────────────────────────────────

fn run_veloce(veloce_run: &Path, args: &[&str]) {
    let _ = Command::new(veloce_run)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
}

fn run_sc(args: &[&str]) {
    let out = Command::new("sc").args(args).output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            // Show a clean subset: STATE line or error
            for line in text.lines() {
                let t = line.trim();
                if !t.is_empty() { println!("  {t}"); }
            }
        }
        Err(e) => eprintln!("  sc error: {e}"),
    }
}

fn launch_dashboard(dir: &Path) {
    let exe = dir.join("veloce-dashboard.exe");
    if !exe.exists() {
        eprintln!("  Dashboard not found at {}", exe.display());
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new(&exe)
            .creation_flags(0x00000008)
            .spawn();
        println!("  Dashboard launched.");
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new(&exe).spawn();
        println!("  Dashboard launched.");
    }
}

fn tail_logs() {
    let log_path = std::env::temp_dir().join("veloce-core.log");
    if !log_path.exists() {
        println!("  Log file not found: {}", log_path.display());
        println!("  (Start VeloceCore to generate logs.)");
        return;
    }

    // Print last 40 lines
    if let Ok(content) = std::fs::read_to_string(&log_path) {
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(40);
        println!("  \u{2500}\u{2500} Last {} lines from {} \u{2500}\u{2500}", lines.len().min(40), log_path.display());
        for line in &lines[start..] {
            println!("  {line}");
        }
        println!("  \u{2500}\u{2500} Live output below (Ctrl-C to stop) \u{2500}\u{2500}");
    }

    // Poll for new lines
    let mut offset = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
    loop {
        std::thread::sleep(std::time::Duration::from_millis(400));
        let Ok(meta) = std::fs::metadata(&log_path) else { continue };
        let len = meta.len();
        if len <= offset { continue; }

        use std::io::{Read, Seek, SeekFrom};
        if let Ok(mut f) = std::fs::File::open(&log_path) {
            if f.seek(SeekFrom::Start(offset)).is_ok() {
                let mut buf = Vec::new();
                if f.read_to_end(&mut buf).is_ok() {
                    offset = len;
                    let text = String::from_utf8_lossy(&buf);
                    for line in text.lines() {
                        if !line.is_empty() { println!("  {line}"); }
                    }
                }
            }
        }
    }
    // Note: user breaks with Ctrl-C; the process exits
}

// ── REPL ─────────────────────────────────────────────────────────────────────

fn main() {
    let dir = install_dir();
    let veloce_run = dir.join("veloce-run.exe");
    let version = env!("CARGO_PKG_VERSION");

    // Set window title on Windows
    #[cfg(windows)]
    { print!("\x1b]0;VeloceNet Shell\x07"); }

    print_banner(version);

    loop {
        print!("  [VeloceNet]> ");
        let _ = io::stdout().flush();

        let mut line = String::new();
        match io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => break,  // EOF / error → exit
            Ok(_) => {}
        }

        let line = line.trim();
        if line.is_empty() { continue; }

        // Tokenize
        let tokens: Vec<&str> = line.splitn(10, ' ').filter(|s| !s.is_empty()).collect();
        let cmd = tokens[0].to_lowercase();
        let rest = &tokens[1..];

        println!();
        match cmd.as_str() {
            "exit" | "quit" => {
                println!("  Goodbye.");
                break;
            }

            "help" => print_help(),

            "version" => println!("  VeloceNetwork v{version}"),

            "mesh" => {
                if rest.is_empty() {
                    println!("  Usage: mesh <status|identity|join|peers|diagnose|ping|leave>");
                } else {
                    let mut args = vec!["mesh"];
                    args.extend_from_slice(rest);
                    run_veloce(&veloce_run, &args);
                }
            }

            "nrpt" => {
                if rest.is_empty() {
                    println!("  Usage: nrpt <enable|disable|status>");
                } else {
                    let mut args = vec!["nrpt"];
                    args.extend_from_slice(rest);
                    run_veloce(&veloce_run, &args);
                }
            }

            "policy" => {
                if rest.is_empty() {
                    println!("  Usage: policy <show|reload>");
                } else {
                    let mut args = vec!["policy"];
                    args.extend_from_slice(rest);
                    run_veloce(&veloce_run, &args);
                }
            }

            "run" => {
                if rest.is_empty() {
                    println!("  Usage: run <executable> [args...]");
                } else {
                    run_veloce(&veloce_run, rest);
                }
            }

            "service" => {
                let sub = rest.first().map(|s| s.to_lowercase()).unwrap_or_default();
                match sub.as_str() {
                    "start"  => { run_sc(&["start",  "VeloceCoreService"]); }
                    "stop"   => { run_sc(&["stop",   "VeloceCoreService"]); }
                    "status" => { run_sc(&["query",  "VeloceCoreService"]); }
                    _ => println!("  Usage: service <start|stop|status>"),
                }
            }

            "dashboard" => launch_dashboard(&dir),

            "logs" => tail_logs(),

            _ => println!("  Unknown command '{cmd}'. Type 'help' for available commands."),
        }

        println!();
    }
}

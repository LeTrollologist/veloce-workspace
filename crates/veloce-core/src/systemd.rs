//! systemd integration for VeloceCore on Linux.
//!
//! Provides:
//! - `notify_ready()`: sends `READY=1` via `sd_notify` (Type=notify units)
//! - `install()`: writes the systemd unit file and reloads the daemon
//! - `uninstall()`: removes the unit file and reloads
//! - `start()` / `stop()`: delegates to `systemctl`
//! - `status()`: checks `systemctl is-active`

#![cfg(unix)]

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

const UNIT_NAME: &str  = "veloce-core.service";
const UNIT_PATH: &str  = "/etc/systemd/system/veloce-core.service";

/// Send `READY=1` to systemd via the `NOTIFY_SOCKET` mechanism.
/// No-op if not running under systemd (returns silently).
pub fn notify_ready() {
    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]);
}

/// Install the systemd unit file for VeloceCore.
///
/// Writes `/etc/systemd/system/veloce-core.service` and runs
/// `systemctl daemon-reload`.  Requires root.
pub fn install(exe_path: &Path) -> Result<()> {
    let exe = exe_path.to_string_lossy();
    let unit = format!(r#"[Unit]
Description=VeloceCore Runtime
Documentation=https://github.com/LeTrollologist/VeloceNetwork-Linux
After=network.target

[Service]
Type=notify
ExecStart={exe} run
ExecStop={exe} stop
Restart=on-failure
RestartSec=5
# Allow VeloceCore to create sub-cgroups under its own slice
Delegate=yes
# systemd creates /run/veloce/ (RuntimeDirectory) and /var/lib/veloce/ (StateDirectory)
RuntimeDirectory=veloce
StateDirectory=veloce
# Log to the journal
StandardOutput=journal
StandardError=journal
SyslogIdentifier=veloce-core

[Install]
WantedBy=multi-user.target
"#);

    std::fs::write(UNIT_PATH, &unit)
        .with_context(|| format!("write unit file {UNIT_PATH}"))?;

    daemon_reload()?;

    tracing::info!("installed systemd unit: {UNIT_PATH}");
    Ok(())
}

/// Remove the systemd unit file and reload the daemon.
pub fn uninstall() -> Result<()> {
    if Path::new(UNIT_PATH).exists() {
        std::fs::remove_file(UNIT_PATH)
            .with_context(|| format!("remove unit file {UNIT_PATH}"))?;
        daemon_reload()?;
        tracing::info!("removed systemd unit: {UNIT_PATH}");
    }
    Ok(())
}

/// Start the VeloceCore service via systemctl.
pub fn start() -> Result<()> {
    systemctl(&["start", UNIT_NAME])
}

/// Stop the VeloceCore service via systemctl.
pub fn stop() -> Result<()> {
    systemctl(&["stop", UNIT_NAME])
}

/// Returns `true` if the VeloceCore service is currently active.
pub fn status() -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", UNIT_NAME])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn daemon_reload() -> Result<()> {
    systemctl(&["daemon-reload"])
}

fn systemctl(args: &[&str]) -> Result<()> {
    let out = Command::new("systemctl")
        .args(args)
        .output()
        .context("run systemctl")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("systemctl {:?} failed: {}", args, stderr.trim());
    }
    Ok(())
}

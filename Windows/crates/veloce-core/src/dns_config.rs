//! Linux DNS configuration for the `.vln` private namespace.
//!
//! Configures split-DNS so that `*.vln` queries are forwarded to
//! VeloceCore's embedded DNS server on `127.0.0.1:5354`.
//!
//! Strategy (in order of preference):
//! 1. systemd-resolved drop-in (`/etc/systemd/resolved.conf.d/veloce-vln.conf`)
//! 2. dnsmasq drop-in (`/etc/dnsmasq.d/veloce-vln.conf`)
//!
//! Requires root to write to /etc/.

#![cfg(unix)]

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// The address:port where VeloceCore's DNS server listens.
pub const VLN_DNS_ADDR: &str = "127.0.0.1:5354";

const RESOLVED_DROPIN_DIR:  &str = "/etc/systemd/resolved.conf.d";
const RESOLVED_DROPIN_PATH: &str = "/etc/systemd/resolved.conf.d/veloce-vln.conf";
const DNSMASQ_DROPIN_PATH:  &str = "/etc/dnsmasq.d/veloce-vln.conf";

/// Install `.vln` DNS routing.
///
/// Tries systemd-resolved first; falls back to dnsmasq if resolved is not active.
pub fn install(dns_addr: &str) -> Result<()> {
    if systemd_resolved_active() {
        install_resolved(dns_addr)?;
    } else if dnsmasq_active() {
        install_dnsmasq(dns_addr)?;
    } else {
        tracing::warn!(
            "neither systemd-resolved nor dnsmasq is active — \
             .vln DNS routing not configured. Start one and re-run \
             `veloce-core dns enable`."
        );
    }
    Ok(())
}

/// Remove `.vln` DNS routing.
pub fn uninstall() -> Result<()> {
    let mut changed = false;
    if Path::new(RESOLVED_DROPIN_PATH).exists() {
        std::fs::remove_file(RESOLVED_DROPIN_PATH)
            .context("remove systemd-resolved drop-in")?;
        restart_resolved();
        changed = true;
    }
    if Path::new(DNSMASQ_DROPIN_PATH).exists() {
        std::fs::remove_file(DNSMASQ_DROPIN_PATH)
            .context("remove dnsmasq drop-in")?;
        restart_dnsmasq();
        changed = true;
    }
    if changed {
        tracing::info!(".vln DNS routing removed");
    }
    Ok(())
}

/// Returns `true` if a DNS routing config is currently installed.
pub fn is_installed() -> bool {
    Path::new(RESOLVED_DROPIN_PATH).exists() || Path::new(DNSMASQ_DROPIN_PATH).exists()
}

/// Returns the currently-installed DNS address or `None`.
pub fn installed_addr() -> Option<String> {
    if Path::new(RESOLVED_DROPIN_PATH).exists() {
        // Parse the DNS= line from the drop-in
        if let Ok(content) = std::fs::read_to_string(RESOLVED_DROPIN_PATH) {
            for line in content.lines() {
                if let Some(addr) = line.strip_prefix("DNS=") {
                    return Some(addr.trim().to_owned());
                }
            }
        }
    }
    if Path::new(DNSMASQ_DROPIN_PATH).exists() {
        return Some(dns_addr_from_dnsmasq());
    }
    None
}

/// Restart the system DNS cache (resolved or dnsmasq).
/// Called `restart_dnscache` for API compat with the Windows nrpt.rs.
#[allow(dead_code)]
pub fn restart_dnscache() {
    if systemd_resolved_active() {
        restart_resolved();
    } else if dnsmasq_active() {
        restart_dnsmasq();
    }
}

// ── Internals ────────────────────────────────────────────────────────────────

fn install_resolved(dns_addr: &str) -> Result<()> {
    std::fs::create_dir_all(RESOLVED_DROPIN_DIR)
        .context("create resolved.conf.d")?;
    let content = format!(
        "[Resolve]\n# VeloceNetwork .vln private namespace\nDNS={dns_addr}\nDomains=~vln\n"
    );
    std::fs::write(RESOLVED_DROPIN_PATH, &content)
        .with_context(|| format!("write {RESOLVED_DROPIN_PATH}"))?;
    restart_resolved();
    tracing::info!(".vln DNS routing installed via systemd-resolved ({RESOLVED_DROPIN_PATH})");
    Ok(())
}

fn install_dnsmasq(dns_addr: &str) -> Result<()> {
    // dns_addr is "127.0.0.1:5354" — dnsmasq needs host and port separately
    let (host, port) = dns_addr.rsplit_once(':').unwrap_or((dns_addr, "53"));
    let content = format!("# VeloceNetwork .vln private namespace\nserver=/vln/{host}#{port}\n");
    std::fs::write(DNSMASQ_DROPIN_PATH, &content)
        .with_context(|| format!("write {DNSMASQ_DROPIN_PATH}"))?;
    restart_dnsmasq();
    tracing::info!(".vln DNS routing installed via dnsmasq ({DNSMASQ_DROPIN_PATH})");
    Ok(())
}

fn systemd_resolved_active() -> bool {
    is_service_active("systemd-resolved")
}

fn dnsmasq_active() -> bool {
    is_service_active("dnsmasq")
}

fn is_service_active(name: &str) -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn restart_resolved() {
    let _ = Command::new("systemctl")
        .args(["restart", "systemd-resolved"])
        .status();
}

fn restart_dnsmasq() {
    let _ = Command::new("systemctl")
        .args(["restart", "dnsmasq"])
        .status();
}

fn dns_addr_from_dnsmasq() -> String {
    if let Ok(content) = std::fs::read_to_string(DNSMASQ_DROPIN_PATH) {
        for line in content.lines() {
            // "server=/vln/127.0.0.1#5354"
            if let Some(rest) = line.strip_prefix("server=/vln/") {
                return rest.replace('#', ":").to_owned();
            }
        }
    }
    VLN_DNS_ADDR.to_owned()
}

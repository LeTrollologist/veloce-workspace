/*!
Windows Name Resolution Policy Table (NRPT) management.

Writes/removes a single NRPT rule that routes all `*.vln` DNS queries to the
VeloceNet DNS server running on `127.0.0.1:5354`.

NRPT is supported on Windows 10 1903+ and Windows 11.  The rule is written
under:
  HKLM\SYSTEM\CurrentControlSet\Services\Dnscache\Parameters\DnsPolicyConfig\VeloceNetwork-VLN

After installing or removing the rule the Windows DNS-cache service (Dnscache)
must be restarted for the change to take effect.  `install()` / `uninstall()`
do NOT restart Dnscache automatically — callers that want immediate effect
should call `restart_dnscache()`.
*/

#[cfg(windows)]
use anyhow::{Context, Result};

#[cfg(windows)]
const BASE:      &str = r"SYSTEM\CurrentControlSet\Services\Dnscache\Parameters\DnsPolicyConfig";
#[cfg(windows)]
const RULE_NAME: &str = "VeloceNetwork-VLN";

/// Default DNS address advertised in the NRPT rule.
#[cfg(windows)]
pub const VLN_DNS_ADDR: &str = "127.0.0.1:5354";

// ── Public API (Windows) ───────────────────────────────────────────────────

/// Write the `.vln` NRPT rule to the registry.
///
/// Requires Administrator privilege.  Idempotent: safe to call when the rule
/// already exists (it will be overwritten with the current address).
#[cfg(windows)]
pub fn install(dns_addr: &str) -> Result<()> {
    use winreg::{enums::*, RegKey};

    let hklm        = RegKey::predef(HKEY_LOCAL_MACHINE);
    let policy_base = hklm
        .open_subkey_with_flags(BASE, KEY_CREATE_SUB_KEY)
        .context("open DnsPolicyConfig key (run as Administrator)")?;

    let (rule, _) = policy_base
        .create_subkey(RULE_NAME)
        .context("create NRPT rule key")?;

    rule.set_value("Version",            &2u32)                    .context("Version")?;
    rule.set_value("ConfigOptions",      &8u32)                    .context("ConfigOptions")?;
    // REG_MULTI_SZ — winreg serialises Vec<String> as MULTI_SZ
    rule.set_value("Name",               &vec![".vln".to_owned()]) .context("Name")?;
    rule.set_value("GenericDNSServers",  &dns_addr)                .context("GenericDNSServers")?;
    rule.set_value("Comment",            &"VeloceNetwork .vln private namespace") .context("Comment")?;

    tracing::info!(dns_addr, "NRPT rule installed: *.vln → {dns_addr}");
    Ok(())
}

/// Remove the `.vln` NRPT rule from the registry.
///
/// Requires Administrator privilege.  No-op if the rule does not exist.
#[cfg(windows)]
pub fn uninstall() -> Result<()> {
    use winreg::{enums::*, RegKey};

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let base  = match hklm.open_subkey_with_flags(BASE, KEY_WRITE) {
        Ok(k)  => k,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).context("open DnsPolicyConfig key"),
    };

    match base.delete_subkey_all(RULE_NAME) {
        Ok(())                                              => tracing::info!("NRPT rule removed"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}   // already gone
        Err(e) => return Err(e).context("delete NRPT rule key"),
    }
    Ok(())
}

/// Returns `true` when the `.vln` NRPT rule key exists in the registry.
#[cfg(windows)]
pub fn is_installed() -> bool {
    use winreg::{enums::*, RegKey};
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(format!("{BASE}\\{RULE_NAME}"))
        .is_ok()
}

/// Returns the DNS address stored in the current NRPT rule, if any.
#[cfg(windows)]
pub fn installed_addr() -> Option<String> {
    use winreg::{enums::*, RegKey};
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(format!("{BASE}\\{RULE_NAME}"))
        .ok()?
        .get_value("GenericDNSServers")
        .ok()
}

/// Restart the Windows DNS-cache service so NRPT rule changes take effect
/// immediately.  Requires Administrator privilege.
///
/// This issues a `net stop Dnscache && net start Dnscache` — on machines
/// where the DNS Client service is set to "Manual", the start call may fail
/// with a non-fatal error (queries still route correctly on next boot).
#[cfg(windows)]
pub fn restart_dnscache() -> Result<()> {
    use std::process::Command;

    let stop = Command::new("net").args(["stop", "Dnscache"]).output()
        .context("net stop Dnscache")?;
    if !stop.status.success() {
        tracing::warn!("net stop Dnscache: {}", String::from_utf8_lossy(&stop.stderr).trim());
    }

    let start = Command::new("net").args(["start", "Dnscache"]).output()
        .context("net start Dnscache")?;
    if !start.status.success() {
        tracing::warn!("net start Dnscache: {}", String::from_utf8_lossy(&start.stderr).trim());
    }

    tracing::info!("Dnscache restarted");
    Ok(())
}

// ── Linux / Unix DNS configuration (re-exports from dns_config) ──────────────

#[cfg(unix)]
pub use crate::dns_config::{
    install,
    uninstall,
    is_installed,
    installed_addr,
    restart_dnscache,
    VLN_DNS_ADDR,
};

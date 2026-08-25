/*!
macOS (Darwin) Native System Integration (v4.3).

Provides macOS-specific platform capabilities:
- Native `/etc/resolver/vln` resolver configuration generation for zero-root *.vln DNS routing.
- `launchd` User Agent LaunchDaemon plist generation (`com.velocenetwork.core.plist`).
- Standard macOS application support socket resolution.
*/

use std::path::{Path, PathBuf};

pub struct DarwinSystem;

impl DarwinSystem {
    /// Return the standard macOS application support directory for VeloceNetwork.
    pub fn app_support_dir() -> PathBuf {
        if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("VeloceNetwork")
        } else {
            PathBuf::from("/tmp/VeloceNetwork")
        }
    }

    /// Return standard Darwin Unix domain socket path.
    pub fn default_socket_path() -> PathBuf {
        Self::app_support_dir().join("veloce.sock")
    }

    /// Generate the macOS `/etc/resolver/vln` configuration string.
    /// On macOS, placing a file at `/etc/resolver/<domain>` directs all queries for that domain to the specified nameserver.
    pub fn generate_resolver_config(dns_port: u16) -> String {
        format!(
            "# VeloceNetwork macOS Resolver for *.vln\nnameserver 127.0.0.1\nport {}\nsearch_order 1\n",
            dns_port
        )
    }

    /// Generate the `launchd` user agent plist content.
    pub fn generate_launchd_plist(bin_path: &Path) -> String {
        let bin_str = bin_path.to_string_lossy();
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.velocenetwork.core</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>--console</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/veloce-core.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/veloce-core.err</string>
</dict>
</plist>
"#,
            bin_str
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_darwin_resolver_and_launchd_generation() {
        let resolver = DarwinSystem::generate_resolver_config(5354);
        assert!(resolver.contains("nameserver 127.0.0.1"));
        assert!(resolver.contains("port 5354"));

        let plist = DarwinSystem::generate_launchd_plist(Path::new("/usr/local/bin/veloce-core"));
        assert!(plist.contains("com.velocenetwork.core"));
        assert!(plist.contains("/usr/local/bin/veloce-core"));
    }
}

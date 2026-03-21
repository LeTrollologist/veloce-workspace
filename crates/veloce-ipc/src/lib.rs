/*!
# veloce-ipc — Wire Protocol

Every component in the VeloceNetwork stack compiles this crate.
It defines the exact bytes that flow over the named pipe, ensuring Core
and every sideloaded app speak the same language across versions.

## Frame layout

```text
Offset  Size  Field
──────  ────  ────────────────────────────────────────────────────────
0       4     Magic: 0x56454C43 ("VELC")
4       1     Version byte (0x01)
5       1     MessageType discriminant (u8)
6       2     Flags bitfield (u16 LE)
8       4     PayloadLen (u32 LE, max 4 MiB)
── header: 12 bytes ─────────────────────────────────────────────────
12      N     Payload: bincode-encoded Envelope { correlation_id, body }
```
*/

pub mod codec;
pub mod error;
pub mod message;

pub use codec::Codec;
pub use error::IpcError;
pub use message::Envelope;

// ── WIRE CONSTANTS ─────────────────────────────────────────────────────────────

/// Frame magic: ASCII "VELC" in big-endian.
pub const MAGIC: u32 = 0x56454C43;

/// Current protocol version byte.
pub const VERSION: u8 = 0x01;

/// Fixed header length in bytes.
pub const HEADER_LEN: usize = 12;

/// Maximum allowed payload size (4 MiB).
pub const MAX_PAYLOAD: u32 = 4 * 1024 * 1024;

// ── PIPE / ENDPOINT NAMES (Windows) ─────────────────────────────────────────

/// Named pipe used by clients to reach VeloceCore (Windows).
pub const PIPE_NAME: &str = r"\\.\pipe\VeloceCore";

/// Named pipe used for VeloceNet control messages (Windows).
pub const PIPE_NET: &str = r"\\.\pipe\VeloceNet";

// ── SOCKET / ENDPOINT PATHS (Linux / Unix) ───────────────────────────────────

/// Unix domain socket used by clients to reach VeloceCore (privileged daemon).
#[cfg(unix)]
pub const SOCKET_PATH: &str = "/run/veloce/core.sock";

/// Unix domain socket for VeloceNet control messages (privileged daemon).
#[cfg(unix)]
pub const SOCKET_NET: &str = "/run/veloce/net.sock";

/// Returns the Unix socket path for user-session connections.
/// Uses `$XDG_RUNTIME_DIR/veloce/core.sock` when available,
/// otherwise falls back to `/run/user/{uid}/veloce/core.sock`.
#[cfg(unix)]
pub fn socket_path_user() -> std::path::PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return std::path::PathBuf::from(xdg).join("veloce").join("core.sock");
    }
    // Fallback: /run/user/{uid}/veloce/core.sock
    let uid = unsafe { libc::getuid() };
    std::path::PathBuf::from(format!("/run/user/{uid}/veloce/core.sock"))
}

// ── SESSION KEY ────────────────────────────────────────────────────────────────

/// Returns the path where VeloceCore writes the per-session PSK.
///
/// * Windows: `%LOCALAPPDATA%\VeloceCore\session.key`
/// * Other:   `$TMPDIR/veloce-session.key`
///
/// The file contains 64 lowercase hex chars (32 raw bytes).  It is
/// recreated each time VeloceCore starts, invalidating all prior sessions.
pub fn psk_path() -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| {
            std::env::var("USERPROFILE")
                .map(|p| format!("{p}\\AppData\\Local"))
                .unwrap_or_else(|_| "C:\\Users\\Default\\AppData\\Local".into())
        });
        std::path::PathBuf::from(base).join("VeloceCore").join("session.key")
    }
    #[cfg(not(windows))]
    {
        std::env::temp_dir().join("veloce-session.key")
    }
}

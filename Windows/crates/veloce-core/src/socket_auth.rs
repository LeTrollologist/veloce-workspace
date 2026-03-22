//! Unix socket peer authentication via SO_PEERCRED.
//!
//! Replaces `pipe_security.rs` (Windows) for the Linux port.
//! Verifies that the connecting process runs as the same UID as VeloceCore,
//! mirroring the Named Pipe ACL gate on Windows.

#![cfg(unix)]

use anyhow::{bail, Context, Result};
use std::os::unix::io::AsRawFd;
use tokio::net::UnixStream;

/// Returns the effective UID of the running process (the server's UID).
pub fn server_uid() -> u32 {
    unsafe { libc::geteuid() }
}

/// Returns the string UID of the server (used for PSK gate analogous to SYSTEM check).
pub fn server_uid_str() -> String {
    server_uid().to_string()
}

/// Validate that the peer connecting on `stream` runs as the same user as
/// the server, and return `(exe_path, peer_pid)`.
///
/// Uses `SO_PEERCRED` to obtain the kernel-verified `ucred { pid, uid, gid }`.
/// The exe path is resolved via `/proc/{pid}/exe`.
pub fn assert_client_is_owner(stream: &UnixStream) -> Result<(String, u32)> {
    let cred = peer_credentials(stream.as_raw_fd())
        .context("getsockopt SO_PEERCRED")?;

    let expected_uid = server_uid();
    if cred.uid != expected_uid {
        bail!(
            "peer UID {} does not match server UID {} — rejecting connection",
            cred.uid,
            expected_uid
        );
    }

    // Read the executable path from /proc/{pid}/exe — same trust anchor as
    // QueryFullProcessImageNameW on Windows.
    let exe_path = std::fs::read_link(format!("/proc/{}/exe", cred.pid))
        .with_context(|| format!("readlink /proc/{}/exe", cred.pid))?
        .to_string_lossy()
        .into_owned();

    Ok((exe_path, cred.pid))
}

// ── Low-level SO_PEERCRED ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct PeerCred {
    pub pid: u32,
    pub uid: u32,
    pub gid: u32,
}

fn peer_credentials(fd: std::os::unix::io::RawFd) -> Result<PeerCred> {
    // struct ucred { pid_t pid; uid_t uid; gid_t gid; }
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;

    let ret = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };

    if ret != 0 {
        bail!("getsockopt SO_PEERCRED failed: {}", std::io::Error::last_os_error());
    }

    Ok(PeerCred {
        pid: cred.pid as u32,
        uid: cred.uid as u32,
        gid: cred.gid as u32,
    })
}

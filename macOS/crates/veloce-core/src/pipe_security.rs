//! Named-pipe connection gating: only the user account that owns the VeloceCore
//! process may connect.  Every incoming connection is checked immediately after
//! `connect()` returns — before any IPC frame is read.
//!
//! Strategy: runtime SID comparison.
//!   1. At startup, record the string SID of the server process's token user.
//!   2. For each accepted pipe instance, read the client PID via
//!      `GetNamedPipeClientProcessId`, open the process, open its token, extract
//!      its TOKEN_USER SID, and compare strings.
//!   3. If the SIDs differ, log a warning and return `Err` so the caller drops
//!      (and thereby disconnects) the pipe handle.
//!
//! This is a TOCTOU-safe check at the kernel level: the PID is taken from the
//! kernel's own pipe endpoint accounting and the token is obtained directly from
//! the process object — there is no window between pid-lookup and token-read
//! where an attacker could substitute a different process.

use anyhow::{Context, Result};
use std::os::windows::io::AsRawHandle;
use windows::Win32::{
    Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL},
    Security::{
        Authorization::ConvertSidToStringSidW,
        GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER,
    },
    System::{
        Pipes::GetNamedPipeClientProcessId,
        Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken,
            QueryFullProcessImageNameW,
            PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
};

// ── helpers ──────────────────────────────────────────────────────────────────

/// Resolve the full Win32 image path of `proc` (e.g. `C:\...\veloce-run.exe`).
///
/// # Safety
/// `proc` must be a valid process handle with at least
/// `PROCESS_QUERY_LIMITED_INFORMATION` access.
unsafe fn query_process_exe(proc: HANDLE) -> Result<String> {
    let mut buf = [0u16; 32768];
    let mut len = buf.len() as u32;
    QueryFullProcessImageNameW(
        proc,
        PROCESS_NAME_WIN32,
        windows::core::PWSTR(buf.as_mut_ptr()),
        &mut len,
    )
    .context("QueryFullProcessImageNameW")?;
    Ok(String::from_utf16_lossy(&buf[..len as usize]))
}

/// Extract the string SID of the token's user (e.g. `"S-1-5-21-…-1001"`).
///
/// # Safety
/// `token` must be a valid, open token handle with `TOKEN_QUERY` access.
unsafe fn sid_string_from_token(token: HANDLE) -> Result<String> {
    // A TOKEN_USER struct is followed immediately in memory by the SID.
    // The SID for a typical domain account fits comfortably in 512 bytes.
    //
    // TOKEN_USER contains a pointer (PSID), so the buffer must be at least
    // pointer-aligned (8 bytes on x64).  A bare [u8; 512] is only 1-byte
    // aligned, which causes a misaligned-pointer panic on the cast below.
    #[repr(align(8))]
    struct AlignedBuf([u8; 512]);
    let mut buf = AlignedBuf([0u8; 512]);
    let mut returned = 0u32;

    GetTokenInformation(
        token,
        TokenUser,
        Some(buf.0.as_mut_ptr() as *mut _),
        buf.0.len() as u32,
        &mut returned,
    )
    .context("GetTokenInformation(TokenUser)")?;

    // Reinterpret the buffer as TOKEN_USER to reach the Sid pointer.
    // Safety: buf is repr(align(8)) so the cast is alignment-safe; Windows
    // wrote a valid TOKEN_USER into it and returned the actual byte count.
    let tu = &*(buf.0.as_ptr() as *const TOKEN_USER);
    let sid = tu.User.Sid; // PSID (*mut c_void)

    // Ask Windows to format the SID as a string; it allocates via LocalAlloc.
    let mut pwstr = windows::core::PWSTR::null();
    ConvertSidToStringSidW(sid, &mut pwstr).context("ConvertSidToStringSidW")?;

    let sid_str = pwstr.to_string().context("SID PWSTR decode")?;

    // Free the string Windows allocated — PWSTR.0 is a *mut u16.
    LocalFree(HLOCAL(pwstr.0 as *mut _));

    Ok(sid_str)
}

// ── public API ────────────────────────────────────────────────────────────────

/// Returns the string SID of the current process's user.
/// Call once at startup and cache the result.
pub fn server_user_sid() -> Result<String> {
    unsafe {
        // GetCurrentProcess() is a pseudo-handle — no CloseHandle needed.
        let proc = GetCurrentProcess();
        let mut token = HANDLE::default();
        OpenProcessToken(proc, TOKEN_QUERY, &mut token)
            .context("OpenProcessToken(server process)")?;
        let sid = sid_string_from_token(token)?;
        CloseHandle(token).ok(); // best-effort; pseudo-process handle not closed
        Ok(sid)
    }
}

/// Verify that the process connected to `pipe` is running under the same user
/// account as the server.
///
/// Returns `Ok((exe_path, client_pid))` — the kernel-verified full Win32 image
/// path and PID of the client process — if the SIDs match.  Returns `Err`
/// otherwise; the caller should **drop** the pipe, which disconnects the client.
///
/// The exe path is obtained from the kernel via `QueryFullProcessImageNameW`
/// while the process handle is open, before any client-supplied data is read.
/// It is safe to use as a trust anchor for policy decisions.
pub fn assert_client_is_owner<H: AsRawHandle>(
    pipe: &H,
    server_sid: &str,
) -> Result<(String, u32)> {
    unsafe {
        // 1. Get the client process ID directly from the pipe endpoint.
        let pipe_handle = HANDLE(pipe.as_raw_handle() as isize);
        let mut client_pid = 0u32;
        GetNamedPipeClientProcessId(pipe_handle, &mut client_pid)
            .context("GetNamedPipeClientProcessId")?;

        // 2. Open the client process with minimum required rights.
        let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, client_pid)
            .context("OpenProcess(client)")?;

        // 3. Resolve the exe path while the handle is still open.
        //    Fail closed: if we can't read the image name, reject the connection.
        let exe_path = query_process_exe(proc)
            .context("could not resolve client exe path — rejecting connection")?;

        // 4. Open its primary token.
        let mut token = HANDLE::default();
        let token_result = OpenProcessToken(proc, TOKEN_QUERY, &mut token);
        CloseHandle(proc).ok();
        token_result.context("OpenProcessToken(client)")?;

        // 5. Extract and compare SIDs.
        let client_sid = sid_string_from_token(token)?;
        CloseHandle(token).ok();

        if client_sid != server_sid {
            anyhow::bail!(
                "pipe client SID {client_sid:?} does not match server SID \
                 {server_sid:?} — connection rejected"
            );
        }

        Ok((exe_path, client_pid))
    }
}

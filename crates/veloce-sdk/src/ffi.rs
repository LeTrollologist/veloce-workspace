/*!
# C-ABI FFI surface

Allows non-Rust applications (C, C++, Python via ctypes, Node via node-ffi)
to integrate VeloceCore without writing Rust.

## Usage from C

```c
#include "veloce_sdk.h"

VeloceHandle* h = veloce_connect("MyApp", "1.0.0");
if (!h) { /* error */ }

VeloceNodeResult result;
veloce_spawn_node(h, "worker", "C:\\worker.exe", NULL, 0, &result);

veloce_register_host(h, "worker.vln", result.node_id, 8080, 0);

veloce_disconnect(h);
```
*/

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint};
use std::sync::Mutex;
use std::time::Duration;

use crate::{client::NodeEventStream, VeloceClient};
use veloce_ipc::message::{Capability, NodeEvent};

/// C representation of a node event.
///
/// `event_type`:  0=Started  1=Exited  2=MemThreshold  3=CpuThrottled  4=LifetimeExpired
///
/// `param1`:  pid (Started) | exit_code (Exited) | current_mb (Mem) | current_pct (Cpu)
/// `param2`:  limit_mb (MemThreshold only)
#[repr(C)]
pub struct CVeloceNodeEvent {
    /// Node UUID, null-terminated ASCII (36 + NUL).
    pub node_id:    [c_char; 37],
    pub event_type: u8,
    pub param1:     u64,
    pub param2:     u64,
}

/// Opaque handle type.
pub struct VeloceHandle {
    inner:  Mutex<Option<tokio::runtime::Runtime>>,
    client: Mutex<Option<VeloceClient>>,
    /// Broadcast event stream — pre-subscribed at connect for `veloce_poll_event`.
    events: Mutex<Option<NodeEventStream>>,
}

// ── FFI EXPORTS ───────────────────────────────────────────────────────────────

/// Connect to VeloceCore and return an opaque handle.
/// Returns NULL on failure (check logs).
/// The handle MUST be freed with `veloce_disconnect`.
#[no_mangle]
pub extern "C" fn veloce_connect(
    app_name:    *const c_char,
    sdk_version: *const c_char,
) -> *mut VeloceHandle {
    let app_name = unsafe {
        match CStr::from_ptr(app_name).to_str() {
            Ok(s) => s.to_owned(),
            Err(_) => return std::ptr::null_mut(),
        }
    };
    let sdk_version = unsafe {
        match CStr::from_ptr(sdk_version).to_str() {
            Ok(s) => s.to_owned(),
            Err(_) => return std::ptr::null_mut(),
        }
    };

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all().build()
    {
        Ok(r)  => r,
        Err(_) => return std::ptr::null_mut(),
    };

    let client = rt.block_on(VeloceClient::connect(
        &app_name,
        &sdk_version,
        vec![
            Capability::SpawnNodes,
            Capability::KillNodes,
            Capability::RegistryRead,
            Capability::RegistryWrite,
            Capability::NetRegister,
            Capability::NetResolve,
        ],
    ));

    match client {
        Ok(c) => {
            let events = c.subscribe_all_events();
            let handle = Box::new(VeloceHandle {
                inner:  Mutex::new(Some(rt)),
                client: Mutex::new(Some(c)),
                events: Mutex::new(Some(events)),
            });
            Box::into_raw(handle)
        }
        Err(e) => {
            eprintln!("veloce_connect failed: {e:#}");
            std::ptr::null_mut()
        }
    }
}

/// Disconnect and free the handle.
#[no_mangle]
pub extern "C" fn veloce_disconnect(handle: *mut VeloceHandle) {
    if handle.is_null() { return; }
    let _h = unsafe { Box::from_raw(handle) };
    // Drop releases runtime and client
}

/// Ping VeloceCore. Returns 0 on success, -1 on failure.
#[no_mangle]
pub extern "C" fn veloce_ping(handle: *mut VeloceHandle) -> c_int {
    with_client(handle, |rt, client| {
        rt.block_on(client.ping())
    })
}

/// C representation of a spawned node result.
#[repr(C)]
pub struct CVeloceNode {
    /// UUID as a 37-byte null-terminated string.
    pub node_id:   [c_char; 37],
    pub pid:       c_uint,
    /// Named pipe path as a null-terminated string (max 260 chars).
    pub node_pipe: [c_char; 261],
}

/// Spawn a node. Returns 0 on success, fills `out`. Returns -1 on error.
#[no_mangle]
pub extern "C" fn veloce_spawn_node(
    handle:    *mut VeloceHandle,
    app_name:  *const c_char,
    executable: *const c_char,
    out:       *mut CVeloceNode,
) -> c_int {
    if out.is_null() { return -1; }
    let app_name = unsafe { CStr::from_ptr(app_name).to_string_lossy().into_owned() };
    let executable = unsafe { CStr::from_ptr(executable).to_string_lossy().into_owned() };

    let result = with_client_result(handle, |rt, client| {
        rt.block_on(client.spawn_node(&app_name, &executable, &[]))
    });

    match result {
        Ok(spawned) => {
            let node_id_str = CString::new(spawned.node_id.to_string()).unwrap();
            let pipe_str    = CString::new(spawned.node_pipe).unwrap();
            unsafe {
                let o = &mut *out;
                let id_bytes = node_id_str.as_bytes_with_nul();
                let id_len   = id_bytes.len().min(37);
                o.node_id[..id_len].copy_from_slice(std::slice::from_raw_parts(id_bytes.as_ptr() as *const c_char, id_len));
                let pipe_bytes = pipe_str.as_bytes_with_nul();
                let pipe_len = pipe_bytes.len().min(261);
                o.node_pipe[..pipe_len].copy_from_slice(std::slice::from_raw_parts(pipe_bytes.as_ptr() as *const c_char, pipe_len));
                o.pid = spawned.pid;
            }
            0
        }
        Err(e) => {
            eprintln!("veloce_spawn_node error: {e:#}");
            -1
        }
    }
}

/// Register a *.vln hostname. Returns 0 on success.
#[no_mangle]
pub extern "C" fn veloce_register_host(
    handle:     *mut VeloceHandle,
    hostname:   *const c_char,
    node_id_str: *const c_char,
    local_port: u16,
    ttl_secs:   u64,
) -> c_int {
    let hostname = unsafe { CStr::from_ptr(hostname).to_string_lossy().into_owned() };
    let node_id  = unsafe {
        let s = CStr::from_ptr(node_id_str).to_string_lossy();
        match uuid::Uuid::parse_str(&s) {
            Ok(id) => id,
            Err(_) => return -1,
        }
    };

    with_client(handle, |rt, client| {
        rt.block_on(client.register_host(&hostname, node_id, local_port, ttl_secs))
    })
}

/// Unregister a *.vln hostname. Returns 0 on success.
#[no_mangle]
pub extern "C" fn veloce_unregister_host(
    handle:   *mut VeloceHandle,
    hostname: *const c_char,
) -> c_int {
    let hostname = unsafe { CStr::from_ptr(hostname).to_string_lossy().into_owned() };
    with_client(handle, |rt, client| {
        rt.block_on(async {
            use veloce_ipc::message::Body;
            // Send unregister directly
            let _resp = client.raw_request(Body::NetUnregisterHost { hostname: hostname.clone() }).await?;
            Ok(())
        })
    })
}

/// Kill a node by UUID string. Returns 0 on success.
#[no_mangle]
pub extern "C" fn veloce_kill_node(
    handle:      *mut VeloceHandle,
    node_id_str: *const c_char,
) -> c_int {
    let node_id = unsafe {
        let s = CStr::from_ptr(node_id_str).to_string_lossy();
        match uuid::Uuid::parse_str(&s) {
            Ok(id) => id,
            Err(_) => return -1,
        }
    };
    with_client(handle, |rt, client| {
        rt.block_on(client.kill_node(node_id)).map(|_| ())
    })
}

/// Resolve a *.vln hostname.
/// Returns 0 on success (out_buf filled), 1 if not found, -1 on error.
#[no_mangle]
pub extern "C" fn veloce_resolve_host(
    handle:   *mut VeloceHandle,
    hostname: *const c_char,
    out_buf:  *mut c_char,
    out_len:  c_int,
) -> c_int {
    if out_buf.is_null() || out_len <= 0 { return -1; }
    let hostname = unsafe { CStr::from_ptr(hostname).to_string_lossy().into_owned() };

    let result = with_client_result(handle, |rt, client| {
        rt.block_on(client.resolve_host(&hostname))
    });

    match result {
        Ok(Some(addr)) => {
            let cs = match CString::new(addr) {
                Ok(s)  => s,
                Err(_) => return -1,
            };
            let bytes = cs.as_bytes_with_nul();
            let copy  = bytes.len().min(out_len as usize);
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, out_buf, copy);
            }
            0
        }
        Ok(None)  => 1,
        Err(e) => { eprintln!("veloce_resolve_host: {e:#}"); -1 }
    }
}

/// Read from the VeloceCore registry.
/// Returns 0 on success, 1 if key not found, -1 on error.
/// `out_len` is both input (buffer capacity) and output (bytes written).
#[no_mangle]
pub extern "C" fn veloce_registry_get(
    handle:  *mut VeloceHandle,
    key:     *const c_char,
    out_buf: *mut u8,
    out_len: *mut c_int,
) -> c_int {
    if out_buf.is_null() || out_len.is_null() { return -1; }
    let key = unsafe { CStr::from_ptr(key).to_string_lossy().into_owned() };

    let result = with_client_result(handle, |rt, client| {
        rt.block_on(client.registry_get(&key))
    });

    match result {
        Ok(Some(value)) => {
            let cap = unsafe { *out_len } as usize;
            let copy = value.len().min(cap);
            unsafe {
                std::ptr::copy_nonoverlapping(value.as_ptr(), out_buf, copy);
                *out_len = copy as c_int;
            }
            0
        }
        Ok(None)  => 1,
        Err(e) => { eprintln!("veloce_registry_get: {e:#}"); -1 }
    }
}

/// Write to the VeloceCore registry. Returns 0 on success.
#[no_mangle]
pub extern "C" fn veloce_registry_set(
    handle:    *mut VeloceHandle,
    key:       *const c_char,
    value:     *const u8,
    value_len: c_int,
) -> c_int {
    if value.is_null() || value_len < 0 { return -1; }
    let key = unsafe { CStr::from_ptr(key).to_string_lossy().into_owned() };
    let val = unsafe { std::slice::from_raw_parts(value, value_len as usize).to_vec() };

    with_client(handle, |rt, client| {
        rt.block_on(client.registry_set(&key, val))
    })
}

/// Returns the compile-time SDK version string.  Static — do not free.
#[no_mangle]
pub extern "C" fn veloce_sdk_version() -> *const c_char {
    static VERSION: std::sync::OnceLock<CString> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        CString::new(env!("CARGO_PKG_VERSION")).expect("version string")
    }).as_ptr()
}

/// Poll for the next node event.
///
/// `timeout_ms`: max wait in milliseconds (0 = non-blocking check).
///
/// Returns:
///  * `0`  — event written to `*out`
///  * `1`  — timed out (no event yet)
///  * `-1` — error (null handle/buf, stream closed, or internal error)
#[no_mangle]
pub extern "C" fn veloce_poll_event(
    handle:     *mut VeloceHandle,
    timeout_ms: u64,
    out:        *mut CVeloceNodeEvent,
) -> c_int {
    if handle.is_null() || out.is_null() { return -1; }
    let h = unsafe { &*handle };

    let rt_guard = h.inner.lock().unwrap();
    let rt = match rt_guard.as_ref() {
        Some(r) => r,
        None    => return -1,
    };

    let mut ev_guard = h.events.lock().unwrap();
    let stream: &mut NodeEventStream = match ev_guard.as_mut() {
        Some(s) => s,
        None    => return -1,
    };

    let timeout = Duration::from_millis(timeout_ms);
    match rt.block_on(tokio::time::timeout(timeout, stream.next())) {
        Err(_elapsed)    => 1,   // timed out
        Ok(None)         => -1,  // stream closed
        Ok(Some(msg)) => {
            let node_id_str = CString::new(msg.node_id.to_string()).unwrap();
            let (event_type, param1, param2) = match msg.event {
                NodeEvent::Started { pid }                                => (0u8, pid as u64, 0u64),
                NodeEvent::Exited { exit_code }                           => (1u8, exit_code as u64, 0u64),
                NodeEvent::MemThresholdExceeded { current_mb, limit_mb } => (2u8, current_mb, limit_mb),
                NodeEvent::CpuThrottled { current_pct }                   => (3u8, current_pct as u64, 0u64),
                NodeEvent::LifetimeExpired                                => (4u8, 0u64, 0u64),
            };
            unsafe {
                let o = &mut *out;
                let id_bytes = node_id_str.as_bytes_with_nul();
                let id_len   = id_bytes.len().min(37);
                o.node_id[..id_len].copy_from_slice(
                    std::slice::from_raw_parts(id_bytes.as_ptr() as *const c_char, id_len),
                );
                o.event_type = event_type;
                o.param1     = param1;
                o.param2     = param2;
            }
            0
        }
    }
}

// ── HELPERS ───────────────────────────────────────────────────────────────────

fn with_client(
    handle: *mut VeloceHandle,
    f: impl FnOnce(&tokio::runtime::Runtime, &mut VeloceClient) -> anyhow::Result<()>,
) -> c_int {
    match with_client_result(handle, f) {
        Ok(())  => 0,
        Err(e) => { eprintln!("veloce FFI error: {e:#}"); -1 }
    }
}

fn with_client_result<T>(
    handle: *mut VeloceHandle,
    f: impl FnOnce(&tokio::runtime::Runtime, &mut VeloceClient) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    if handle.is_null() { anyhow::bail!("null handle"); }
    let h = unsafe { &*handle };
    let rt_guard     = h.inner.lock().unwrap();
    let rt           = rt_guard.as_ref().ok_or_else(|| anyhow::anyhow!("runtime gone"))?;
    let mut c_guard  = h.client.lock().unwrap();
    let client       = c_guard.as_mut().ok_or_else(|| anyhow::anyhow!("client gone"))?;
    f(rt, client)
}
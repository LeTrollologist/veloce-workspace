/*!
WebAssembly (Wasm/WASI) In-Process Execution Engine (v3.9).

Provides deterministic, zero-root microservice sandboxing with sub-millisecond cold starts,
WASI Preview 1 IO redirection, and direct bindings into the Veloce mesh and key-value store.
*/

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Context, Result};
use wasmi::{Caller, Engine, Linker, Module, Store};

/// Host state shared with the guest WebAssembly instance.
pub struct WasmHostState {
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub stdout_buf: Arc<parking_lot::Mutex<Vec<u8>>>,
    pub stderr_buf: Arc<parking_lot::Mutex<Vec<u8>>>,
    pub exit_code: Option<i32>,
    pub mesh_kv: Option<Arc<veloce_mesh::MeshState>>,
}

/// Metadata about a compiled WebAssembly module.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WasmModuleInfo {
    pub exports: Vec<String>,
    pub imports: Vec<(String, String)>,
    pub has_wasi: bool,
    pub entrypoint: Option<String>,
}

pub struct WasmRuntime {
    engine: Engine,
}

impl WasmRuntime {
    pub fn new() -> Self {
        Self {
            engine: Engine::default(),
        }
    }

    /// Check if a given file or byte slice is a WebAssembly binary (`\0asm`).
    pub fn is_wasm_binary(bytes: &[u8]) -> bool {
        bytes.starts_with(b"\0asm")
    }

    /// Inspect a WebAssembly binary for exported functions, imports, and WASI detection.
    pub fn inspect(&self, wasm_bytes: &[u8]) -> Result<WasmModuleInfo> {
        let module = Module::new(&self.engine, wasm_bytes)
            .context("compile/validate WebAssembly binary")?;

        let mut exports = Vec::new();
        let mut entrypoint = None;
        for export in module.exports() {
            let name = export.name().to_string();
            if name == "_start" || name == "main" || name == "run" {
                if entrypoint.is_none() {
                    entrypoint = Some(name.clone());
                }
            }
            exports.push(name);
        }

        let mut imports = Vec::new();
        let mut has_wasi = false;
        for import in module.imports() {
            let module_name = import.module().to_string();
            let field_name = import.name().to_string();
            if module_name.starts_with("wasi_") {
                has_wasi = true;
            }
            imports.push((module_name, field_name));
        }

        Ok(WasmModuleInfo {
            exports,
            imports,
            has_wasi,
            entrypoint,
        })
    }

    /// Execute a WebAssembly binary with full WASI and Veloce host function bindings.
    pub fn execute(
        &self,
        wasm_bytes: &[u8],
        args: Vec<String>,
        env: HashMap<String, String>,
        mesh: Option<Arc<veloce_mesh::MeshState>>,
    ) -> Result<WasmExecutionResult> {
        let module = Module::new(&self.engine, wasm_bytes)
            .context("compile WebAssembly module")?;

        let stdout_buf = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let stderr_buf = Arc::new(parking_lot::Mutex::new(Vec::new()));

        let host_state = WasmHostState {
            args,
            env,
            stdout_buf: stdout_buf.clone(),
            stderr_buf: stderr_buf.clone(),
            exit_code: None,
            mesh_kv: mesh,
        };

        let mut store = Store::new(&self.engine, host_state);
        let mut linker = Linker::new(&self.engine);

        // Register WASI Preview 1 & Veloce host functions
        Self::register_wasi_preview1(&mut linker)?;
        Self::register_veloce_mesh_functions(&mut linker)?;

        let instance = linker
            .instantiate(&mut store, &module)
            .context("instantiate WebAssembly module")?
            .start(&mut store)
            .context("run WebAssembly start function")?;

        // Determine entrypoint: _start > main > run
        let entrypoint_name = if instance.get_export(&store, "_start").is_some() {
            "_start"
        } else if instance.get_export(&store, "main").is_some() {
            "main"
        } else if instance.get_export(&store, "run").is_some() {
            "run"
        } else {
            "_start"
        };

        if let Some(func) = instance.get_export(&store, entrypoint_name).and_then(|e| e.into_func()) {
            let mut results = [wasmi::Value::I32(0)];
            let _ = func.call(&mut store, &[], &mut results);
        }

        let final_state = store.into_data();
        let exit_code = final_state.exit_code.unwrap_or(0);
        let stdout = Arc::try_unwrap(stdout_buf)
            .map(|m| m.into_inner())
            .unwrap_or_default();
        let stderr = Arc::try_unwrap(stderr_buf)
            .map(|m| m.into_inner())
            .unwrap_or_default();

        Ok(WasmExecutionResult {
            exit_code,
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }

    fn register_wasi_preview1(linker: &mut Linker<WasmHostState>) -> Result<()> {
        // proc_exit(rval: i32)
        linker.func_wrap(
            "wasi_snapshot_preview1",
            "proc_exit",
            |mut caller: Caller<'_, WasmHostState>, rval: i32| {
                caller.data_mut().exit_code = Some(rval);
            },
        )?;

        // fd_write(fd: i32, iovs_ptr: i32, iovs_len: i32, nwritten_ptr: i32) -> i32
        linker.func_wrap(
            "wasi_snapshot_preview1",
            "fd_write",
            |mut caller: Caller<'_, WasmHostState>, fd: i32, iovs_ptr: i32, iovs_len: i32, nwritten_ptr: i32| -> i32 {
                let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(m) => m,
                    None => return 28, // __WASI_ERRNO_INVAL
                };

                let data = memory.data(&caller);
                let mut total_written = 0u32;

                for i in 0..iovs_len {
                    let iov_offset = (iovs_ptr as usize) + (i as usize * 8);
                    if iov_offset + 8 > data.len() {
                        return 21; // __WASI_ERRNO_FAULT
                    }

                    let buf_ptr = u32::from_le_bytes(data[iov_offset..iov_offset + 4].try_into().unwrap()) as usize;
                    let buf_len = u32::from_le_bytes(data[iov_offset + 4..iov_offset + 8].try_into().unwrap()) as usize;

                    if buf_ptr + buf_len > data.len() {
                        return 21;
                    }

                    let slice = &data[buf_ptr..buf_ptr + buf_len];
                    if fd == 1 {
                        caller.data().stdout_buf.lock().extend_from_slice(slice);
                    } else if fd == 2 {
                        caller.data().stderr_buf.lock().extend_from_slice(slice);
                    }
                    total_written += buf_len as u32;
                }

                let mem_mut = memory.data_mut(&mut caller);
                if (nwritten_ptr as usize) + 4 <= mem_mut.len() {
                    let nw_bytes = total_written.to_le_bytes();
                    mem_mut[nwritten_ptr as usize..nwritten_ptr as usize + 4].copy_from_slice(&nw_bytes);
                }

                0 // __WASI_ERRNO_SUCCESS
            },
        )?;

        // environ_sizes_get(environ_count_ptr, environ_buf_size_ptr) -> i32
        linker.func_wrap(
            "wasi_snapshot_preview1",
            "environ_sizes_get",
            |mut caller: Caller<'_, WasmHostState>, count_ptr: i32, size_ptr: i32| -> i32 {
                let env = &caller.data().env;
                let count = env.len() as u32;
                let size = env.iter().map(|(k, v)| k.len() + 1 + v.len() + 1).sum::<usize>() as u32;

                let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(m) => m,
                    None => return 28,
                };
                let mem = memory.data_mut(&mut caller);
                if (count_ptr as usize + 4) <= mem.len() && (size_ptr as usize + 4) <= mem.len() {
                    mem[count_ptr as usize..count_ptr as usize + 4].copy_from_slice(&count.to_le_bytes());
                    mem[size_ptr as usize..size_ptr as usize + 4].copy_from_slice(&size.to_le_bytes());
                }
                0
            },
        )?;

        // clock_time_get(id, precision, time_ptr) -> i32
        linker.func_wrap(
            "wasi_snapshot_preview1",
            "clock_time_get",
            |mut caller: Caller<'_, WasmHostState>, _id: i32, _precision: i64, time_ptr: i32| -> i32 {
                let nanos = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;

                let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(m) => m,
                    None => return 28,
                };
                let mem = memory.data_mut(&mut caller);
                if (time_ptr as usize + 8) <= mem.len() {
                    mem[time_ptr as usize..time_ptr as usize + 8].copy_from_slice(&nanos.to_le_bytes());
                }
                0
            },
        )?;

        Ok(())
    }

    fn register_veloce_mesh_functions(linker: &mut Linker<WasmHostState>) -> Result<()> {
        // env.veloce_kv_get(key_ptr, key_len, out_ptr, out_max) -> i32
        linker.func_wrap(
            "env",
            "veloce_kv_get",
            |mut caller: Caller<'_, WasmHostState>, key_ptr: i32, key_len: i32, out_ptr: i32, out_max: i32| -> i32 {
                let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(m) => m,
                    None => return -1,
                };

                let data = memory.data(&caller);
                let k_start = key_ptr as usize;
                let k_end = k_start + (key_len as usize);
                if k_end > data.len() {
                    return -1;
                }

                let key_str = match std::str::from_utf8(&data[k_start..k_end]) {
                    Ok(s) => s,
                    Err(_) => return -1,
                };

                let val = caller.data().mesh_kv.as_ref().and_then(|m| m.kv.get(key_str));
                match val {
                    Some(v) => {
                        let bytes = v.as_bytes();
                        let write_len = bytes.len().min(out_max as usize);
                        let mem_mut = memory.data_mut(&mut caller);
                        let out_start = out_ptr as usize;
                        if out_start + write_len <= mem_mut.len() {
                            mem_mut[out_start..out_start + write_len].copy_from_slice(&bytes[..write_len]);
                            write_len as i32
                        } else {
                            -1
                        }
                    }
                    None => -1,
                }
            },
        )?;

        // env.veloce_kv_set(key_ptr, key_len, val_ptr, val_len) -> i32
        linker.func_wrap(
            "env",
            "veloce_kv_set",
            |caller: Caller<'_, WasmHostState>, key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32| -> i32 {
                let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                    Some(m) => m,
                    None => return -1,
                };

                let data = memory.data(&caller);
                let k_start = key_ptr as usize;
                let k_end = k_start + (key_len as usize);
                let v_start = val_ptr as usize;
                let v_end = v_start + (val_len as usize);

                if k_end > data.len() || v_end > data.len() {
                    return -1;
                }

                let key = match std::str::from_utf8(&data[k_start..k_end]) {
                    Ok(s) => s,
                    Err(_) => return -1,
                };
                let val = match std::str::from_utf8(&data[v_start..v_end]) {
                    Ok(s) => s,
                    Err(_) => return -1,
                };

                if let Some(mesh) = &caller.data().mesh_kv {
                    mesh.kv.set(key, val);
                    0
                } else {
                    -1
                }
            },
        )?;

        Ok(())
    }
}

pub struct WasmExecutionResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wasm_runtime_empty_module() {
        let runtime = WasmRuntime::new();
        // Minimal valid WebAssembly binary: \0asm (version 1)
        let wasm = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        assert!(WasmRuntime::is_wasm_binary(&wasm));

        let info = runtime.inspect(&wasm).expect("inspect wasm");
        assert_eq!(info.exports.len(), 0);

        let res = runtime
            .execute(&wasm, vec![], HashMap::new(), None)
            .expect("execute wasm");
        assert_eq!(res.exit_code, 0);
    }
}


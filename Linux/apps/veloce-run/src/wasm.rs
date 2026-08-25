/*!
WebAssembly (Wasm/WASI) CLI runtime & inspection (v3.9).
*/

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use wasmi::{Caller, Engine, Linker, Module, Store};

#[derive(Args, Debug)]
pub struct WasmArgs {
    #[command(subcommand)]
    pub action: WasmAction,
}

#[derive(Subcommand, Debug)]
pub enum WasmAction {
    /// Execute a WebAssembly (Wasm/WASI) binary directly in userspace.
    Run(WasmRunArgs),
    /// Inspect a WebAssembly binary for exported functions, imports, and WASI support.
    Inspect {
        /// Path to the .wasm binary
        file: PathBuf,
    },
}

#[derive(Args, Debug)]
pub struct WasmRunArgs {
    /// Path to the .wasm file to run
    pub file: PathBuf,

    /// Arguments forwarded to the WebAssembly guest
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,

    /// Extra environment variables (KEY=VALUE); may be repeated
    #[arg(short = 'e', long = "env", value_name = "KEY=VALUE")]
    pub extra_env: Vec<String>,
}

struct WasmHostState {
    args: Vec<String>,
    env: HashMap<String, String>,
    stdout_buf: Arc<Mutex<Vec<u8>>>,
    stderr_buf: Arc<Mutex<Vec<u8>>>,
    exit_code: Option<i32>,
}

pub fn run_wasm(action: WasmAction) -> Result<()> {
    match action {
        WasmAction::Run(args) => handle_wasm_run(args),
        WasmAction::Inspect { file } => handle_wasm_inspect(file),
    }
}

pub fn handle_wasm_run(args: WasmRunArgs) -> Result<()> {
    let wasm_bytes = std::fs::read(&args.file)
        .with_context(|| format!("read WASM file '{}'", args.file.display()))?;

    if !wasm_bytes.starts_with(b"\0asm") {
        bail!("file '{}' is not a valid WebAssembly binary (missing '\\0asm' magic header)", args.file.display());
    }

    let mut env_map = HashMap::new();
    for env_str in args.extra_env {
        if let Some((k, v)) = env_str.split_once('=') {
            env_map.insert(k.to_string(), v.to_string());
        }
    }

    let engine = Engine::default();
    let module = Module::new(&engine, &wasm_bytes[..])
        .context("compile WebAssembly module")?;

    let stdout_buf = Arc::new(Mutex::new(Vec::new()));
    let stderr_buf = Arc::new(Mutex::new(Vec::new()));

    let host_state = WasmHostState {
        args: args.args,
        env: env_map,
        stdout_buf: stdout_buf.clone(),
        stderr_buf: stderr_buf.clone(),
        exit_code: None,
    };

    let mut store = Store::new(&engine, host_state);
    let mut linker = Linker::new(&engine);

    // Register WASI preview 1
    register_wasi_preview1(&mut linker)?;

    let instance = linker
        .instantiate(&mut store, &module)
        .context("instantiate WebAssembly module")?
        .start(&mut store)
        .context("run WebAssembly start function")?;

    let entrypoint = if instance.get_export(&store, "_start").is_some() {
        "_start"
    } else if instance.get_export(&store, "main").is_some() {
        "main"
    } else if instance.get_export(&store, "run").is_some() {
        "run"
    } else {
        "_start"
    };

    if let Some(func) = instance.get_export(&store, entrypoint).and_then(|e| e.into_func()) {
        let mut results = [wasmi::Value::I32(0)];
        let _ = func.call(&mut store, &[], &mut results);
    }

    let final_state = store.into_data();
    let exit_code = final_state.exit_code.unwrap_or(0);

    let stdout = Arc::try_unwrap(stdout_buf).map(|m| m.into_inner().unwrap_or_default()).unwrap_or_default();
    let stderr = Arc::try_unwrap(stderr_buf).map(|m| m.into_inner().unwrap_or_default()).unwrap_or_default();

    if !stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&stdout));
    }
    if !stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&stderr));
    }

    if exit_code != 0 {
        std::process::exit(exit_code);
    }

    Ok(())
}

pub fn handle_wasm_inspect(file: PathBuf) -> Result<()> {
    let wasm_bytes = std::fs::read(&file)
        .with_context(|| format!("read WASM file '{}'", file.display()))?;

    if !wasm_bytes.starts_with(b"\0asm") {
        bail!("file '{}' is not a valid WebAssembly binary", file.display());
    }

    let engine = Engine::default();
    let module = Module::new(&engine, &wasm_bytes[..])
        .context("validate WebAssembly binary")?;

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

    println!("========================================================");
    println!(" WebAssembly Binary Inspector: {}", file.display());
    println!("========================================================");
    println!("  Format:     WebAssembly 1.0 (WASM)");
    println!("  WASI:       {}", if has_wasi { "Yes (WASI Snapshot Preview 1)" } else { "No (Pure Compute)" });
    println!("  Entrypoint: {}", entrypoint.as_deref().unwrap_or("(none)"));
    println!();

    println!("Exported Symbols ({}):", exports.len());
    if exports.is_empty() {
        println!("  (none)");
    } else {
        for exp in &exports {
            println!("  - {}", exp);
        }
    }
    println!();

    println!("Imported Modules ({}):", imports.len());
    if imports.is_empty() {
        println!("  (none)");
    } else {
        for (module, field) in &imports {
            println!("  - {}::{}", module, field);
        }
    }
    println!("========================================================");

    Ok(())
}

fn register_wasi_preview1(linker: &mut Linker<WasmHostState>) -> Result<()> {
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "proc_exit",
        |mut caller: Caller<'_, WasmHostState>, rval: i32| {
            caller.data_mut().exit_code = Some(rval);
        },
    )?;

    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_write",
        |mut caller: Caller<'_, WasmHostState>, fd: i32, iovs_ptr: i32, iovs_len: i32, nwritten_ptr: i32| -> i32 {
            let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                Some(m) => m,
                None => return 28,
            };

            let data = memory.data(&caller);
            let mut total_written = 0u32;

            for i in 0..iovs_len {
                let iov_offset = (iovs_ptr as usize) + (i as usize * 8);
                if iov_offset + 8 > data.len() {
                    return 21;
                }

                let buf_ptr = u32::from_le_bytes(data[iov_offset..iov_offset + 4].try_into().unwrap()) as usize;
                let buf_len = u32::from_le_bytes(data[iov_offset + 4..iov_offset + 8].try_into().unwrap()) as usize;

                if buf_ptr + buf_len > data.len() {
                    return 21;
                }

                let slice = &data[buf_ptr..buf_ptr + buf_len];
                if fd == 1 {
                    if let Ok(mut lock) = caller.data().stdout_buf.lock() {
                        lock.extend_from_slice(slice);
                    }
                } else if fd == 2 {
                    if let Ok(mut lock) = caller.data().stderr_buf.lock() {
                        lock.extend_from_slice(slice);
                    }
                }
                total_written += buf_len as u32;
            }

            let mem_mut = memory.data_mut(&mut caller);
            if (nwritten_ptr as usize) + 4 <= mem_mut.len() {
                let nw_bytes = total_written.to_le_bytes();
                mem_mut[nwritten_ptr as usize..nwritten_ptr as usize + 4].copy_from_slice(&nw_bytes);
            }

            0
        },
    )?;

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

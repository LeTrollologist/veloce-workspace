# 🚀 VeloceNetwork — Getting Started & Installation Guide

Welcome to **VeloceNetwork** — a lightweight, zero-kernel, zero-root userspace service mesh and execution runtime.

This guide walks you through installing, building, running, and testing live demos across **Windows**, **Linux**, and **Android** (compile-your-own-build).

---

## 📋 Table of Contents

1. [Windows Installation & Quickstart](#-windows-installation--quickstart)
2. [Linux Installation & Quickstart](#-linux-installation--quickstart)
3. [Android: Compile Your Own Build (DIY)](#-android-compile-your-own-build-diy)
4. [5 Live Demos & Tutorials](#-5-live-demos--tutorials)
   - [Demo 1: Multi-Service Mesh & Userspace DNS](#demo-1-multi-service-mesh--userspace-dns)
   - [Demo 2: Zero-Trust Team Share (VM3 Codes)](#demo-2-zero-trust-team-share-vm3-codes)
   - [Demo 3: Live OpenTelemetry Waterfall Tracing](#demo-3-live-opentelemetry-waterfall-tracing)
   - [Demo 4: Kubernetes Telepresence & Interception](#demo-4-kubernetes-telepresence--interception)
   - [Demo 5: Sandboxed WebAssembly (Wasm/WASI)](#demo-5-sandboxed-webassembly-wasmwasi)

---

## 🪟 Windows Installation & Quickstart

### Prerequisites
- Windows 10 (1809+) or Windows 11 / Server 2019+
- Standard (unprivileged) user account or Administrator
- [Rust Toolchain](https://rustup.rs/) (stable MSVC or GNU) for compiling from source

### Option A: Build from Source
```powershell
# Clone the monorepo
git clone https://github.com/LeTrollologist/veloce-workspace.git
cd veloce-workspace/Windows

# Build all workspace binaries
cargo build --release -j 2

# Binaries are in Windows/target/release/
#   - veloce-core.exe       (Background Daemon & Service Mesh Engine)
#   - veloce-run.exe        (CLI Tool & Orchestrator)
#   - veloce-launcher.exe   (Desktop System Tray App)
#   - veloce-dashboard.exe  (Svelte 5 / Canvas 2D GUI)
```

### Option B: Running the Daemon
```powershell
# Start VeloceCore in interactive unprivileged console mode (no admin required)
.\target\release\veloce-core.exe --console
```

---

## 🐧 Linux Installation & Quickstart

### Prerequisites
- Any modern 64-bit Linux distribution (Ubuntu 20.04+, Debian 11+, Fedora, Arch, Alpine)
- `systemd` (user or system) or custom init system
- Rust stable (`x86_64-unknown-linux-gnu`)

### Building from Source
```bash
# Clone the repository
git clone https://github.com/LeTrollologist/veloce-workspace.git
cd veloce-workspace/Linux

# Build all workspace binaries
cargo build --release -j 2

# Binaries are in Linux/target/release/
#   - veloce-core           (Daemon)
#   - veloce-run            (CLI)
```

### Running as a systemd User Service
```bash
# Copy binary to local bin
mkdir -p ~/.local/bin
cp target/release/veloce-core target/release/veloce-run ~/.local/bin/

# Start VeloceCore daemon directly
~/.local/bin/veloce-core --console
```

---

## 📱 Android: Compile Your Own Build (DIY)

VeloceNetwork provides a native Rust JNI mobile core (`veloce-mobile`) that embeds the entire P2P WireGuard-grade Noise_IK mesh, userspace DNS, SOCKS5 proxy, and replicated KV store into a single unprivileged shared library (`libveloce_mobile.so`). 

On Android, it routes `*.vln` hostnames seamlessly using the standard Android `VpnService` API **without requiring root or custom ROMs**.

### 1. Prerequisites & Toolchain Setup
```bash
# Install Android NDK (r25+ recommended) and Rust Android targets
rustup target add aarch64-linux-android      # 64-bit ARM (modern devices)
rustup target add armv7-linux-androideabi   # 32-bit ARM (older devices)
rustup target add x86_64-linux-android      # 64-bit x86 (emulators)
rustup target add i686-linux-android        # 32-bit x86 (emulators)

# Install cargo-ndk helper
cargo install cargo-ndk
```

### 2. Compiling the Native Shared Libraries
```bash
cd veloce-workspace/Linux   # or Windows

# Set your Android NDK home path
export ANDROID_NDK_HOME=/path/to/android-sdk/ndk/25.2.9519653

# Build for all Android CPU architectures
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -t x86 \
  -o ./android-app/app/src/main/jniLibs \
  build -p veloce-mobile --release
```

### 3. Integrating with Java/Kotlin in Android Studio
The native library exports standard JNI bindings in `com.velocenetwork.mobile.VeloceNative`:

```kotlin
package com.velocenetwork.mobile

object VeloceNative {
    init {
        System.loadLibrary("veloce_mobile")
    }

    external fun startNode(dataDir: String, joinCode: String?, meshPort: Int): Boolean
    external fun stopNode(): Boolean
    external fun isRunning(): Boolean
    external fun getNodeStatus(): String
    external fun getPeers(): String
    external fun getKv(key: String): String?
    external fun setKv(key: String, value: String): Boolean
}
```

### 4. Running the Android VpnService
In your Android Companion App, start the VPN service to intercept `*.vln` DNS queries and proxy them into the local SOCKS5 port (`127.0.0.1:1055`):

```kotlin
class VeloceVpnService : VpnService() {
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // Start native Veloce engine
        val dataDir = applicationContext.filesDir.absolutePath
        val joinCode = intent?.getStringExtra("JOIN_CODE")
        VeloceNative.startNode(dataDir, joinCode, 10550)

        // Establish zero-root VPN interface
        val builder = Builder()
            .addAddress("10.55.0.2", 24)
            .addDnsServer("127.0.0.1")
            .addRoute("10.55.0.0", 16)
        val vpnInterface = builder.establish()
        return START_STICKY
    }
}
```

Build and install to your phone via Android Studio or `./gradlew installDebug`!

---

## 🧪 5 Live Demos & Tutorials

### Demo 1: Multi-Service Mesh & Userspace DNS
Launch two microservices that communicate transparently over `.vln` domain names:

```bash
# Terminal 1: Launch background backend database
veloce-run --name db --hostname database.vln --port 5432 --mem 256 -- python -m http.server 5432

# Terminal 2: Launch web frontend that connects to database.vln
veloce-run --name web --hostname web.vln --port 8080 -- curl -s http://database.vln:5432

# Verify active services in the mesh
veloce-run ps
```

---

### Demo 2: Zero-Trust Team Share (VM3 Codes)
Share a local port securely with a teammate without opening firewall ports, forwarding NAT, or running public tunnels:

```bash
# Developer A: Share local dev server on port 3000
veloce-run share 3000 --name api-service --ttl 2h
# Output: Share Code: vshare://vm3-eyJhbGciOi...

# Developer B: Connect to Developer A's share instantly
veloce-run join vshare://vm3-eyJhbGciOi...

# Developer B can now access the service directly:
curl http://api-service.shared.vln
```

---

### Demo 3: Live OpenTelemetry Waterfall Tracing
Inspect microservice latencies and distributed request traces in terminal ASCII:

```bash
# List recent distributed traces across the mesh
veloce-run trace list

# Inspect an end-to-end trace waterfall
veloce-run trace inspect 4bf92f3577b34da6a3ce929d0e0e4736

# Export traces directly to Jaeger or Grafana Tempo
veloce-run trace export --endpoint http://localhost:4318/v1/traces --enable
```

---

### Demo 4: Kubernetes Telepresence & Interception
Route live traffic from a staging cloud cluster into your local IDE debugger:

```bash
# Connect local environment to remote Kubernetes staging namespace
veloce-run bridge connect --peer 10.96.0.10:7474 --namespace staging

# Shadow live cluster requests with "X-Debug: true" to local port 9000
veloce-run bridge intercept --service order-service --header "X-Debug: true" --target 9000

# View active telepresence bridges
veloce-run bridge list
```

---

### Demo 5: Sandboxed WebAssembly (Wasm/WASI)
Execute OS-agnostic WebAssembly modules inside a strict zero-root memory sandbox:

```bash
# Inspect WebAssembly exports and imports
veloce-run wasm inspect ./service.wasm

# Run the Wasm module with environment variables
veloce-run wasm run ./service.wasm --env LOG_LEVEL=debug
```

---

## 📖 Complete CLI Reference
For full documentation of all CLI subcommands and flags, refer to [`COMMANDS.md`](COMMANDS.md).

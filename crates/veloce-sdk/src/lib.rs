/*!
# veloce-sdk

Rust async client and C FFI layer for sideloaded applications.

## Rust usage

```rust,no_run
use veloce_sdk::VeloceClient;
use veloce_ipc::message::Capability;

// let mut client = VeloceClient::connect(...).await?;
```

## C usage (via `veloce_sdk.dll`)

```c
#include "veloce_sdk.h"
VeloceHandle* h = veloce_connect("MyApp", "1.0.0");
veloce_ping(h);
veloce_disconnect(h);
```
*/

pub mod client;
pub mod ffi;

pub use client::VeloceClient;

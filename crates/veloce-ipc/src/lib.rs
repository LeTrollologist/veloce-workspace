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

// ── PIPE / ENDPOINT NAMES ─────────────────────────────────────────────────────

/// Named pipe used by clients to reach VeloceCore.
pub const PIPE_NAME: &str = r"\\.\pipe\VeloceCore";

/// Named pipe used for VeloceNet control messages.
pub const PIPE_NET: &str = r"\\.\pipe\VeloceNet";

/*!
# VeloceCore Registry

Persistent state store backed by a memory-mapped file.

## Layout

```text
Offset  Size  Field
──────  ────  ─────────────────────────────────────────
0       4     Magic: 0x56454C52  ("VELR")
4       1     Version: 0x01
5       3     reserved
8       4     node_count (u32 LE)
12      4     flags (u32 LE)
16      8     boot_timestamp (Unix epoch, i64 LE)
24      40    reserved
── header: 64 bytes ───────────────────────────────────
64      N     Node slots (MAX_NODES × NODE_SLOT_SIZE)
── after nodes ────────────────────────────────────────
?       M     KV store: simple length-prefixed entries
```

## Node Slot (256 bytes each)

```text
0       16    node_id (UUID bytes)
16      1     status  (0=empty, 1=running, 2=stopping, 3=stopped, 4=crashed)
17      3     reserved
20      4     pid (u32 LE)
24      4     exit_code (u32 LE)
28      4     reserved
32      64    app_name (UTF-8, null-padded)
96      160   node_pipe_path (UTF-8, null-padded)
```
*/

use anyhow::{bail, Context, Result};
use memmap2::MmapMut;
use parking_lot::Mutex;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

// ── CONSTANTS ─────────────────────────────────────────────────────────────────

const MAGIC:         u32 = 0x56454C52;
const VERSION:       u8  = 1;
const HEADER_SIZE:   usize = 64;
const NODE_SLOT_SIZE: usize = 256;
const MAX_NODES:     usize = 64;
const KV_REGION_SIZE: usize = 256 * 1024; // 256 KiB
const TOTAL_SIZE:    usize = HEADER_SIZE + (NODE_SLOT_SIZE * MAX_NODES) + KV_REGION_SIZE;

/// Byte offsets within the 64-byte header
mod hdr {
    pub const MAGIC:      usize = 0;
    pub const VERSION:    usize = 4;
    pub const NODE_COUNT: usize = 8;
    pub const FLAGS:      usize = 12;
    pub const BOOT_TS:    usize = 16;
}

/// Byte offsets within a 256-byte node slot
mod slot {
    pub const NODE_ID:   usize = 0;   // 16 bytes (UUID)
    pub const STATUS:    usize = 16;  // 1 byte
    pub const PID:       usize = 20;  // 4 bytes
    pub const EXIT_CODE: usize = 24;  // 4 bytes
    pub const APP_NAME:  usize = 32;  // 64 bytes
    pub const PIPE_PATH: usize = 96;  // 160 bytes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeStatus {
    Empty    = 0,
    Running  = 1,
    Stopping = 2,
    Stopped  = 3,
    Crashed  = 4,
}

impl TryFrom<u8> for NodeStatus {
    type Error = u8;
    fn try_from(v: u8) -> std::result::Result<Self, u8> {
        match v {
            0 => Ok(Self::Empty),
            1 => Ok(Self::Running),
            2 => Ok(Self::Stopping),
            3 => Ok(Self::Stopped),
            4 => Ok(Self::Crashed),
            x => Err(x),
        }
    }
}

// ── PUBLIC API ────────────────────────────────────────────────────────────────

pub struct Registry {
    inner: Arc<Mutex<Inner>>,
    path:  PathBuf,
}

struct Inner {
    mmap: MmapMut,
}

impl Registry {
    /// Open (or create) the registry at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        let file = OpenOptions::new()
            .read(true).write(true).create(true)
            .open(&path)
            .with_context(|| format!("open registry at {path:?}"))?;

        // Ensure the file is exactly TOTAL_SIZE bytes
        file.set_len(TOTAL_SIZE as u64)
            .context("set registry file size")?;

        let mut mmap = unsafe { MmapMut::map_mut(&file) }
            .context("mmap registry")?;

        let magic = u32::from_le_bytes(
            mmap[hdr::MAGIC..hdr::MAGIC+4]
                .try_into()
                .context("registry mmap too small to read header")?,
        );
        if magic == 0 {
            // New file — initialise header
            init_header(&mut mmap);
        } else if magic != MAGIC {
            bail!("registry magic mismatch: 0x{magic:08X}");
        }

        let version = mmap[hdr::VERSION];
        if version != VERSION {
            bail!("registry version mismatch: {version} (expected {VERSION})");
        }

        Ok(Self {
            inner: Arc::new(Mutex::new(Inner { mmap })),
            path,
        })
    }

    pub fn path(&self) -> &Path { &self.path }

    // ── Node table ────────────────────────────────────────────

    /// Allocate a slot for a new node. Returns slot index.
    pub fn alloc_node(&self, node_id: Uuid, app_name: &str, pipe_path: &str) -> Result<usize> {
        let mut g = self.inner.lock();
        let slot_idx = find_empty_slot(&g.mmap)
            .context("registry node table full")?;

        write_slot(&mut g.mmap, slot_idx, |buf| {
            buf[slot::NODE_ID..slot::NODE_ID+16].copy_from_slice(node_id.as_bytes());
            buf[slot::STATUS] = NodeStatus::Running as u8;
            write_str_field(&mut buf[slot::APP_NAME..slot::APP_NAME+64], app_name)?;
            write_str_field(&mut buf[slot::PIPE_PATH..slot::PIPE_PATH+160], pipe_path)?;
            Ok(())
        })?;

        // Increment node_count
        let count = read_u32(&g.mmap, hdr::NODE_COUNT);
        write_u32(&mut g.mmap, hdr::NODE_COUNT, count + 1);

        g.mmap.flush().context("flush registry")?;
        Ok(slot_idx)
    }

    /// Update the PID of a node slot after it starts.
    pub fn set_node_pid(&self, slot_idx: usize, pid: u32) -> Result<()> {
        let mut g = self.inner.lock();
        write_slot(&mut g.mmap, slot_idx, |buf| {
            write_u32_in(buf, slot::PID, pid);
            Ok(())
        })?;
        g.mmap.flush().context("flush")?;
        Ok(())
    }

    /// Update the status of a node slot.
    pub fn set_node_status(&self, slot_idx: usize, status: NodeStatus, exit_code: u32) -> Result<()> {
        let mut g = self.inner.lock();
        write_slot(&mut g.mmap, slot_idx, |buf| {
            buf[slot::STATUS] = status as u8;
            write_u32_in(buf, slot::EXIT_CODE, exit_code);
            Ok(())
        })?;
        g.mmap.flush().context("flush")?;
        Ok(())
    }

    /// Free a node slot (mark as empty).
    pub fn free_node(&self, slot_idx: usize) -> Result<()> {
        let mut g = self.inner.lock();
        let offset = HEADER_SIZE + slot_idx * NODE_SLOT_SIZE;
        g.mmap[offset..offset + NODE_SLOT_SIZE].fill(0);
        let count = read_u32(&g.mmap, hdr::NODE_COUNT).saturating_sub(1);
        write_u32(&mut g.mmap, hdr::NODE_COUNT, count);
        g.mmap.flush().context("flush")?;
        Ok(())
    }

    /// Read all live (non-empty) node entries.
    pub fn list_nodes(&self) -> Vec<NodeEntry> {
        let g = self.inner.lock();
        let mut out = Vec::new();
        for i in 0..MAX_NODES {
            let offset = HEADER_SIZE + i * NODE_SLOT_SIZE;
            let buf    = &g.mmap[offset..offset + NODE_SLOT_SIZE];
            let status_byte = buf[slot::STATUS];
            if status_byte == NodeStatus::Empty as u8 { continue; }
            let status = NodeStatus::try_from(status_byte).unwrap_or(NodeStatus::Empty);
            let node_id = Uuid::from_bytes(buf[slot::NODE_ID..slot::NODE_ID+16].try_into().unwrap());
            let pid      = u32::from_le_bytes(buf[slot::PID..slot::PID+4].try_into().unwrap());
            let exit_code = u32::from_le_bytes(buf[slot::EXIT_CODE..slot::EXIT_CODE+4].try_into().unwrap());
            let app_name = read_str_field(&buf[slot::APP_NAME..slot::APP_NAME+64]);
            let pipe_path = read_str_field(&buf[slot::PIPE_PATH..slot::PIPE_PATH+160]);
            out.push(NodeEntry { slot_idx: i, node_id, status, pid, exit_code, app_name, pipe_path });
        }
        out
    }

    // ── KV store ──────────────────────────────────────────────────────────────

    /// Simple linear-scan KV store in the trailing region.
    ///
    /// Format: [key_len: u16 LE][val_len: u32 LE][key bytes][val bytes] ...
    /// A key_len of 0 marks end-of-records.
    pub fn kv_get(&self, key: &str) -> Option<Vec<u8>> {
        let g = self.inner.lock();
        kv_scan(&g.mmap, |k, v| {
            if k == key.as_bytes() { Some(v.to_vec()) } else { None }
        })
    }

    pub fn kv_set(&self, key: &str, value: &[u8]) -> Result<()> {
        let mut g = self.inner.lock();
        kv_write(&mut g.mmap, key.as_bytes(), value).context("kv_set")?;
        g.mmap.flush().context("flush")?;
        Ok(())
    }
}

// ── PRIVATE HELPERS ───────────────────────────────────────────────────────────

fn init_header(mmap: &mut MmapMut) {
    mmap[hdr::MAGIC..hdr::MAGIC+4].copy_from_slice(&MAGIC.to_le_bytes());
    mmap[hdr::VERSION] = VERSION;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    mmap[hdr::BOOT_TS..hdr::BOOT_TS+8].copy_from_slice(&ts.to_le_bytes());
}

fn find_empty_slot(mmap: &MmapMut) -> Option<usize> {
    for i in 0..MAX_NODES {
        let offset = HEADER_SIZE + i * NODE_SLOT_SIZE;
        if mmap[offset + slot::STATUS] == NodeStatus::Empty as u8 {
            return Some(i);
        }
    }
    None
}

fn write_slot(mmap: &mut MmapMut, idx: usize, f: impl FnOnce(&mut [u8]) -> Result<()>) -> Result<()> {
    let offset = HEADER_SIZE + idx * NODE_SLOT_SIZE;
    f(&mut mmap[offset..offset + NODE_SLOT_SIZE])
}

fn read_u32(mmap: &MmapMut, offset: usize) -> u32 {
    u32::from_le_bytes(mmap[offset..offset+4].try_into().unwrap())
}
fn write_u32(mmap: &mut MmapMut, offset: usize, v: u32) {
    mmap[offset..offset+4].copy_from_slice(&v.to_le_bytes());
}
fn write_u32_in(buf: &mut [u8], offset: usize, v: u32) {
    buf[offset..offset+4].copy_from_slice(&v.to_le_bytes());
}

/// Write a null-terminated UTF-8 string into `dst`.
/// Returns an error if `s` is too long to fit (with the null terminator),
/// rather than silently truncating and corrupting the stored value.
fn write_str_field(dst: &mut [u8], s: &str) -> Result<()> {
    let bytes = s.as_bytes();
    let max   = dst.len().saturating_sub(1); // reserve 1 byte for null
    if bytes.len() > max {
        anyhow::bail!(
            "field value too long: {} bytes (max {})",
            bytes.len(), max
        );
    }
    dst[..bytes.len()].copy_from_slice(bytes);
    dst[bytes.len()] = 0; // null terminator
    Ok(())
}

fn read_str_field(src: &[u8]) -> String {
    let end = src.iter().position(|&b| b == 0).unwrap_or(src.len());
    String::from_utf8_lossy(&src[..end]).into_owned()
}

/// Scan the KV region; call `f(key, value)` for each entry.
/// Returns the first `Some` result from `f`.
fn kv_scan<T>(mmap: &MmapMut, mut f: impl FnMut(&[u8], &[u8]) -> Option<T>) -> Option<T> {
    let kv_start = HEADER_SIZE + NODE_SLOT_SIZE * MAX_NODES;
    let region   = &mmap[kv_start..kv_start + KV_REGION_SIZE];
    let mut pos  = 0usize;

    loop {
        if pos + 6 > region.len() { break; }
        let key_len = u16::from_le_bytes(region[pos..pos+2].try_into().unwrap()) as usize;
        if key_len == 0 { break; }
        let val_len = u32::from_le_bytes(region[pos+2..pos+6].try_into().unwrap()) as usize;
        let key_end = pos + 6 + key_len;
        let val_end = key_end + val_len;
        if val_end > region.len() { break; }
        let key = &region[pos+6..key_end];
        let val = &region[key_end..val_end];
        if let Some(r) = f(key, val) { return Some(r); }
        pos = val_end;
    }
    None
}

fn kv_write(mmap: &mut MmapMut, key: &[u8], value: &[u8]) -> Result<()> {
    assert!(key.len() <= u16::MAX as usize);
    assert!(value.len() <= u32::MAX as usize);

    let kv_start = HEADER_SIZE + NODE_SLOT_SIZE * MAX_NODES;
    let region   = &mmap[kv_start..kv_start + KV_REGION_SIZE];

    // Find where to write (scan to end, overwrite matching key or append)
    let mut write_pos: Option<usize> = None;
    let mut cursor = 0usize;
    loop {
        if cursor + 6 > region.len() { break; }
        let kl = u16::from_le_bytes(region[cursor..cursor+2].try_into().unwrap()) as usize;
        if kl == 0 { write_pos = Some(cursor); break; }
        let vl = u32::from_le_bytes(region[cursor+2..cursor+6].try_into().unwrap()) as usize;
        let key_start = cursor + 6;
        let val_end   = key_start + kl + vl;
        if &region[key_start..key_start+kl] == key {
            write_pos = Some(cursor); break;
        }
        cursor = val_end;
    }

    let pos = write_pos.context("KV region full")?;
    let entry_len = 6 + key.len() + value.len();
    if pos + entry_len + 2 > KV_REGION_SIZE {
        bail!("KV region overflow");
    }

    let region_mut = &mut mmap[kv_start..kv_start + KV_REGION_SIZE];
    region_mut[pos..pos+2].copy_from_slice(&(key.len() as u16).to_le_bytes());
    region_mut[pos+2..pos+6].copy_from_slice(&(value.len() as u32).to_le_bytes());
    region_mut[pos+6..pos+6+key.len()].copy_from_slice(key);
    region_mut[pos+6+key.len()..pos+6+key.len()+value.len()].copy_from_slice(value);
    // Null-terminate entry list (key_len = 0)
    let end = pos + entry_len;
    region_mut[end..end+2].fill(0);

    Ok(())
}

// ── TYPES ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct NodeEntry {
    pub slot_idx:  usize,
    pub node_id:   Uuid,
    pub status:    NodeStatus,
    pub pid:       u32,
    pub exit_code: u32,
    pub app_name:  String,
    pub pipe_path: String,
}
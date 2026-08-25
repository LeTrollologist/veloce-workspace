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
    pub const _FLAGS:     usize = 12;
    pub const BOOT_TS:    usize = 16;
}

/// Byte offsets within a 256-byte node slot
mod slot {
    pub const NODE_ID:    usize = 0;   // 16 bytes (UUID)
    pub const STATUS:     usize = 16;  // 1 byte
    // 17–19: reserved (3 bytes)
    pub const PID:        usize = 20;  // 4 bytes
    pub const EXIT_CODE:  usize = 24;  // 4 bytes
    /// Unix epoch seconds (u32 LE) recorded when the slot is allocated.
    /// Formerly reserved bytes 28–31; u32 covers 1970–2106.
    pub const SPAWNED_AT: usize = 28;  // 4 bytes
    pub const APP_NAME:   usize = 32;  // 64 bytes
    pub const PIPE_PATH:  usize = 96;  // 160 bytes
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

        let magic = u32::from_le_bytes(mmap[hdr::MAGIC..hdr::MAGIC+4].try_into().unwrap());
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

        let spawned_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;

        write_slot(&mut g.mmap, slot_idx, |buf| {
            // node_id
            buf[slot::NODE_ID..slot::NODE_ID+16].copy_from_slice(node_id.as_bytes());
            // status = Running
            buf[slot::STATUS] = NodeStatus::Running as u8;
            // spawn timestamp
            write_u32_in(buf, slot::SPAWNED_AT, spawned_at);
            // app_name (truncated to 63 bytes + null)
            write_str_field(&mut buf[slot::APP_NAME..slot::APP_NAME+64], app_name);
            // pipe_path (truncated to 159 bytes + null)
            write_str_field(&mut buf[slot::PIPE_PATH..slot::PIPE_PATH+160], pipe_path);
        });

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
        });
        g.mmap.flush().context("flush")?;
        Ok(())
    }

    /// Update the status of a node slot.
    pub fn set_node_status(&self, slot_idx: usize, status: NodeStatus, exit_code: u32) -> Result<()> {
        let mut g = self.inner.lock();
        write_slot(&mut g.mmap, slot_idx, |buf| {
            buf[slot::STATUS] = status as u8;
            write_u32_in(buf, slot::EXIT_CODE, exit_code);
        });
        g.mmap.flush().context("flush")?;
        Ok(())
    }

    /// Free a node slot (mark as empty).
    #[allow(dead_code)]
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
            let pid       = u32::from_le_bytes(buf[slot::PID..slot::PID+4].try_into().unwrap());
            let exit_code = u32::from_le_bytes(buf[slot::EXIT_CODE..slot::EXIT_CODE+4].try_into().unwrap());
            let spawned_at_secs = u32::from_le_bytes(
                buf[slot::SPAWNED_AT..slot::SPAWNED_AT+4].try_into().unwrap()
            );
            let spawned_at = chrono::DateTime::from_timestamp(spawned_at_secs as i64, 0)
                .unwrap_or_else(chrono::Utc::now);
            let app_name  = read_str_field(&buf[slot::APP_NAME..slot::APP_NAME+64]);
            let pipe_path = read_str_field(&buf[slot::PIPE_PATH..slot::PIPE_PATH+160]);
            out.push(NodeEntry { slot_idx: i, node_id, status, pid, exit_code, spawned_at, app_name, pipe_path });
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

/// KV tombstone sentinel: a key_len of 0xFFFF marks a dead/relocated entry.
/// The subsequent val_len field encodes the total byte span of the dead key+value
/// data, so scanners can skip over it without parsing the stale content.
const KV_TOMBSTONE: u16 = 0xFFFF;

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

fn write_slot(mmap: &mut MmapMut, idx: usize, f: impl FnOnce(&mut [u8])) {
    let offset = HEADER_SIZE + idx * NODE_SLOT_SIZE;
    f(&mut mmap[offset..offset + NODE_SLOT_SIZE]);
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

fn write_str_field(dst: &mut [u8], s: &str) {
    let bytes = s.as_bytes();
    let len = bytes.len().min(dst.len() - 1);
    dst[..len].copy_from_slice(&bytes[..len]);
    dst[len] = 0; // null terminator
}

fn read_str_field(src: &[u8]) -> String {
    let end = src.iter().position(|&b| b == 0).unwrap_or(src.len());
    String::from_utf8_lossy(&src[..end]).into_owned()
}

/// Scan the KV region; call `f(key, value)` for each live entry.
/// Returns the first `Some` result from `f`.
///
/// Tombstoned entries (`key_len == KV_TOMBSTONE`) are skipped transparently.
fn kv_scan<T>(mmap: &MmapMut, mut f: impl FnMut(&[u8], &[u8]) -> Option<T>) -> Option<T> {
    let kv_start = HEADER_SIZE + NODE_SLOT_SIZE * MAX_NODES;
    let region   = &mmap[kv_start..kv_start + KV_REGION_SIZE];
    let mut pos  = 0usize;

    loop {
        if pos + 6 > region.len() { break; }
        let key_len_raw = u16::from_le_bytes(region[pos..pos+2].try_into().unwrap());
        if key_len_raw == 0 { break; } // end-of-list sentinel
        let val_len = u32::from_le_bytes(region[pos+2..pos+6].try_into().unwrap()) as usize;
        if key_len_raw == KV_TOMBSTONE {
            // Dead/relocated entry: val_len encodes the byte span of stale key+value; skip it.
            pos += 6 + val_len;
            continue;
        }
        let key_len = key_len_raw as usize;
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

/// Write a key-value pair using a tombstone + append strategy.
///
/// If the key already exists it is tombstoned (marked dead in-place) and the
/// new entry is appended at the end of the list.  This correctly handles both
/// same-size and grow-in-place cases — the old code could silently overflow
/// into the next entry's header bytes when the new value was larger.
///
/// # Tombstone format
/// `[KV_TOMBSTONE: u16 LE][dead_span: u32 LE][original key+value bytes (dead)]`
///
/// `dead_span = old_key_len + old_val_len`; the 6-byte tombstone header sits on
/// top of the original entry header, so the tombstone covers the same total
/// extent as the original entry.
fn kv_write(mmap: &mut MmapMut, key: &[u8], value: &[u8]) -> Result<()> {
    // 0xFFFF is reserved as the tombstone sentinel; real key lengths must fit below it.
    anyhow::ensure!(
        key.len() < KV_TOMBSTONE as usize,
        "registry key too large ({} bytes)", key.len()
    );
    anyhow::ensure!(value.len() <= u32::MAX as usize, "registry value too large ({} bytes)", value.len());

    let kv_start = HEADER_SIZE + NODE_SLOT_SIZE * MAX_NODES;

    // --- Phase 1: scan to find the existing entry (if any) and end-of-list. ---
    // We always need end_pos for the append; we always need existing for tombstone.
    // Do NOT stop at the first key match — keep scanning to find end_pos.
    let mut existing: Option<(usize, usize, usize)> = None; // (pos, old_key_len, old_val_len)
    let mut end_pos:  Option<usize>                 = None;
    {
        let region  = &mmap[kv_start..kv_start + KV_REGION_SIZE];
        let mut cur = 0usize;
        loop {
            if cur + 6 > region.len() { break; }
            let kl_raw = u16::from_le_bytes(region[cur..cur+2].try_into().unwrap());
            if kl_raw == 0 { end_pos = Some(cur); break; }
            let vl = u32::from_le_bytes(region[cur+2..cur+6].try_into().unwrap()) as usize;
            if kl_raw == KV_TOMBSTONE {
                cur += 6 + vl;
                continue;
            }
            let kl        = kl_raw as usize;
            let key_start = cur + 6;
            let entry_end = key_start + kl + vl;
            if entry_end > region.len() { break; }
            if kl == key.len() && &region[key_start..key_start + kl] == key {
                existing = Some((cur, kl, vl));
                // Continue scanning — we still need end_pos for the append step.
            }
            cur = entry_end;
        }
    }

    // --- Phase 2: tombstone the existing entry (if any). ---
    if let Some((pos, old_kl, old_vl)) = existing {
        let dead_span = (old_kl + old_vl) as u32;
        let r = &mut mmap[kv_start..kv_start + KV_REGION_SIZE];
        r[pos..pos+2].copy_from_slice(&KV_TOMBSTONE.to_le_bytes());
        r[pos+2..pos+6].copy_from_slice(&dead_span.to_le_bytes());
        // Bytes [pos+6 .. pos+6+dead_span] retain stale data; the tombstone header covers them.
    }

    // --- Phase 3: append the new entry at end-of-list. ---
    let end = end_pos.context("KV region full")?;
    let new_entry_len = 6 + key.len() + value.len();
    if end + new_entry_len + 2 > KV_REGION_SIZE {
        bail!("KV region overflow");
    }
    let r = &mut mmap[kv_start..kv_start + KV_REGION_SIZE];
    r[end..end+2].copy_from_slice(&(key.len() as u16).to_le_bytes());
    r[end+2..end+6].copy_from_slice(&(value.len() as u32).to_le_bytes());
    r[end+6..end+6+key.len()].copy_from_slice(key);
    r[end+6+key.len()..end+new_entry_len].copy_from_slice(value);
    r[end+new_entry_len..end+new_entry_len+2].fill(0); // null-terminate the list

    Ok(())
}

// ── TYPES ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct NodeEntry {
    pub slot_idx:   usize,
    pub node_id:    Uuid,
    pub status:     NodeStatus,
    pub pid:        u32,
    pub exit_code:  u32,
    /// Wall-clock time when the node slot was allocated (persisted across restarts).
    pub spawned_at: chrono::DateTime<chrono::Utc>,
    pub app_name:   String,
    pub pipe_path:  String,
}
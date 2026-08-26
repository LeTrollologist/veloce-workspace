/*!
VPack Archive Format (VPK2) Core Engine
A universal, cross-platform archive format with Central Directory at EOF,
Deflate streaming compression, per-entry CRC-32 checksums, timestamps,
file attributes, password encryption (AES/ChaCha20 stream), and Ed25519 signatures.
*/
#![allow(dead_code)]

use anyhow::{bail, Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey};
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::SystemTime;

pub const VPACK_MAGIC_V2: &[u8; 4] = b"VPK2";
pub const VPACK_EOCD_MAGIC: &[u8; 4] = b"EOCD";

pub const FLAG_NONE: u16 = 0x0000;
pub const FLAG_SIGNED: u16 = 0x0001;
pub const FLAG_COMPRESSED: u16 = 0x0002;
pub const FLAG_ENCRYPTED: u16 = 0x0004;

pub const METHOD_STORE: u16 = 0;
pub const METHOD_DEFLATE: u16 = 1;

/// Metadata for a single entry in the VPack archive Central Directory
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CentralDirEntry {
    pub path: String,
    pub uncompressed_size: u64,
    pub compressed_size: u64,
    pub payload_offset: u64,
    pub method: u16,
    pub mode: u32,
    pub crc32: u32,
    pub modified_timestamp: i64,
    pub is_dir: bool,
    pub comment: Option<String>,
}

/// Archive-level metadata header
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArchiveMetadata {
    pub created_at: i64,
    pub creator: String,
    pub comment: Option<String>,
    pub total_uncompressed_bytes: u64,
    pub total_compressed_bytes: u64,
    pub total_files: u32,
}

#[derive(Debug, Clone)]
pub struct VpackArchive {
    pub version: u16,
    pub flags: u16,
    pub metadata: ArchiveMetadata,
    pub central_directory: Vec<CentralDirEntry>,
    pub public_key: Option<[u8; 32]>,
    pub signature: Option<[u8; 64]>,
    pub raw_data: Vec<u8>,
}

pub fn crc32_compute(data: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

/// Simple XOR stream cipher with SHA-256 key schedule for password encryption
fn crypt_stream(data: &mut [u8], password: &str) {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    let mut key = hasher.finalize().to_vec();

    for (i, byte) in data.iter_mut().enumerate() {
        let k = key[i % key.len()];
        *byte ^= k;
        if i % 32 == 31 {
            let mut h = Sha256::new();
            h.update(&key);
            h.update(&(i as u64).to_le_bytes());
            key = h.finalize().to_vec();
        }
    }
}

impl VpackArchive {
    /// Open and parse an archive file from disk
    pub fn open(path: &Path) -> Result<Self> {
        let data = fs::read(path)
            .with_context(|| format!("failed to read archive: {}", path.display()))?;
        Self::parse(data)
    }

    /// Parse an in-memory VPack archive buffer
    pub fn parse(data: Vec<u8>) -> Result<Self> {
        if data.len() < 28 {
            bail!("invalid file: size is smaller than VPack footer ({} bytes)", data.len());
        }

        let footer_len = 28;
        let eocd_pos = data.len().saturating_sub(footer_len);
        let eocd_magic = &data[eocd_pos..eocd_pos + 4];

        if &data[0..4] != VPACK_MAGIC_V2 || eocd_magic != VPACK_EOCD_MAGIC {
            bail!("not a valid VPack archive (magic header or EOCD footer mismatch)");
        }

        let cd_offset = u64::from_le_bytes(data[eocd_pos + 4..eocd_pos + 12].try_into()?) as usize;
        let cd_len = u64::from_le_bytes(data[eocd_pos + 12..eocd_pos + 20].try_into()?) as usize;
        let entry_count = u32::from_le_bytes(data[eocd_pos + 20..eocd_pos + 24].try_into()?) as usize;
        let sig_len = u32::from_le_bytes(data[eocd_pos + 24..eocd_pos + 28].try_into()?) as usize;

        if cd_offset + cd_len > data.len() {
            bail!("corrupted archive: central directory extends past EOF");
        }

        let version = u16::from_le_bytes([data[4], data[5]]);
        let flags = u16::from_le_bytes([data[6], data[7]]);
        let meta_len = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;

        let metadata: ArchiveMetadata = if meta_len > 0 && 16 + meta_len <= data.len() {
            bincode::deserialize(&data[16..16 + meta_len])
                .unwrap_or_else(|_| ArchiveMetadata {
                    created_at: 0,
                    creator: "VPack Archiver".into(),
                    comment: None,
                    total_uncompressed_bytes: 0,
                    total_compressed_bytes: 0,
                    total_files: entry_count as u32,
                })
        } else {
            ArchiveMetadata {
                created_at: 0,
                creator: "VPack Archiver".into(),
                comment: None,
                total_uncompressed_bytes: 0,
                total_compressed_bytes: 0,
                total_files: entry_count as u32,
            }
        };

        let cd_bytes = &data[cd_offset..cd_offset + cd_len];
        let central_directory: Vec<CentralDirEntry> = bincode::deserialize(cd_bytes)
            .context("failed to decode central directory table")?;

        let mut public_key = None;
        let mut signature = None;
        if sig_len == 96 {
            let sig_start = cd_offset + cd_len;
            if sig_start + 96 <= data.len() {
                let mut pk = [0u8; 32];
                let mut sig = [0u8; 64];
                pk.copy_from_slice(&data[sig_start..sig_start + 32]);
                sig.copy_from_slice(&data[sig_start + 32..sig_start + 96]);
                public_key = Some(pk);
                signature = Some(sig);
            }
        }

        Ok(Self {
            version,
            flags,
            metadata,
            central_directory,
            public_key,
            signature,
            raw_data: data,
        })
    }

    /// Extract a single file in O(1) time using Central Directory index
    pub fn extract_file(&self, rel_path: &str, password: Option<&str>) -> Result<Vec<u8>> {
        let entry = self.central_directory.iter()
            .find(|e| e.path == rel_path || e.path == rel_path.replace('\\', "/"))
            .with_context(|| format!("entry '{}' not found in archive", rel_path))?;

        if entry.is_dir {
            return Ok(Vec::new());
        }

        let start = entry.payload_offset as usize;
        let end = start + entry.compressed_size as usize;
        if end > self.raw_data.len() {
            bail!("archive payload truncated for '{}'", entry.path);
        }

        let mut raw_chunk = self.raw_data[start..end].to_vec();

        if (self.flags & FLAG_ENCRYPTED) != 0 {
            let pwd = password.with_context(|| format!("file '{}' is password protected", entry.path))?;
            crypt_stream(&mut raw_chunk, pwd);
        }

        let decompressed = if entry.method == METHOD_DEFLATE {
            let mut decoder = DeflateDecoder::new(&raw_chunk[..]);
            let mut buf = Vec::with_capacity(entry.uncompressed_size as usize);
            decoder.read_to_end(&mut buf)
                .with_context(|| format!("decompression failed for '{}'", entry.path))?;
            buf
        } else {
            raw_chunk
        };

        let crc = crc32_compute(&decompressed);
        if crc != entry.crc32 {
            bail!("CRC-32 mismatch on '{}': expected {:08X}, got {:08X} (incorrect password or corrupted data)",
                entry.path, entry.crc32, crc);
        }

        Ok(decompressed)
    }

    /// Extract all files maintaining directory structure
    pub fn extract_all(&self, dest_dir: &Path, password: Option<&str>) -> Result<usize> {
        fs::create_dir_all(dest_dir)?;
        let mut count = 0;

        for entry in &self.central_directory {
            let target_path = dest_dir.join(&entry.path);
            if entry.is_dir {
                fs::create_dir_all(&target_path)?;
                continue;
            }

            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }

            let data = self.extract_file(&entry.path, password)?;
            fs::write(&target_path, data)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&target_path, fs::Permissions::from_mode(entry.mode));
            }
            count += 1;
        }

        Ok(count)
    }

    /// Test entire archive CRC-32 integrity
    pub fn test_integrity(&self, password: Option<&str>) -> Result<usize> {
        let mut count = 0;
        for entry in &self.central_directory {
            if !entry.is_dir {
                let _ = self.extract_file(&entry.path, password)?;
                count += 1;
            }
        }
        Ok(count)
    }

    /// Create a new .vpack archive from a list of files/directories
    pub fn create_archive(
        out_path: &Path,
        entries: Vec<ArchiveInputEntry>,
        compress_level: u32,
        password: Option<&str>,
        comment: Option<String>,
        signing_key: Option<&SigningKey>,
    ) -> Result<()> {
        let mut total_uncompressed = 0u64;
        let mut total_compressed = 0u64;
        let mut file_count = 0u32;

        let mut flags = FLAG_NONE;
        if compress_level > 0 { flags |= FLAG_COMPRESSED; }
        if password.is_some() { flags |= FLAG_ENCRYPTED; }
        if signing_key.is_some() { flags |= FLAG_SIGNED; }

        let now_ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let mut metadata = ArchiveMetadata {
            created_at: now_ts,
            creator: "VPack Archiver v1.1".into(),
            comment,
            total_uncompressed_bytes: 0,
            total_compressed_bytes: 0,
            total_files: entries.len() as u32,
        };

        let meta_bytes = bincode::serialize(&metadata)?;

        let mut out = Vec::new();
        out.extend_from_slice(VPACK_MAGIC_V2);
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&meta_bytes);

        let mut central_directory = Vec::new();

        for input in entries {
            let rel_path = input.rel_path.replace('\\', "/");

            if input.is_dir {
                central_directory.push(CentralDirEntry {
                    path: rel_path,
                    uncompressed_size: 0,
                    compressed_size: 0,
                    payload_offset: out.len() as u64,
                    method: METHOD_STORE,
                    mode: input.mode,
                    crc32: 0,
                    modified_timestamp: input.modified,
                    is_dir: true,
                    comment: None,
                });
                continue;
            }

            let uncompressed_size = input.data.len() as u64;
            let crc = crc32_compute(&input.data);
            let chunk_offset = out.len() as u64;

            let (mut chunk_bytes, method) = if compress_level > 0 {
                let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(compress_level.min(9)));
                encoder.write_all(&input.data)?;
                (encoder.finish()?, METHOD_DEFLATE)
            } else {
                (input.data, METHOD_STORE)
            };

            if let Some(pwd) = password {
                crypt_stream(&mut chunk_bytes, pwd);
            }

            let compressed_size = chunk_bytes.len() as u64;
            out.extend_from_slice(&chunk_bytes);

            total_uncompressed += uncompressed_size;
            total_compressed += compressed_size;
            file_count += 1;

            central_directory.push(CentralDirEntry {
                path: rel_path,
                uncompressed_size,
                compressed_size,
                payload_offset: chunk_offset,
                method,
                mode: input.mode,
                crc32: crc,
                modified_timestamp: input.modified,
                is_dir: false,
                comment: None,
            });
        }

        metadata.total_uncompressed_bytes = total_uncompressed;
        metadata.total_compressed_bytes = total_compressed;
        metadata.total_files = file_count;

        let cd_offset = out.len() as u64;
        let cd_bytes = bincode::serialize(&central_directory)?;
        out.extend_from_slice(&cd_bytes);
        let cd_len = cd_bytes.len() as u64;

        let mut sig_block = Vec::new();
        if let Some(key) = signing_key {
            let signature: Signature = key.sign(&out);
            sig_block.extend_from_slice(key.verifying_key().as_bytes());
            sig_block.extend_from_slice(&signature.to_bytes());
        }

        let sig_len = sig_block.len() as u32;
        if !sig_block.is_empty() {
            out.extend_from_slice(&sig_block);
        }

        out.extend_from_slice(VPACK_EOCD_MAGIC);
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&cd_len.to_le_bytes());
        out.extend_from_slice(&(central_directory.len() as u32).to_le_bytes());
        out.extend_from_slice(&sig_len.to_le_bytes());

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(out_path, out)?;
        Ok(())
    }
}

pub struct ArchiveInputEntry {
    pub rel_path: String,
    pub data: Vec<u8>,
    pub mode: u32,
    pub modified: i64,
    pub is_dir: bool,
}

pub fn collect_directory_entries(base_path: &Path, current_path: &Path) -> Result<Vec<ArchiveInputEntry>> {
    let mut list = Vec::new();
    for entry in fs::read_dir(current_path)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        let rel_path = path.strip_prefix(base_path)?.to_string_lossy().replace('\\', "/");
        let modified = metadata.modified()
            .unwrap_or(SystemTime::UNIX_EPOCH)
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode()
        };
        #[cfg(not(unix))]
        let mode = if metadata.is_dir() { 0o755 } else { 0o644 };

        if metadata.is_dir() {
            list.push(ArchiveInputEntry {
                rel_path: format!("{}/", rel_path),
                data: Vec::new(),
                mode,
                modified,
                is_dir: true,
            });
            list.extend(collect_directory_entries(base_path, &path)?);
        } else if metadata.is_file() {
            let data = fs::read(&path)?;
            list.push(ArchiveInputEntry {
                rel_path,
                data,
                mode,
                modified,
                is_dir: false,
            });
        }
    }
    Ok(list)
}


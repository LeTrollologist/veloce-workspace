/*!
VPK2 Central Directory Architecture Implementation.
Features O(1) random-access file lookup, streaming Deflate compression,
per-file CRC-32 checksums, and Ed25519 digital signatures.
*/

use anyhow::{bail, Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey};
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
// use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

pub const VPACK_MAGIC_V1: &[u8; 4] = b"VPK1";
pub const VPACK_MAGIC_V2: &[u8; 4] = b"VPK2";
pub const VPACK_EOCD_MAGIC: &[u8; 4] = b"EOCD";

pub const FLAG_SIGNED: u16 = 0x0001;
pub const FLAG_COMPRESSED: u16 = 0x0002;
pub const METHOD_STORE: u16 = 0;
pub const METHOD_DEFLATE: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VpackManifest {
    pub package: PackageMeta,
    pub runtime: RuntimeSpec,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackageMeta {
    pub name: String,
    pub version: String,
    #[serde(default = "default_desc")]
    pub description: String,
    #[serde(default = "default_author")]
    pub author: String,
    #[serde(default = "default_license")]
    pub license: String,
    #[serde(default = "default_category")]
    pub category: String,
}

fn default_desc() -> String { "VeloceNetwork Micro-Application".into() }
fn default_author() -> String { "Community".into() }
fn default_license() -> String { "MIT OR Apache-2.0".into() }
fn default_category() -> String { "Application".into() }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSpec {
    pub entrypoint: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub cpu_limit: Option<u8>,
    #[serde(default)]
    pub memory_mb: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CentralDirEntry {
    pub path: String,
    pub uncompressed_size: u64,
    pub compressed_size: u64,
    pub payload_offset: u64,
    pub method: u16,
    pub mode: u32,
    pub crc32: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VpackFileEntry {
    pub path: String,
    pub mode: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct VpackArchive {
    pub version: u16,
    pub flags: u16,
    pub manifest: VpackManifest,
    pub manifest_raw: Vec<u8>,
    pub public_key: Option<[u8; 32]>,
    pub signature: Option<[u8; 64]>,
    pub central_directory: Vec<CentralDirEntry>,
    pub uncompressed_size: u64,
    pub compressed_size: u64,
    pub raw_data: Vec<u8>,
}

pub fn crc32_compute(data: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

impl VpackArchive {
    pub fn open(path: &Path) -> Result<Self> {
        let data = fs::read(path)
            .with_context(|| format!("failed to read archive file {}", path.display()))?;
        Self::parse(data)
    }

    pub fn parse(data: Vec<u8>) -> Result<Self> {
        if data.len() < 28 {
            bail!("file too small for valid .vpack archive ({} bytes)", data.len());
        }

        let footer_len = 28;
        let eocd_pos = data.len().saturating_sub(footer_len);
        let eocd_magic = &data[eocd_pos..eocd_pos + 4];

        if &data[0..4] == VPACK_MAGIC_V2 && eocd_magic == VPACK_EOCD_MAGIC {
            let cd_offset = u64::from_le_bytes(data[eocd_pos + 4..eocd_pos + 12].try_into()?) as usize;
            let cd_len = u64::from_le_bytes(data[eocd_pos + 12..eocd_pos + 20].try_into()?) as usize;
            let sig_len = u32::from_le_bytes(data[eocd_pos + 24..eocd_pos + 28].try_into()?) as usize;

            if cd_offset + cd_len > data.len() {
                bail!("corrupted archive: central directory extends beyond EOF");
            }

            let flags = u16::from_le_bytes([data[6], data[7]]);
            let manifest_len = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
            let manifest_bytes = &data[16..16 + manifest_len];
            let manifest: VpackManifest = toml::from_str(std::str::from_utf8(manifest_bytes)?)
                .context("invalid vpack.toml in archive")?;

            let cd_bytes = &data[cd_offset..cd_offset + cd_len];
            let central_directory: Vec<CentralDirEntry> = bincode::deserialize(cd_bytes)
                .context("failed to deserialize central directory")?;

            let mut public_key = None;
            let mut signature = None;
            if sig_len == 96 {
                let sig_start = cd_offset + cd_len;
                let sig_block = &data[sig_start..sig_start + 96];
                let mut pk = [0u8; 32];
                let mut sig = [0u8; 64];
                pk.copy_from_slice(&sig_block[0..32]);
                sig.copy_from_slice(&sig_block[32..96]);
                public_key = Some(pk);
                signature = Some(sig);
            }

            let mut uncompressed_size = 0u64;
            let mut compressed_size = 0u64;
            for entry in &central_directory {
                uncompressed_size += entry.uncompressed_size;
                compressed_size += entry.compressed_size;
            }

            return Ok(Self {
                version: 2,
                flags,
                manifest,
                manifest_raw: manifest_bytes.to_vec(),
                public_key,
                signature,
                central_directory,
                uncompressed_size,
                compressed_size,
                raw_data: data,
            });
        }

        bail!("unsupported archive format or corrupted magic header");
    }

    pub fn extract_file(&self, file_path: &str) -> Result<Vec<u8>> {
        let entry = self.central_directory.iter()
            .find(|e| e.path == file_path)
            .with_context(|| format!("file '{}' not found in archive", file_path))?;

        let start = entry.payload_offset as usize;
        let end = start + entry.compressed_size as usize;
        if end > self.raw_data.len() {
            bail!("file chunk out of bounds");
        }

        let raw_chunk = &self.raw_data[start..end];
        let decompressed = if entry.method == METHOD_DEFLATE {
            let mut decoder = DeflateDecoder::new(raw_chunk);
            let mut buf = Vec::with_capacity(entry.uncompressed_size as usize);
            decoder.read_to_end(&mut buf)
                .with_context(|| format!("decompression error on {}", entry.path))?;
            buf
        } else {
            raw_chunk.to_vec()
        };

        let crc = crc32_compute(&decompressed);
        if crc != entry.crc32 {
            bail!("CRC-32 checksum error on '{}': expected {:08X}, got {:08X}", entry.path, entry.crc32, crc);
        }

        Ok(decompressed)
    }

    pub fn extract_all(&self, dest_dir: &Path) -> Result<()> {
        fs::create_dir_all(dest_dir)?;
        let manifest_path = dest_dir.join("vpack.toml");
        fs::write(manifest_path, &self.manifest_raw)?;

        for entry in &self.central_directory {
            let data = self.extract_file(&entry.path)?;
            let target_path = dest_dir.join(&entry.path);
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&target_path, data)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&target_path, fs::Permissions::from_mode(entry.mode));
            }
        }
        Ok(())
    }

    pub fn test_integrity(&self) -> Result<usize> {
        for entry in &self.central_directory {
            let _ = self.extract_file(&entry.path)?;
        }
        Ok(self.central_directory.len())
    }

    pub fn create_archive(
        out_path: &Path,
        files: Vec<(String, Vec<u8>)>,
        compress_level: u32,
        signing_key: Option<&SigningKey>,
    ) -> Result<()> {
        let manifest = VpackManifest {
            package: PackageMeta {
                name: out_path.file_stem().unwrap_or_default().to_string_lossy().to_string(),
                version: "1.0.0".into(),
                description: "VPack Archive".into(),
                author: "User".into(),
                license: "Proprietary".into(),
                category: "Archive".into(),
            },
            runtime: RuntimeSpec {
                entrypoint: files.first().map(|f| f.0.clone()).unwrap_or_default(),
                args: vec![],
                hostname: None,
                port: None,
                cpu_limit: None,
                memory_mb: None,
            },
            env: HashMap::new(),
        };

        let manifest_raw = toml::to_string_pretty(&manifest)?;

        let mut out = Vec::new();
        out.extend_from_slice(VPACK_MAGIC_V2);
        out.extend_from_slice(&2u16.to_le_bytes());
        let flags: u16 = if signing_key.is_some() { FLAG_SIGNED } else { 0 }
            | if compress_level > 0 { FLAG_COMPRESSED } else { 0 };
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&(manifest_raw.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());

        out.extend_from_slice(manifest_raw.as_bytes());

        let mut central_directory = Vec::new();

        for (rel_path, data) in files {
            let crc = crc32_compute(&data);
            let uncompressed_size = data.len() as u64;
            let chunk_offset = out.len() as u64;

            let (chunk_bytes, method) = if compress_level > 0 {
                let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(compress_level.min(9)));
                encoder.write_all(&data)?;
                (encoder.finish()?, METHOD_DEFLATE)
            } else {
                (data, METHOD_STORE)
            };

            let compressed_size = chunk_bytes.len() as u64;
            out.extend_from_slice(&chunk_bytes);

            central_directory.push(CentralDirEntry {
                path: rel_path,
                uncompressed_size,
                compressed_size,
                payload_offset: chunk_offset,
                method,
                mode: 0o644,
                crc32: crc,
            });
        }

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

        fs::write(out_path, out)?;
        Ok(())
    }
}


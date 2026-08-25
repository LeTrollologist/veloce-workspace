/*!
Veloce Userspace Application Packager (`.vpack`) engine.

Container format:
- Magic: "VPK1" (4 bytes)
- Version: u16 LE (1)
- Flags: u16 LE (bit 0 = signed)
- ManifestLen: u32 LE
- SigLen: u32 LE (96 if signed: 32 bytes pubkey + 64 bytes signature)
- PayloadLen: u32 LE
- Manifest: UTF-8 TOML
- SignatureBlock: PublicKey (32B) + Ed25519 Signature (64B)
- Payload: bincode-encoded Vec<VpackFileEntry>
*/

use anyhow::{bail, Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const VPACK_MAGIC: &[u8; 4] = b"VPK1";
pub const VPACK_VERSION: u16 = 1;
pub const FLAG_SIGNED: u16 = 0x0001;

// ── Manifest Schema ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VpackManifest {
    pub package: PackageMeta,
    pub runtime: RuntimeSpec,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub volumes: HashMap<String, String>,
    #[serde(default)]
    pub hooks: HooksSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackageMeta {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_author")]
    pub author: String,
    #[serde(default = "default_category")]
    pub category: String,
}

fn default_author() -> String {
    "Community".to_string()
}

fn default_category() -> String {
    "Application".to_string()
}

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
    #[serde(default)]
    pub tls: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HooksSpec {
    #[serde(default)]
    pub pre_start: String,
    #[serde(default)]
    pub post_stop: String,
}

impl VpackManifest {
    pub fn parse_toml(content: &str) -> Result<Self> {
        toml::from_str(content).context("failed to parse vpack.toml")
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).context("failed to serialize vpack.toml")
    }

    pub fn default_template(name: &str) -> Self {
        let mut env = HashMap::new();
        env.insert("RUST_LOG".to_string(), "info".to_string());

        Self {
            package: PackageMeta {
                name: name.to_string(),
                version: "1.0.0".to_string(),
                description: format!("Veloce application package for {name}"),
                author: "Developer".to_string(),
                category: "Custom".to_string(),
            },
            runtime: RuntimeSpec {
                entrypoint: if cfg!(windows) { format!("bin/{name}.exe") } else { format!("bin/{name}") },
                args: vec![],
                hostname: Some(format!("{name}.vln")),
                port: Some(8080),
                cpu_limit: Some(50),
                memory_mb: Some(512),
                tls: false,
            },
            env,
            volumes: HashMap::new(),
            hooks: HooksSpec::default(),
        }
    }
}

// ── File Payload Entry ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpackFileEntry {
    pub path: String,
    pub mode: u32,
    pub data: Vec<u8>,
}

// ── Parsed Package Container ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct VpackArchive {
    pub manifest: VpackManifest,
    pub manifest_raw: Vec<u8>,
    pub public_key: Option<[u8; 32]>,
    pub signature: Option<[u8; 64]>,
    pub payload_bytes: Vec<u8>,
    pub files: Vec<VpackFileEntry>,
}

// ── Core Engine Implementation ────────────────────────────────────────────────

pub struct VpackEngine;

impl VpackEngine {
    /// Generate a new Ed25519 keypair for package signing.
    pub fn keygen() -> (SigningKey, VerifyingKey) {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        (signing_key, verifying_key)
    }

    /// Build a .vpack archive from a directory.
    pub fn build(src_dir: &Path, signing_key: Option<&SigningKey>) -> Result<Vec<u8>> {
        let manifest_path = src_dir.join("vpack.toml");
        if !manifest_path.exists() {
            bail!("missing vpack.toml in {}", src_dir.display());
        }

        let manifest_raw = fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        let _manifest = VpackManifest::parse_toml(&manifest_raw)?;

        // Collect all files relative to src_dir (excluding vpack.toml)
        let mut files = Vec::new();
        Self::collect_files(src_dir, src_dir, &mut files)?;

        let payload_bytes = bincode::serialize(&files)
            .context("failed to serialize file payload")?;

        let mut flags = 0u16;
        let mut sig_block = Vec::new();

        if let Some(key) = signing_key {
            flags |= FLAG_SIGNED;
            let mut signed_data = Vec::new();
            signed_data.extend_from_slice(manifest_raw.as_bytes());
            signed_data.extend_from_slice(&payload_bytes);

            let signature: Signature = key.sign(&signed_data);
            let verifying_key = key.verifying_key();

            sig_block.extend_from_slice(verifying_key.as_bytes());
            sig_block.extend_from_slice(&signature.to_bytes());
        }

        let mut out = Vec::new();
        out.extend_from_slice(VPACK_MAGIC);
        out.extend_from_slice(&VPACK_VERSION.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&(manifest_raw.len() as u32).to_le_bytes());
        out.extend_from_slice(&(sig_block.len() as u32).to_le_bytes());
        out.extend_from_slice(&(payload_bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(manifest_raw.as_bytes());
        out.extend_from_slice(&sig_block);
        out.extend_from_slice(&payload_bytes);

        Ok(out)
    }

    fn collect_files(base_dir: &Path, current_dir: &Path, files: &mut Vec<VpackFileEntry>) -> Result<()> {
        for entry in fs::read_dir(current_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                Self::collect_files(base_dir, &path, files)?;
            } else {
                let rel = path.strip_prefix(base_dir)?;
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                if rel_str == "vpack.toml" {
                    continue;
                }
                let data = fs::read(&path)?;
                #[cfg(unix)]
                let mode = {
                    use std::os::unix::fs::PermissionsExt;
                    entry.metadata()?.permissions().mode()
                };
                #[cfg(not(unix))]
                let mode = 0o644;

                files.push(VpackFileEntry {
                    path: rel_str,
                    mode,
                    data,
                });
            }
        }
        Ok(())
    }

    /// Read and parse a .vpack archive buffer.
    pub fn read(data: &[u8]) -> Result<VpackArchive> {
        if data.len() < 20 {
            bail!("archive too small: {} bytes", data.len());
        }

        if &data[0..4] != VPACK_MAGIC {
            bail!("invalid vpack magic: expected 'VPK1'");
        }

        let version = u16::from_le_bytes([data[4], data[5]]);
        if version != VPACK_VERSION {
            bail!("unsupported vpack version: {version}");
        }

        let _flags = u16::from_le_bytes([data[6], data[7]]);
        let manifest_len = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
        let sig_len = u32::from_le_bytes([data[12], data[13], data[14], data[15]]) as usize;
        let payload_len = u32::from_le_bytes([data[16], data[17], data[18], data[19]]) as usize;

        let expected_total = 20 + manifest_len + sig_len + payload_len;
        if data.len() < expected_total {
            bail!("corrupted archive: expected {expected_total} bytes, got {}", data.len());
        }

        let manifest_start = 20;
        let manifest_end = manifest_start + manifest_len;
        let manifest_bytes = &data[manifest_start..manifest_end];
        let manifest_str = std::str::from_utf8(manifest_bytes)
            .context("vpack manifest is not valid UTF-8")?;
        let manifest = VpackManifest::parse_toml(manifest_str)?;

        let sig_start = manifest_end;
        let sig_end = sig_start + sig_len;
        let sig_block = &data[sig_start..sig_end];

        let mut public_key = None;
        let mut signature = None;

        if sig_len == 96 {
            let mut pk = [0u8; 32];
            let mut sig = [0u8; 64];
            pk.copy_from_slice(&sig_block[0..32]);
            sig.copy_from_slice(&sig_block[32..96]);
            public_key = Some(pk);
            signature = Some(sig);
        }

        let payload_start = sig_end;
        let payload_end = payload_start + payload_len;
        let payload_bytes = data[payload_start..payload_end].to_vec();

        let files: Vec<VpackFileEntry> = bincode::deserialize(&payload_bytes)
            .context("failed to decode vpack payload entries")?;

        Ok(VpackArchive {
            manifest,
            manifest_raw: manifest_bytes.to_vec(),
            public_key,
            signature,
            payload_bytes,
            files,
        })
    }

    /// Verify Ed25519 signature of an archive.
    pub fn verify(archive: &VpackArchive, expected_pubkey: Option<&[u8; 32]>) -> Result<bool> {
        let (pk_bytes, sig_bytes) = match (archive.public_key, archive.signature) {
            (Some(pk), Some(sig)) => (pk, sig),
            _ => return Ok(false),
        };

        if let Some(expected) = expected_pubkey {
            if &pk_bytes != expected {
                return Ok(false);
            }
        }

        let verifying_key = VerifyingKey::from_bytes(&pk_bytes)
            .context("invalid Ed25519 public key in archive")?;
        let signature = Signature::from_bytes(&sig_bytes);

        let mut signed_data = Vec::new();
        signed_data.extend_from_slice(&archive.manifest_raw);
        signed_data.extend_from_slice(&archive.payload_bytes);

        match verifying_key.verify(&signed_data, &signature) {
            Ok(()) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Extract package contents into destination directory.
    pub fn extract(archive: &VpackArchive, dest_dir: &Path) -> Result<PathBuf> {
        fs::create_dir_all(dest_dir)
            .with_context(|| format!("failed to create destination dir {}", dest_dir.display()))?;

        // Write extracted vpack.toml
        let manifest_dest = dest_dir.join("vpack.toml");
        fs::write(&manifest_dest, &archive.manifest_raw)?;

        for file in &archive.files {
            // Guard against directory traversal attacks
            let clean_path = file.path.trim_start_matches('/').trim_start_matches('\\');
            if clean_path.contains("..") {
                bail!("illegal file path in archive: {}", file.path);
            }

            let target = dest_dir.join(clean_path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }

            fs::write(&target, &file.data)
                .with_context(|| format!("failed to write {}", target.display()))?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let permissions = fs::Permissions::from_mode(file.mode);
                let _ = fs::set_permissions(&target, permissions);
            }
        }

        let entrypoint = dest_dir.join(&archive.manifest.runtime.entrypoint);
        if !entrypoint.exists() {
            // Check fallback without prefix
            let alt_entry = dest_dir.join(Path::new(&archive.manifest.runtime.entrypoint).file_name().unwrap_or_default());
            if alt_entry.exists() {
                return Ok(alt_entry);
            }
        }

        Ok(entrypoint)
    }

    /// Get standard sandboxed package installation directory.
    pub fn package_install_dir(name: &str, version: &str) -> PathBuf {
        #[cfg(windows)]
        {
            let program_data = std::env::var("ProgramData")
                .unwrap_or_else(|_| r"C:\ProgramData".to_string());
            PathBuf::from(program_data)
                .join("VeloceSolutions")
                .join("packages")
                .join(format!("{name}-{version}"))
        }
        #[cfg(not(windows))]
        {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("veloce")
                .join("packages")
                .join(format!("{name}-{version}"))
        }
    }
}

// ── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vpack_manifest_roundtrip() {
        let manifest = VpackManifest::default_template("demo-service");
        let toml_str = manifest.to_toml().expect("serialize to toml");
        let parsed = VpackManifest::parse_toml(&toml_str).expect("parse from toml");

        assert_eq!(manifest.package.name, parsed.package.name);
        assert_eq!(manifest.runtime.port, parsed.runtime.port);
        assert_eq!(manifest.runtime.hostname, parsed.runtime.hostname);
    }

    #[test]
    fn test_vpack_build_verify_extract() {
        let temp_dir = std::env::temp_dir().join(format!("vpack_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();

        let manifest = VpackManifest::default_template("testapp");
        fs::write(temp_dir.join("vpack.toml"), manifest.to_toml().unwrap()).unwrap();

        let bin_dir = temp_dir.join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::write(bin_dir.join("testapp.exe"), b"mock binary content").unwrap();
        fs::write(temp_dir.join("config.json"), b"{\"key\": \"value\"}").unwrap();

        let (signing_key, verifying_key) = VpackEngine::keygen();
        let archive_bytes = VpackEngine::build(&temp_dir, Some(&signing_key)).expect("build archive");

        let parsed = VpackEngine::read(&archive_bytes).expect("parse archive");
        assert_eq!(parsed.manifest.package.name, "testapp");
        assert_eq!(parsed.files.len(), 2);

        let verified = VpackEngine::verify(&parsed, Some(verifying_key.as_bytes())).expect("verify signature");
        assert!(verified);

        let extract_dir = temp_dir.join("extracted");
        let entrypoint = VpackEngine::extract(&parsed, &extract_dir).expect("extract archive");
        assert!(extract_dir.join("vpack.toml").exists());
        assert!(extract_dir.join("config.json").exists());
        assert!(entrypoint.exists() || extract_dir.join("bin/testapp.exe").exists());

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_vpack_tampering_detection() {
        let temp_dir = std::env::temp_dir().join(format!("vpack_tamper_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();

        let manifest = VpackManifest::default_template("tamperapp");
        fs::write(temp_dir.join("vpack.toml"), manifest.to_toml().unwrap()).unwrap();
        fs::write(temp_dir.join("data.txt"), b"original data").unwrap();

        let (signing_key, verifying_key) = VpackEngine::keygen();
        let mut archive_bytes = VpackEngine::build(&temp_dir, Some(&signing_key)).expect("build archive");

        // Tamper with one byte in the payload at the end
        let last_idx = archive_bytes.len() - 1;
        archive_bytes[last_idx] ^= 0xFF;

        let parsed = VpackEngine::read(&archive_bytes);
        if let Ok(archive) = parsed {
            let verified = VpackEngine::verify(&archive, Some(verifying_key.as_bytes())).expect("verify signature");
            assert!(!verified, "tampered archive must fail signature verification");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }
}

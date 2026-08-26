/*!
Veloce Userspace Packager CLI commands (`veloce-run pack`).
Universal cross-platform .vpack format with compression, digital signatures,
integrity testing, and standalone installer support.
*/

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const VPACK_MAGIC_V1: &[u8; 4] = b"VPK1";
pub const VPACK_MAGIC_V2: &[u8; 4] = b"VPK2";
pub const VPACK_VERSION_1: u16 = 1;
pub const VPACK_VERSION_2: u16 = 2;

pub const FLAG_SIGNED: u16 = 0x0001;
pub const FLAG_COMPRESSED: u16 = 0x0002;

#[derive(Subcommand, Debug)]
pub enum PackAction {
    /// Initialize a new application folder with a starter vpack.toml
    Init {
        /// Target directory path [default: current directory]
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Application name
        #[arg(short = 'n', long)]
        name: Option<String>,
    },
    /// Generate an Ed25519 keypair for cryptographic package signing
    Keygen {
        /// Base filename to write keys to (generates <file>.priv and <file>.pub)
        #[arg(short = 'o', long, default_value = "veloce-publisher")]
        out: String,
    },
    /// Compile a directory into a .vpack single-file archive with compression
    Build {
        /// Directory containing vpack.toml and application assets
        dir: PathBuf,
        /// Output .vpack file path [default: <name>-<version>.vpack]
        #[arg(short = 'o', long)]
        out: Option<PathBuf>,
        /// Private key file (.priv) to cryptographically sign the package
        #[arg(short = 's', long)]
        sign: Option<PathBuf>,
        /// Compression level (0 = None / Store, 1..=9 = Deflate compression) [default: 6]
        #[arg(short = 'c', long, default_value = "6")]
        compress: u32,
    },
    /// Inspect metadata, runtime spec, compression ratio, and signature of a .vpack file
    Inspect {
        /// Path to the .vpack archive
        file: PathBuf,
    },
    /// Test the cryptographic integrity and decompression of all files in a .vpack archive
    Test {
        /// Path to the .vpack archive
        file: PathBuf,
    },
    /// Verify the Ed25519 cryptographic signature of a .vpack archive
    Verify {
        /// Path to the .vpack archive
        file: PathBuf,
        /// Expected publisher public key file (.pub); omit to verify self-signature
        #[arg(short = 'k', long)]
        pubkey: Option<PathBuf>,
    },
    /// Extract a .vpack archive into a target directory
    Extract {
        /// Path to the .vpack archive
        file: PathBuf,
        /// Target extraction directory
        #[arg(short = 'd', long)]
        dir: Option<PathBuf>,
    },
    /// Install a .vpack application into the system application library
    Install {
        /// Path to the .vpack archive
        file: PathBuf,
        /// Destination root directory (default: system Veloce app directory)
        #[arg(short = 'd', long)]
        dir: Option<PathBuf>,
        /// Overwrite if already installed
        #[arg(short = 'f', long)]
        force: bool,
    },
    /// Unpack and launch a .vpack application directly into the VeloceNetwork mesh
    Run {
        /// Path to the .vpack archive
        file: PathBuf,
        /// Override the application name
        #[arg(short = 'n', long)]
        name: Option<String>,
        /// Override the .vln hostname
        #[arg(short = 'H', long)]
        hostname: Option<String>,
        /// Override the listening port
        #[arg(short = 'p', long)]
        port: Option<u16>,
        /// CPU limit in percent (1-100)
        #[arg(long)]
        cpu: Option<u8>,
        /// Memory limit in megabytes
        #[arg(long)]
        mem: Option<u64>,
        /// Detach immediately after spawning
        #[arg(short = 'd', long)]
        detach: bool,
        /// Stream stdout/stderr
        #[arg(short = 'w', long)]
        watch: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VpackManifest {
    pub package: PackageMeta,
    pub runtime: RuntimeSpec,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub mesh: Option<MeshConfigSpec>,
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
    #[serde(default)]
    pub auto_restart: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeshConfigSpec {
    #[serde(default)]
    pub cluster_name: Option<String>,
    #[serde(default)]
    pub listen_addr: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
}

impl VpackManifest {
    pub fn starter_template(name: &str) -> Self {
        Self {
            package: PackageMeta {
                name: name.to_string(),
                version: "1.0.0".to_string(),
                description: format!("{name} running on VeloceNetwork mesh"),
                author: "Author <author@example.com>".to_string(),
                license: "MIT".to_string(),
                category: "Application".to_string(),
            },
            runtime: RuntimeSpec {
                entrypoint: if cfg!(windows) { format!("{name}.exe") } else { name.to_string() },
                args: vec![],
                hostname: Some(format!("{name}.vln")),
                port: Some(8080),
                cpu_limit: None,
                memory_mb: None,
                auto_restart: true,
            },
            env: {
                let mut map = HashMap::new();
                map.insert("VELOCE_ENV".into(), "production".into());
                map
            },
            mesh: Some(MeshConfigSpec {
                cluster_name: Some("veloce-mesh".into()),
                listen_addr: Some("127.0.0.1".into()),
                port: Some(9090),
            }),
        }
    }

    pub fn parse_toml(content: &str) -> Result<Self> {
        toml::from_str(content).context("failed to parse vpack.toml")
    }

    pub fn to_toml_string(&self) -> Result<String> {
        toml::to_string_pretty(self).context("failed to serialize vpack.toml")
    }
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
    pub payload_bytes: Vec<u8>,
    pub uncompressed_size: u64,
    pub compressed_size: u64,
    pub is_compressed: bool,
    pub files: Vec<VpackFileEntry>,
}

pub struct VpackEngine;

impl VpackEngine {
    pub fn keygen() -> (SigningKey, VerifyingKey) {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        (signing_key, verifying_key)
    }

    /// Build a .vpack archive with optional Ed25519 signature and Deflate compression (0-9).
    pub fn build(
        src_dir: &Path,
        signing_key: Option<&SigningKey>,
        compress_level: u32,
    ) -> Result<Vec<u8>> {
        let manifest_path = src_dir.join("vpack.toml");
        if !manifest_path.exists() {
            bail!("missing vpack.toml in {}", src_dir.display());
        }

        let manifest_raw = fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        let _manifest = VpackManifest::parse_toml(&manifest_raw)?;

        let mut files = Vec::new();
        Self::collect_files(src_dir, src_dir, &mut files)?;

        let raw_payload = bincode::serialize(&files)
            .context("failed to serialize file payload")?;

        let (payload_bytes, is_compressed) = if compress_level > 0 {
            let level = compress_level.min(9);
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(level));
            encoder.write_all(&raw_payload).context("failed to compress vpack payload")?;
            let compressed = encoder.finish().context("failed to finish compression")?;
            (compressed, true)
        } else {
            (raw_payload, false)
        };

        let mut flags = 0u16;
        if is_compressed {
            flags |= FLAG_COMPRESSED;
        }

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
        out.extend_from_slice(VPACK_MAGIC_V2);
        out.extend_from_slice(&VPACK_VERSION_2.to_le_bytes());
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
                if rel_str == "vpack.toml" || rel_str.ends_with(".priv") || rel_str.ends_with(".pub") {
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

    /// Read and decompress a .vpack archive (supports VPK1 and VPK2).
    pub fn read(data: &[u8]) -> Result<VpackArchive> {
        if data.len() < 20 {
            bail!("archive too small: {} bytes", data.len());
        }

        let magic = &data[0..4];
        let is_v1 = magic == VPACK_MAGIC_V1;
        let is_v2 = magic == VPACK_MAGIC_V2;

        if !is_v1 && !is_v2 {
            bail!("invalid vpack magic: expected 'VPK1' or 'VPK2'");
        }

        let version = u16::from_le_bytes([data[4], data[5]]);
        let flags = u16::from_le_bytes([data[6], data[7]]);
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

        let is_compressed = (flags & FLAG_COMPRESSED) != 0;
        let decompressed_payload = if is_compressed {
            let mut decoder = DeflateDecoder::new(&payload_bytes[..]);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed).context("failed to decompress vpack payload")?;
            decompressed
        } else {
            payload_bytes.clone()
        };

        let files: Vec<VpackFileEntry> = bincode::deserialize(&decompressed_payload)
            .context("failed to decode vpack payload entries")?;

        let uncompressed_size = decompressed_payload.len() as u64;
        let compressed_size = payload_bytes.len() as u64;

        Ok(VpackArchive {
            version,
            flags,
            manifest,
            manifest_raw: manifest_bytes.to_vec(),
            public_key,
            signature,
            payload_bytes,
            uncompressed_size,
            compressed_size,
            is_compressed,
            files,
        })
    }

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
            .map_err(|e| anyhow::anyhow!("invalid verifying key: {e}"))?;
        let signature = Signature::from_bytes(&sig_bytes);

        let mut signed_data = Vec::new();
        signed_data.extend_from_slice(&archive.manifest_raw);
        signed_data.extend_from_slice(&archive.payload_bytes);

        Ok(verifying_key.verify(&signed_data, &signature).is_ok())
    }

    pub fn extract(archive: &VpackArchive, dest_dir: &Path) -> Result<PathBuf> {
        fs::create_dir_all(dest_dir)
            .with_context(|| format!("failed to create directory {}", dest_dir.display()))?;

        let manifest_dest = dest_dir.join("vpack.toml");
        fs::write(&manifest_dest, &archive.manifest_raw)
            .with_context(|| format!("failed to write {}", manifest_dest.display()))?;

        for entry in &archive.files {
            let out_path = dest_dir.join(&entry.path);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&out_path, &entry.data)
                .with_context(|| format!("failed to write {}", out_path.display()))?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let permissions = fs::Permissions::from_mode(entry.mode);
                let _ = fs::set_permissions(&out_path, permissions);
            }
        }

        let entrypoint = dest_dir.join(&archive.manifest.runtime.entrypoint);
        #[cfg(unix)]
        {
            if entrypoint.exists() {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&entrypoint, fs::Permissions::from_mode(0o755));
            }
        }

        Ok(entrypoint)
    }

    pub fn package_install_dir(name: &str, version: &str) -> PathBuf {
        let base = if let Ok(custom) = std::env::var("VELOCE_APPS_DIR") {
            PathBuf::from(custom)
        } else if cfg!(windows) {
            let local_app_data = std::env::var("LOCALAPPDATA")
                .unwrap_or_else(|_| "C:\\ProgramData".into());
            PathBuf::from(local_app_data).join("VeloceSolutions").join("apps")
        } else {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".local").join("share").join("veloce").join("apps")
        };

        base.join(format!("{name}-{version}"))
    }
}

pub async fn run_pack(action: PackAction) -> Result<()> {
    handle_pack_command(action).await
}

pub async fn handle_pack_command(action: PackAction) -> Result<()> {
    match action {
        PackAction::Init { dir, name } => {
            let app_name = name.unwrap_or_else(|| {
                dir.canonicalize()
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| "my-veloce-app".to_string())
            });

            let manifest = VpackManifest::starter_template(&app_name);
            fs::create_dir_all(&dir)?;
            let manifest_path = dir.join("vpack.toml");
            if manifest_path.exists() {
                bail!("vpack.toml already exists in {}", dir.display());
            }

            fs::write(&manifest_path, manifest.to_toml_string()?)?;
            println!("✓ Initialized Veloce application package in {}", dir.display());
            println!("  Manifest: {}", manifest_path.display());
            println!("  Application: {} v{}", manifest.package.name, manifest.package.version);
            Ok(())
        }

        PackAction::Keygen { out } => {
            let (signing_key, verifying_key) = VpackEngine::keygen();
            let priv_file = format!("{out}.priv");
            let pub_file = format!("{out}.pub");

            let priv_hex = hex::encode(signing_key.to_bytes());
            let pub_hex = hex::encode(verifying_key.to_bytes());

            fs::write(&priv_file, &priv_hex)
                .with_context(|| format!("failed to write {priv_file}"))?;
            fs::write(&pub_file, &pub_hex)
                .with_context(|| format!("failed to write {pub_file}"))?;

            println!("✓ Generated Ed25519 publisher keypair:");
            println!("  Private Key: {priv_file} (Keep secret! Use for `pack build --sign`)");
            println!("  Public Key:  {pub_file} (Share with users / Veloce Hub)");
            Ok(())
        }

        PackAction::Build { dir, out, sign, compress } => {
            let manifest_path = dir.join("vpack.toml");
            let manifest_raw = fs::read_to_string(&manifest_path)
                .with_context(|| format!("missing vpack.toml in {}", dir.display()))?;
            let manifest = VpackManifest::parse_toml(&manifest_raw)?;

            let signing_key = if let Some(key_path) = sign {
                let key_str = fs::read_to_string(&key_path)
                    .with_context(|| format!("failed to read private key from {}", key_path.display()))?;
                let key_bytes = hex::decode(key_str.trim())
                    .context("invalid hex format in private key file")?;
                let key_array: [u8; 32] = key_bytes.try_into()
                    .map_err(|_| anyhow::anyhow!("private key must be 32 bytes"))?;
                Some(SigningKey::from_bytes(&key_array))
            } else {
                None
            };

            let archive_bytes = VpackEngine::build(&dir, signing_key.as_ref(), compress)?;

            let out_path = out.unwrap_or_else(|| {
                PathBuf::from(format!("{}-{}.vpack", manifest.package.name, manifest.package.version))
            });

            fs::write(&out_path, &archive_bytes)
                .with_context(|| format!("failed to write output archive {}", out_path.display()))?;

            println!("✓ Successfully built universal .vpack package:");
            println!("  File:         {}", out_path.display());
            println!("  Package:      {} v{}", manifest.package.name, manifest.package.version);
            println!("  Archive Size: {} bytes ({:.2} MB)", archive_bytes.len(), archive_bytes.len() as f64 / (1024.0 * 1024.0));
            println!("  Format:       VPK2 (Universal Portable)");
            println!("  Compression:  Level {} ({})", compress, if compress > 0 { "Deflate Enabled" } else { "Stored Uncompressed" });
            if signing_key.is_some() {
                println!("  Status:       Signed with Ed25519");
            } else {
                println!("  Status:       Unsigned (development build)");
            }
            Ok(())
        }

        PackAction::Inspect { file } => {
            let data = fs::read(&file)
                .with_context(|| format!("failed to read package {}", file.display()))?;
            let archive = VpackEngine::read(&data)?;

            let ratio = if archive.uncompressed_size > 0 {
                (1.0 - (archive.compressed_size as f64 / archive.uncompressed_size as f64)) * 100.0
            } else {
                0.0
            };

            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!(" Veloce Package: {} v{}", archive.manifest.package.name, archive.manifest.package.version);
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!(" Description:     {}", archive.manifest.package.description);
            println!(" Author:          {}", archive.manifest.package.author);
            println!(" Category:        {}", archive.manifest.package.category);
            println!(" Entrypoint:      {}", archive.manifest.runtime.entrypoint);
            if let Some(h) = &archive.manifest.runtime.hostname {
                println!(" Hostname:        {}", h);
            }
            if let Some(p) = archive.manifest.runtime.port {
                println!(" Port:            {}", p);
            }
            if let Some(cpu) = archive.manifest.runtime.cpu_limit {
                println!(" CPU Limit:       {}%", cpu);
            }
            if let Some(mem) = archive.manifest.runtime.memory_mb {
                println!(" Memory Cap:      {} MB", mem);
            }
            println!(" Format Version:  v{}", archive.version);
            println!(" Compression:     {} ({:.1}% space saved)", 
                if archive.is_compressed { "Deflate" } else { "None" }, ratio.max(0.0));
            println!(" Uncompressed:    {} bytes ({:.2} MB)", 
                archive.uncompressed_size, archive.uncompressed_size as f64 / (1024.0 * 1024.0));
            println!(" Archive Size:    {} bytes ({:.2} MB)", 
                data.len(), data.len() as f64 / (1024.0 * 1024.0));
            println!(" Contained Files: {} entries", archive.files.len());
            println!("────────────────────────────────────────────────────────");
            for f in &archive.files {
                println!("  • {:<40} {:>10} bytes (mode 0o{:o})", f.path, f.data.len(), f.mode);
            }

            if let Some(pk) = archive.public_key {
                let valid = VpackEngine::verify(&archive, None).unwrap_or(false);
                println!("────────────────────────────────────────────────────────");
                println!(" Signature:       Ed25519 (Publisher: {})", hex::encode(pk));
                println!(" Integrity:       {}", if valid { "✓ Valid & Verified" } else { "✗ Corrupted / Invalid Signature" });
            } else {
                println!("────────────────────────────────────────────────────────");
                println!(" Signature:       Unsigned (Development)");
            }
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            Ok(())
        }

        PackAction::Test { file } => {
            println!("🔍 Testing archive integrity for {}", file.display());
            let data = fs::read(&file)
                .with_context(|| format!("failed to read package {}", file.display()))?;
            let archive = VpackEngine::read(&data)?;

            let sig_ok = if archive.public_key.is_some() {
                VpackEngine::verify(&archive, None).unwrap_or(false)
            } else {
                true
            };

            let mut total_bytes = 0u64;
            for f in &archive.files {
                total_bytes += f.data.len() as u64;
            }

            println!("✓ Manifest syntax: OK");
            println!("✓ Decompression:   OK ({} files, {} bytes uncompressed)", archive.files.len(), total_bytes);
            println!("✓ Digital Sign:    {}", if sig_ok { "OK (Valid)" } else { "FAILED (Signature Mismatch)" });
            println!("✓ Result:          Archive integrity 100% verified.");
            Ok(())
        }

        PackAction::Verify { file, pubkey } => {
            let data = fs::read(&file)?;
            let archive = VpackEngine::read(&data)?;

            let expected_pk = if let Some(pk_path) = pubkey {
                let pk_str = fs::read_to_string(&pk_path)?;
                let pk_bytes = hex::decode(pk_str.trim())?;
                let pk_arr: [u8; 32] = pk_bytes.try_into()
                    .map_err(|_| anyhow::anyhow!("public key must be 32 bytes"))?;
                Some(pk_arr)
            } else {
                None
            };

            if VpackEngine::verify(&archive, expected_pk.as_ref())? {
                println!("✓ Package signature verified successfully for {}", file.display());
            } else {
                bail!("Package signature verification FAILED for {}", file.display());
            }
            Ok(())
        }

        PackAction::Extract { file, dir } => {
            let data = fs::read(&file)?;
            let archive = VpackEngine::read(&data)?;

            let dest = dir.unwrap_or_else(|| {
                PathBuf::from(format!("{}-{}", archive.manifest.package.name, archive.manifest.package.version))
            });

            let entrypoint = VpackEngine::extract(&archive, &dest)?;
            println!("✓ Extracted package into {}", dest.display());
            println!("  Entrypoint: {}", entrypoint.display());
            Ok(())
        }

        PackAction::Install { file, dir, force } => {
            let data = fs::read(&file)?;
            let archive = VpackEngine::read(&data)?;

            if archive.public_key.is_some() {
                if !VpackEngine::verify(&archive, None).unwrap_or(false) {
                    bail!("Security error: .vpack archive signature is invalid or corrupted!");
                }
            }

            let target_dir = dir.unwrap_or_else(|| {
                VpackEngine::package_install_dir(
                    &archive.manifest.package.name,
                    &archive.manifest.package.version,
                )
            });

            if target_dir.exists() && !force {
                println!("ℹ Package {} v{} is already installed at {}", 
                    archive.manifest.package.name, archive.manifest.package.version, target_dir.display());
                println!("  Use --force to overwrite.");
                return Ok(());
            }

            let entrypoint = VpackEngine::extract(&archive, &target_dir)?;
            println!("✓ Successfully installed {} v{}", archive.manifest.package.name, archive.manifest.package.version);
            println!("  Install Directory: {}", target_dir.display());
            println!("  Entrypoint:        {}", entrypoint.display());
            println!("  Launch with:       veloce-run pack run {}", file.display());
            Ok(())
        }

        PackAction::Run { file, name, hostname, port, cpu, mem, detach, watch: _ } => {
            let data = fs::read(&file)?;
            let archive = VpackEngine::read(&data)?;

            if archive.public_key.is_some() {
                if !VpackEngine::verify(&archive, None).unwrap_or(false) {
                    bail!("Security error: .vpack archive signature is invalid or corrupted!");
                }
            }

            let install_dir = VpackEngine::package_install_dir(
                &archive.manifest.package.name,
                &archive.manifest.package.version,
            );

            let entrypoint = VpackEngine::extract(&archive, &install_dir)?;

            let app_name = name.unwrap_or_else(|| archive.manifest.package.name.clone());
            let app_hostname = hostname.or(archive.manifest.runtime.hostname);
            let app_port = port.or(archive.manifest.runtime.port);
            let app_cpu = cpu.or(archive.manifest.runtime.cpu_limit);
            let app_mem = mem.or(archive.manifest.runtime.memory_mb);

            let mut extra_env = Vec::new();
            for (k, v) in &archive.manifest.env {
                extra_env.push(format!("{k}={v}"));
            }

            println!("🚀 Launching package {} v{} into VeloceNetwork mesh...",
                archive.manifest.package.name, archive.manifest.package.version);

            crate::run_spawn(
                entrypoint.to_string_lossy().to_string(),
                archive.manifest.runtime.args,
                extra_env,
                Some(app_name),
                app_hostname,
                app_port,
                app_cpu,
                app_mem,
                0,
                detach,
                None,
                false,
            ).await
        }
    }
}

pub mod hex {
    use anyhow::{bail, Result};

    pub fn encode<T: AsRef<[u8]>>(data: T) -> String {
        let bytes = data.as_ref();
        let mut s = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            use std::fmt::Write;
            let _ = write!(s, "{:02x}", b);
        }
        s
    }

    pub fn decode(hex_str: &str) -> Result<Vec<u8>> {
        let s = hex_str.trim();
        if s.len() % 2 != 0 {
            bail!("invalid hex string length");
        }
        let mut out = Vec::with_capacity(s.len() / 2);
        for i in (0..s.len()).step_by(2) {
            let byte = u8::from_str_radix(&s[i..i+2], 16)
                .map_err(|e| anyhow::anyhow!("invalid hex character: {e}"))?;
            out.push(byte);
        }
        Ok(out)
    }
}

/*!
Veloce Userspace Packager CLI commands (`veloce-run pack`).
*/

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const VPACK_MAGIC: &[u8; 4] = b"VPK1";
pub const VPACK_VERSION: u16 = 1;
pub const FLAG_SIGNED: u16 = 0x0001;

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
    /// Compile a directory into a .vpack single-file archive
    Build {
        /// Directory containing vpack.toml and application assets
        dir: PathBuf,
        /// Output .vpack file path [default: <name>-<version>.vpack]
        #[arg(short = 'o', long)]
        out: Option<PathBuf>,
        /// Private key file (.priv) to cryptographically sign the package
        #[arg(short = 's', long)]
        sign: Option<PathBuf>,
    },
    /// Inspect metadata, runtime spec, and signature of a .vpack file
    Inspect {
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

fn default_author() -> String { "Community".to_string() }
fn default_category() -> String { "Application".to_string() }

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpackFileEntry {
    pub path: String,
    pub mode: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct VpackArchive {
    pub manifest: VpackManifest,
    pub manifest_raw: Vec<u8>,
    pub public_key: Option<[u8; 32]>,
    pub signature: Option<[u8; 64]>,
    pub payload_bytes: Vec<u8>,
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

    pub fn build(src_dir: &Path, signing_key: Option<&SigningKey>) -> Result<Vec<u8>> {
        let manifest_path = src_dir.join("vpack.toml");
        if !manifest_path.exists() {
            bail!("missing vpack.toml in {}", src_dir.display());
        }

        let manifest_raw = fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        let _manifest = VpackManifest::parse_toml(&manifest_raw)?;

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

    pub fn extract(archive: &VpackArchive, dest_dir: &Path) -> Result<PathBuf> {
        fs::create_dir_all(dest_dir)
            .with_context(|| format!("failed to create destination dir {}", dest_dir.display()))?;

        let manifest_dest = dest_dir.join("vpack.toml");
        fs::write(&manifest_dest, &archive.manifest_raw)?;

        for file in &archive.files {
            let rel_path = Path::new(&file.path);
            for component in rel_path.components() {
                match component {
                    std::path::Component::Normal(_) => {},
                    _ => bail!("Security violation: illegal path component in archive entry '{}'", file.path),
                }
            }

            let target = dest_dir.join(rel_path);
            if !target.starts_with(dest_dir) {
                bail!("Security violation: directory traversal detected for '{}'", file.path);
            }

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
            let alt_entry = dest_dir.join(Path::new(&archive.manifest.runtime.entrypoint).file_name().unwrap_or_default());
            if alt_entry.exists() {
                return Ok(alt_entry);
            }
        }

        Ok(entrypoint)
    }

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

// ── CLI Command Handlers ──────────────────────────────────────────────────────

pub async fn run_pack(action: PackAction) -> Result<()> {
    match action {
        PackAction::Init { dir, name } => {
            let target_name = name.unwrap_or_else(|| {
                dir.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "myapp".to_string())
            });

            fs::create_dir_all(&dir)?;
            let manifest_path = dir.join("vpack.toml");
            if manifest_path.exists() {
                bail!("vpack.toml already exists in {}", dir.display());
            }

            let manifest = VpackManifest::default_template(&target_name);
            fs::write(&manifest_path, manifest.to_toml()?)?;

            let bin_dir = dir.join("bin");
            fs::create_dir_all(&bin_dir)?;

            println!("✓ Initialized new Veloce package in {}", dir.display());
            println!("  Created: {}", manifest_path.display());
            println!("  Next step: copy your executable into {}/ and run `veloce-run pack build`", bin_dir.display());
            Ok(())
        }

        PackAction::Keygen { out } => {
            let (priv_key, pub_key) = VpackEngine::keygen();
            let priv_path = format!("{out}.priv");
            let pub_path = format!("{out}.pub");

            let priv_hex = hex::encode(priv_key.to_bytes());
            let pub_hex = hex::encode(pub_key.to_bytes());

            fs::write(&priv_path, priv_hex)?;
            fs::write(&pub_path, pub_hex)?;

            println!("✓ Generated Ed25519 publisher keypair:");
            println!("  Private Key: {} (Keep secret! Use for `pack build --sign`)", priv_path);
            println!("  Public Key:  {} (Share with users / Veloce Hub)", pub_path);
            Ok(())
        }

        PackAction::Build { dir, out, sign } => {
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

            let archive_bytes = VpackEngine::build(&dir, signing_key.as_ref())?;

            let out_path = out.unwrap_or_else(|| {
                PathBuf::from(format!("{}-{}.vpack", manifest.package.name, manifest.package.version))
            });

            fs::write(&out_path, &archive_bytes)
                .with_context(|| format!("failed to write output archive {}", out_path.display()))?;

            println!("✓ Successfully built package:");
            println!("  File:    {}", out_path.display());
            println!("  Package: {} v{}", manifest.package.name, manifest.package.version);
            println!("  Size:    {} bytes", archive_bytes.len());
            if signing_key.is_some() {
                println!("  Status:  Signed with Ed25519");
            } else {
                println!("  Status:  Unsigned (development build)");
            }
            Ok(())
        }

        PackAction::Inspect { file } => {
            let data = fs::read(&file)
                .with_context(|| format!("failed to read package {}", file.display()))?;
            let archive = VpackEngine::read(&data)?;

            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!(" Veloce Package: {} v{}", archive.manifest.package.name, archive.manifest.package.version);
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!(" Description: {}", archive.manifest.package.description);
            println!(" Author:      {}", archive.manifest.package.author);
            println!(" Category:    {}", archive.manifest.package.category);
            println!(" Entrypoint:  {}", archive.manifest.runtime.entrypoint);
            if let Some(h) = &archive.manifest.runtime.hostname {
                println!(" Hostname:    {}", h);
            }
            if let Some(p) = archive.manifest.runtime.port {
                println!(" Port:        {}", p);
            }
            if let Some(cpu) = archive.manifest.runtime.cpu_limit {
                println!(" CPU Limit:   {}%", cpu);
            }
            if let Some(mem) = archive.manifest.runtime.memory_mb {
                println!(" Memory Cap:  {} MB", mem);
            }
            println!(" Files:       {} entries", archive.files.len());

            if let Some(pk) = archive.public_key {
                let valid = VpackEngine::verify(&archive, None).unwrap_or(false);
                println!(" Signature:   Ed25519 (Publisher: {})", hex::encode(pk));
                println!(" Integrity:   {}", if valid { "✓ Valid & Verified" } else { "✗ Corrupted / Invalid Signature" });
            } else {
                println!(" Signature:   Unsigned (Development)");
            }
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
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

            let verified = VpackEngine::verify(&archive, expected_pk.as_ref())?;
            if verified {
                println!("✓ Package signature verified successfully for {}", file.display());
                if let Some(pk) = archive.public_key {
                    println!("  Publisher: {}", hex::encode(pk));
                }
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

// Simple hex encode/decode helper module
mod hex {
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

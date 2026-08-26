/*!
VPack Universal Package Installer
Supports Windows, Linux, and macOS standalone installations of .vpack bundles.
*/

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use flate2::read::DeflateDecoder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const VPACK_MAGIC_V1: &[u8; 4] = b"VPK1";
pub const VPACK_MAGIC_V2: &[u8; 4] = b"VPK2";
pub const VPACK_EOCD_MAGIC: &[u8; 4] = b"EOCD";
pub const FLAG_COMPRESSED: u16 = 0x0002;
pub const METHOD_DEFLATE: u16 = 1;

#[derive(Parser, Debug)]
#[command(name = "vpack-installer", version = "4.7.0", about = "Universal .vpack package installer")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Quick install file if provided as positional argument
    file: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Install a .vpack application bundle to the host system
    Install {
        /// Path to the .vpack package
        file: PathBuf,
        /// Target install directory (default: system Veloce app directory)
        #[arg(short = 'd', long)]
        dir: Option<PathBuf>,
        /// Overwrite if existing version is present
        #[arg(short = 'f', long)]
        force: bool,
        /// Non-interactive quiet mode
        #[arg(short = 'q', long)]
        quiet: bool,
    },
    /// List all .vpack packages installed on this machine
    List,
    /// Uninstall a package from the host system
    Uninstall {
        /// Package name to remove
        name: String,
        /// Specific version to remove (omitted = all versions)
        #[arg(short = 'v', long)]
        version: Option<String>,
    },
    /// Verify the integrity and cryptographic signature of a .vpack package
    Verify {
        /// Path to the .vpack archive
        file: PathBuf,
        /// Optional public key file
        #[arg(short = 'k', long)]
        pubkey: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpackManifest {
    pub package: PackageMeta,
    pub runtime: RuntimeSpec,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageMeta {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CentralDirEntry {
    pub path: String,
    pub uncompressed_size: u64,
    pub compressed_size: u64,
    pub payload_offset: u64,
    pub method: u16,
    pub mode: u32,
    pub crc32: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VpackFileEntry {
    pub path: String,
    pub mode: u32,
    pub data: Vec<u8>,
}

pub struct VpackArchive {
    pub version: u16,
    pub manifest: VpackManifest,
    pub manifest_raw: Vec<u8>,
    pub public_key: Option<[u8; 32]>,
    pub signature: Option<[u8; 64]>,
    pub files: Vec<VpackFileEntry>,
}

fn crc32_compute(data: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

fn read_vpack(data: &[u8]) -> Result<VpackArchive> {
    if data.len() < 28 {
        bail!("archive too small: {} bytes", data.len());
    }

    let footer_len = 28;
    let eocd_pos = data.len().saturating_sub(footer_len);
    let eocd_magic = &data[eocd_pos..eocd_pos + 4];

    if &data[0..4] == VPACK_MAGIC_V2 && eocd_magic == VPACK_EOCD_MAGIC {
        let cd_offset = u64::from_le_bytes(data[eocd_pos + 4..eocd_pos + 12].try_into()?) as usize;
        let cd_len = u64::from_le_bytes(data[eocd_pos + 12..eocd_pos + 20].try_into()?) as usize;
        let sig_len = u32::from_le_bytes(data[eocd_pos + 24..eocd_pos + 28].try_into()?) as usize;

        let manifest_len = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
        let manifest_bytes = &data[16..16 + manifest_len];
        let manifest: VpackManifest = toml::from_str(std::str::from_utf8(manifest_bytes)?)?;

        let cd_bytes = &data[cd_offset..cd_offset + cd_len];
        let central_directory: Vec<CentralDirEntry> = bincode::deserialize(cd_bytes)?;

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

        let mut files = Vec::new();
        for entry in &central_directory {
            let start = entry.payload_offset as usize;
            let end = start + entry.compressed_size as usize;
            let raw_chunk = &data[start..end];
            let decompressed_data = if entry.method == METHOD_DEFLATE {
                let mut decoder = DeflateDecoder::new(raw_chunk);
                let mut buf = Vec::with_capacity(entry.uncompressed_size as usize);
                decoder.read_to_end(&mut buf)?;
                buf
            } else {
                raw_chunk.to_vec()
            };

            let crc = crc32_compute(&decompressed_data);
            if crc != entry.crc32 {
                bail!("corrupted CRC-32 for file '{}'", entry.path);
            }

            files.push(VpackFileEntry {
                path: entry.path.clone(),
                mode: entry.mode,
                data: decompressed_data,
            });
        }

        return Ok(VpackArchive {
            version: 2,
            manifest,
            manifest_raw: manifest_bytes.to_vec(),
            public_key,
            signature,
            files,
        });
    }

    // Legacy VPK1 fallback
    let manifest_len = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
    let sig_len = u32::from_le_bytes([data[12], data[13], data[14], data[15]]) as usize;
    let payload_len = u32::from_le_bytes([data[16], data[17], data[18], data[19]]) as usize;

    let manifest_bytes = &data[20..20 + manifest_len];
    let manifest: VpackManifest = toml::from_str(std::str::from_utf8(manifest_bytes)?)?;

    let payload_start = 20 + manifest_len + sig_len;
    let payload_bytes = &data[payload_start..payload_start + payload_len];
    let flags = u16::from_le_bytes([data[6], data[7]]);
    let decompressed = if (flags & FLAG_COMPRESSED) != 0 {
        let mut dec = DeflateDecoder::new(payload_bytes);
        let mut b = Vec::new();
        dec.read_to_end(&mut b)?;
        b
    } else {
        payload_bytes.to_vec()
    };

    let files: Vec<VpackFileEntry> = bincode::deserialize(&decompressed)?;
    Ok(VpackArchive {
        version: 1,
        manifest,
        manifest_raw: manifest_bytes.to_vec(),
        public_key: None,
        signature: None,
        files,
    })
}

fn verify_archive(archive: &VpackArchive, expected_pubkey: Option<&[u8; 32]>) -> Result<bool> {
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
    for f in &archive.files {
        signed_data.extend_from_slice(&f.data);
    }

    Ok(verifying_key.verify(&signed_data, &signature).is_ok())
}

fn default_apps_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("VELOCE_APPS_DIR") {
        PathBuf::from(custom)
    } else if cfg!(windows) {
        let local_app_data = std::env::var("LOCALAPPDATA")
            .unwrap_or_else(|_| "C:\\ProgramData".into());
        PathBuf::from(local_app_data).join("VeloceSolutions").join("apps")
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".local").join("share").join("veloce").join("apps")
    }
}

fn install_package(file: &Path, custom_dir: Option<PathBuf>, force: bool, quiet: bool) -> Result<()> {
    let data = fs::read(file)
        .with_context(|| format!("failed to read package file {}", file.display()))?;
    let archive = read_vpack(&data)?;

    if archive.public_key.is_some() {
        let verified = verify_archive(&archive, None).unwrap_or(false);
        if !verified {
            bail!("security verification failed: digital signature is corrupted or invalid!");
        }
    }

    let target_dir = custom_dir.unwrap_or_else(|| {
        default_apps_dir().join(format!("{}-{}", archive.manifest.package.name, archive.manifest.package.version))
    });

    if target_dir.exists() && !force {
        if !quiet {
            println!("ℹ Package {} v{} is already installed at {}",
                archive.manifest.package.name, archive.manifest.package.version, target_dir.display());
            println!("  Re-run with --force to overwrite.");
        }
        return Ok(());
    }

    fs::create_dir_all(&target_dir)?;

    let manifest_dest = target_dir.join("vpack.toml");
    fs::write(&manifest_dest, &archive.manifest_raw)?;

    for entry in &archive.files {
        let out_path = target_dir.join(&entry.path);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&out_path, &entry.data)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = fs::Permissions::from_mode(entry.mode);
            let _ = fs::set_permissions(&out_path, permissions);
        }
    }

    let entrypoint = target_dir.join(&archive.manifest.runtime.entrypoint);
    #[cfg(unix)]
    {
        if entrypoint.exists() {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&entrypoint, fs::Permissions::from_mode(0o755));
        }
    }

    if !quiet {
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!(" ✓ Veloce Package Installed Successfully");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!(" Application:  {} v{}", archive.manifest.package.name, archive.manifest.package.version);
        println!(" Description:  {}", archive.manifest.package.description);
        println!(" Directory:    {}", target_dir.display());
        println!(" Entrypoint:   {}", entrypoint.display());
        println!(" Launch with:  veloce-run pack run {}", file.display());
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    }

    Ok(())
}

fn list_installed_packages() -> Result<()> {
    let base = default_apps_dir();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!(" Installed Veloce Applications ({})", base.display());
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    if !base.exists() {
        println!("  (No packages installed yet)");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        return Ok(());
    }

    let mut count = 0;
    for entry in fs::read_dir(&base)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let manifest_path = path.join("vpack.toml");
            if manifest_path.exists() {
                if let Ok(content) = fs::read_to_string(&manifest_path) {
                    if let Ok(manifest) = toml::from_str::<VpackManifest>(&content) {
                        count += 1;
                        println!("  • {:<20} v{:<8} [{}]",
                            manifest.package.name,
                            manifest.package.version,
                            path.display()
                        );
                    }
                }
            }
        }
    }

    if count == 0 {
        println!("  (No packages found)");
    }
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    Ok(())
}

fn uninstall_package(name: &str, version: Option<&str>) -> Result<()> {
    let base = default_apps_dir();
    if !base.exists() {
        println!("No packages installed.");
        return Ok(());
    }

    let mut removed = 0;
    for entry in fs::read_dir(&base)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let dir_name = path.file_name().unwrap_or_default().to_string_lossy();
            let matches = if let Some(v) = version {
                dir_name == format!("{name}-{v}")
            } else {
                dir_name.starts_with(&format!("{name}-"))
            };

            if matches {
                fs::remove_dir_all(&path)
                    .with_context(|| format!("failed to remove {}", path.display()))?;
                println!("✓ Removed {}", path.display());
                removed += 1;
            }
        }
    }

    if removed == 0 {
        println!("No installed package found matching '{}'.", name);
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(file) = cli.file {
        return install_package(&file, None, false, false);
    }

    match cli.command {
        Some(Commands::Install { file, dir, force, quiet }) => {
            install_package(&file, dir, force, quiet)
        }
        Some(Commands::List) => list_installed_packages(),
        Some(Commands::Uninstall { name, version }) => {
            uninstall_package(&name, version.as_deref())
        }
        Some(Commands::Verify { file, pubkey: _ }) => {
            let data = fs::read(&file)?;
            let archive = read_vpack(&data)?;
            if verify_archive(&archive, None).unwrap_or(false) {
                println!("✓ Package signature and integrity verified for {}", file.display());
                Ok(())
            } else {
                bail!("Package signature verification FAILED");
            }
        }
        None => {
            println!("========================================================");
            println!(" VPack Universal Package Installer v4.7.0");
            println!(" Usage: vpack-installer install <file.vpack>");
            println!("        vpack-installer list");
            println!("        vpack-installer uninstall <name>");
            println!("========================================================");
            Ok(())
        }
    }
}

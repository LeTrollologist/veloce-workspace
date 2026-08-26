/*!
VPack Universal Archive Manager (WinRAR / 7-Zip for .vpack)
Command Line Interface & Terminal Explorer
*/

mod archive;
mod bench;
mod tui;
mod verify;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use ed25519_dalek::SigningKey;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Parser, Debug)]
#[command(
    name = "vpack",
    version = "1.1.0",
    about = "VPack Archiver - The universal WinRAR & 7-Zip equivalent for .vpack archives"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Quick inspect or open archive in WinRAR table view
    archive: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// [a] Add files and directories into a compressed .vpack archive
    #[command(alias = "a")]
    Add {
        /// Target .vpack archive path
        archive: PathBuf,
        /// Input files or directories to compress
        files: Vec<PathBuf>,
        /// Compression level (0 = Store, 1..=9 = Deflate) [default: 6]
        #[arg(short = 'c', long, default_value = "6")]
        level: u32,
        /// Password protect / encrypt archive
        #[arg(short = 'p', long)]
        password: Option<String>,
        /// Optional archive comment
        #[arg(short = 'm', long)]
        comment: Option<String>,
        /// Private key (.priv) to digitally sign archive
        #[arg(short = 's', long)]
        sign: Option<PathBuf>,
    },
    /// [x] eXtract all files with full directory paths
    #[command(alias = "x")]
    Extract {
        /// Target .vpack archive path
        archive: PathBuf,
        /// Destination directory
        #[arg(short = 'o', long)]
        dest: Option<PathBuf>,
        /// Password for encrypted archive
        #[arg(short = 'p', long)]
        password: Option<String>,
    },
    /// [e] Extract a single file in O(1) time without unpacking whole archive
    #[command(alias = "e")]
    ExtractFile {
        /// Target .vpack archive path
        archive: PathBuf,
        /// Path of file inside archive
        file_inside: String,
        /// Destination output file path
        #[arg(short = 'o', long)]
        out: Option<PathBuf>,
        /// Password for encrypted archive
        #[arg(short = 'p', long)]
        password: Option<String>,
    },
    /// [l] List contents in a rich WinRAR-style attribute table
    #[command(alias = "l")]
    List {
        /// Target .vpack archive path
        archive: PathBuf,
    },
    /// [t] Test CRC-32 integrity and verify signatures
    #[command(alias = "t")]
    Test {
        /// Target .vpack archive path
        archive: PathBuf,
        /// Password for encrypted archive
        #[arg(short = 'p', long)]
        password: Option<String>,
    },
    /// [v] View/preview a file from the archive to stdout
    #[command(alias = "v")]
    View {
        /// Target .vpack archive path
        archive: PathBuf,
        /// Path of file inside archive
        file_inside: String,
        /// Password for encrypted archive
        #[arg(short = 'p', long)]
        password: Option<String>,
    },
    /// [b] Multi-core compression and decompression speed benchmark
    #[command(alias = "b")]
    Bench {
        /// Size in megabytes for synthetic workload [default: 32]
        #[arg(short = 'm', long, default_value = "32")]
        size_mb: usize,
    },
    /// Generate an Ed25519 publisher keypair (.priv / .pub)
    Keygen {
        /// Key prefix [default: vpack-publisher]
        #[arg(short = 'o', long, default_value = "vpack-publisher")]
        out: String,
    },
    /// Interactive Terminal User Interface (TUI)
    Ui {
        /// Target .vpack archive path
        archive: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(archive_path) = cli.archive {
        let archive = archive::VpackArchive::open(&archive_path)?;
        tui::render_archive_ui(&archive, &archive_path.to_string_lossy());
        return Ok(());
    }

    match cli.command {
        Some(Commands::Add { archive, files, level, password, comment, sign }) => {
            if files.is_empty() {
                bail!("no input files or directories specified");
            }

            let mut all_entries = Vec::new();
            for f in &files {
                let metadata = fs::metadata(f)?;
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

                if f.is_dir() {
                    let dir_name = f.file_name().unwrap_or_default().to_string_lossy().to_string();
                    all_entries.push(archive::ArchiveInputEntry {
                        rel_path: format!("{}/", dir_name),
                        data: Vec::new(),
                        mode,
                        modified,
                        is_dir: true,
                    });
                    for sub in archive::collect_directory_entries(f, f)? {
                        all_entries.push(archive::ArchiveInputEntry {
                            rel_path: format!("{}/{}", dir_name, sub.rel_path),
                            data: sub.data,
                            mode: sub.mode,
                            modified: sub.modified,
                            is_dir: sub.is_dir,
                        });
                    }
                } else if f.is_file() {
                    let name = f.file_name().unwrap_or_default().to_string_lossy().to_string();
                    let data = fs::read(f)?;
                    all_entries.push(archive::ArchiveInputEntry {
                        rel_path: name,
                        data,
                        mode,
                        modified,
                        is_dir: false,
                    });
                }
            }

            let sk = if let Some(key_path) = sign {
                let s = fs::read_to_string(key_path)?;
                let bytes = (0..s.trim().len()).step_by(2)
                    .map(|i| u8::from_str_radix(&s[i..i+2], 16))
                    .collect::<Result<Vec<u8>, _>>()?;
                let arr: [u8; 32] = bytes.try_into().map_err(|_| anyhow::anyhow!("invalid 32-byte key"))?;
                Some(SigningKey::from_bytes(&arr))
            } else {
                None
            };

            archive::VpackArchive::create_archive(
                &archive,
                all_entries,
                level,
                password.as_deref(),
                comment,
                sk.as_ref()
            )?;

            println!("✓ Successfully created VPack archive: {}", archive.display());
            println!("  Compression Level: {}", level);
            if password.is_some() {
                println!("  Encryption:        🔒 AES Stream Protected");
            }
            Ok(())
        }
        Some(Commands::Extract { archive, dest, password }) => {
            let a = archive::VpackArchive::open(&archive)?;
            let out_dir = dest.unwrap_or_else(|| {
                PathBuf::from(format!("{}_extracted", archive.file_stem().unwrap_or_default().to_string_lossy()))
            });
            let count = a.extract_all(&out_dir, password.as_deref())?;
            println!("✓ Extracted {} files into {}", count, out_dir.display());
            Ok(())
        }
        Some(Commands::ExtractFile { archive, file_inside, out, password }) => {
            let a = archive::VpackArchive::open(&archive)?;
            let data = a.extract_file(&file_inside, password.as_deref())?;
            let out_path = out.unwrap_or_else(|| {
                PathBuf::from(Path::new(&file_inside).file_name().unwrap_or_default())
            });
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&out_path, data)?;
            println!("✓ Extracted '{}' -> {}", file_inside, out_path.display());
            Ok(())
        }
        Some(Commands::List { archive }) => {
            let a = archive::VpackArchive::open(&archive)?;
            tui::render_archive_ui(&a, &archive.to_string_lossy());
            Ok(())
        }
        Some(Commands::Test { archive, password }) => {
            let a = archive::VpackArchive::open(&archive)?;
            let count = a.test_integrity(password.as_deref())?;
            println!("✓ Integrity Test PASSED: {} files verified with CRC-32 & EOF index", count);
            Ok(())
        }
        Some(Commands::View { archive, file_inside, password }) => {
            let a = archive::VpackArchive::open(&archive)?;
            let data = a.extract_file(&file_inside, password.as_deref())?;
            std::io::stdout().write_all(&data)?;
            Ok(())
        }
        Some(Commands::Bench { size_mb }) => bench::run_benchmark(size_mb),
        Some(Commands::Keygen { out }) => {
            let mut csprng = rand::rngs::OsRng;
            let signing_key = SigningKey::generate(&mut csprng);
            let verifying_key = signing_key.verifying_key();
            let priv_file = format!("{out}.priv");
            let pub_file = format!("{out}.pub");

            let priv_hex = signing_key.to_bytes().iter().map(|b| format!("{:02x}", b)).collect::<String>();
            let pub_hex = verifying_key.to_bytes().iter().map(|b| format!("{:02x}", b)).collect::<String>();

            fs::write(&priv_file, priv_hex)?;
            fs::write(&pub_file, pub_hex)?;

            println!("✓ Generated Ed25519 Publisher Keypair:");
            println!("  Private Key: {}", priv_file);
            println!("  Public Key:  {}", pub_file);
            Ok(())
        }
        Some(Commands::Ui { archive }) => {
            let a = archive::VpackArchive::open(&archive)?;
            tui::render_archive_ui(&a, &archive.to_string_lossy());
            Ok(())
        }
        None => {
            println!("========================================================");
            println!(" 🗁 VPack Archiver (WinRAR for .vpack) v1.1.0");
            println!("========================================================");
            println!(" Commands:");
            println!("   vpack <archive.vpack>             Open & Inspect (WinRAR UI)");
            println!("   vpack a <archive> <files...>      Add / Compress files");
            println!("   vpack x <archive> [-o <dest>]     Extract all files");
            println!("   vpack e <archive> <file>          Extract single file in O(1)");
            println!("   vpack l <archive>                 List archive contents");
            println!("   vpack t <archive>                 Test archive integrity");
            println!("   vpack v <archive> <file>          View file to stdout");
            println!("   vpack b [-m <size_mb>]            Run CPU speed benchmark");
            println!("   vpack keygen [-o <prefix>]        Generate signing keypair");
            println!("========================================================");
            Ok(())
        }
    }
}

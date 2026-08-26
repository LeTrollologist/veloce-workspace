/*!
VPack Archiver CLI & GUI (WinRAR for .vpack)
*/

mod archive;
mod bench;
mod tui;
mod verify;

use anyhow::Result;
use clap::{Parser, Subcommand};
use ed25519_dalek::SigningKey;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(
    name = "vpack-archiver",
    version = "1.0.0",
    about = "VPack Archiver - The WinRAR & 7-Zip equivalent for .vpack archives"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Quick open / inspect archive if path provided
    archive: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// [a] Add files/directories into a .vpack archive with compression
    Add {
        /// Target .vpack archive path
        archive: PathBuf,
        /// Files or directories to include
        files: Vec<PathBuf>,
        /// Compression level (0 = Store, 1..=9 = Deflate) [default: 6]
        #[arg(short = 'c', long, default_value = "6")]
        level: u32,
        /// Private key file (.priv) to sign archive
        #[arg(short = 's', long)]
        sign: Option<PathBuf>,
    },
    /// [x] Extract all files with full directory tree
    Extract {
        /// Target .vpack archive
        archive: PathBuf,
        /// Destination directory
        #[arg(short = 'o', long)]
        dest: Option<PathBuf>,
    },
    /// [e] Extract a single file from the archive using O(1) Central Directory seek
    ExtractFile {
        /// Target .vpack archive
        archive: PathBuf,
        /// Relative path of file inside archive
        file_inside: String,
        /// Output file path
        #[arg(short = 'o', long)]
        out: Option<PathBuf>,
    },
    /// [l] List contents of archive in WinRAR-style table
    List {
        /// Target .vpack archive
        archive: PathBuf,
    },
    /// [t] Test archive integrity, CRC32, and signature
    Test {
        /// Target .vpack archive
        archive: PathBuf,
    },
    /// [v] View/preview a file directly from archive to stdout
    View {
        /// Target .vpack archive
        archive: PathBuf,
        /// Relative path of file inside archive
        file_inside: String,
    },
    /// [b] Benchmark compression and decompression performance (WinRAR Benchmark)
    Bench {
        /// Test payload size in megabytes [default: 32]
        #[arg(short = 'm', long, default_value = "32")]
        size_mb: usize,
    },
    /// Generate a publisher Ed25519 keypair (.priv / .pub)
    Keygen {
        /// Output key name prefix [default: vpack-publisher]
        #[arg(short = 'o', long, default_value = "vpack-publisher")]
        out: String,
    },
    /// Interactive visual archive browser (TUI)
    Ui {
        /// Target .vpack archive
        archive: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(archive_path) = cli.archive {
        let archive = archive::VpackArchive::open(&archive_path)?;
        tui::render_archive_ui(&archive);
        return Ok(());
    }

    match cli.command {
        Some(Commands::Add { archive, files, level, sign }) => {
            let mut raw_files = Vec::new();
            for f in &files {
                if f.is_dir() {
                    for entry in walk_dir(f, f)? {
                        raw_files.push(entry);
                    }
                } else if f.is_file() {
                    let rel = f.file_name().unwrap_or_default().to_string_lossy().to_string();
                    let data = fs::read(f)?;
                    raw_files.push((rel, data));
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

            archive::VpackArchive::create_archive(&archive, raw_files, level, sk.as_ref())?;
            println!("✓ Created archive {} (compression level {})", archive.display(), level);
            Ok(())
        }
        Some(Commands::Extract { archive, dest }) => {
            let a = archive::VpackArchive::open(&archive)?;
            let out_dir = dest.unwrap_or_else(|| {
                PathBuf::from(format!("{}_extracted", archive.file_stem().unwrap_or_default().to_string_lossy()))
            });
            a.extract_all(&out_dir)?;
            println!("✓ Extracted all files into {}", out_dir.display());
            Ok(())
        }
        Some(Commands::ExtractFile { archive, file_inside, out }) => {
            let a = archive::VpackArchive::open(&archive)?;
            let data = a.extract_file(&file_inside)?;
            let out_path = out.unwrap_or_else(|| PathBuf::from(Path::new(&file_inside).file_name().unwrap()));
            fs::write(&out_path, data)?;
            println!("✓ Extracted '{}' -> {}", file_inside, out_path.display());
            Ok(())
        }
        Some(Commands::List { archive }) => {
            let a = archive::VpackArchive::open(&archive)?;
            tui::render_archive_ui(&a);
            Ok(())
        }
        Some(Commands::Test { archive }) => {
            let a = archive::VpackArchive::open(&archive)?;
            let count = a.test_integrity()?;
            println!("✓ Archive integrity test PASSED ({} files verified with CRC-32 & EOF index)", count);
            Ok(())
        }
        Some(Commands::View { archive, file_inside }) => {
            let a = archive::VpackArchive::open(&archive)?;
            let data = a.extract_file(&file_inside)?;
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
            tui::render_archive_ui(&a);
            Ok(())
        }
        None => {
            println!("========================================================");
            println!(" VPack Archiver (WinRAR for .vpack) v1.0.0");
            println!(" Usage:");
            println!("   vpack-archiver <file.vpack>             Open & Inspect");
            println!("   vpack-archiver a <out.vpack> <files...> Add / Compress");
            println!("   vpack-archiver x <in.vpack> [-o <dir>]  Extract All");
            println!("   vpack-archiver e <in.vpack> <file>      Extract Single");
            println!("   vpack-archiver t <in.vpack>             Test Integrity");
            println!("   vpack-archiver b [-m <size_mb>]         Benchmark");
            println!("========================================================");
            Ok(())
        }
    }
}

fn walk_dir(base: &Path, current: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk_dir(base, &path)?);
        } else {
            let rel = path.strip_prefix(base)?;
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let data = fs::read(&path)?;
            files.push((rel_str, data));
        }
    }
    Ok(files)
}


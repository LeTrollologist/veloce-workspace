/*!
CLI command handler for Micro-Mini OS & VeloceVFS (`veloce-run os`).
*/

use anyhow::Result;
use clap::Subcommand;
use veloce_sdk::VeloceClient;

#[derive(Subcommand, Debug)]
pub enum OsCommands {
    /// Show Micro-OS kernel status, memory, and virtual mounts.
    Status,

    /// VeloceVFS virtual filesystem operations.
    Vfs {
        #[command(subcommand)]
        command: VfsSubcommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum VfsSubcommands {
    /// List directory contents in VFS.
    Ls {
        /// Virtual path (default: "/")
        #[arg(default_value = "/")]
        path: String,
    },
    /// Print file content from VFS.
    Cat {
        /// Virtual file path
        path: String,
    },
    /// Write text content directly to a VFS file.
    Write {
        /// Virtual file path
        path: String,
        /// Content string to write
        content: String,
    },
    /// Display inode metadata and permissions.
    Stat {
        /// Virtual path
        path: String,
    },
    /// Import a host file into VFS.
    Import {
        /// Path on host machine
        host_path: String,
        /// Target virtual path in VFS
        vfs_path: String,
    },
    /// Export a VFS file to the host filesystem.
    Export {
        /// Virtual file path in VFS
        vfs_path: String,
        /// Target path on host machine
        host_path: String,
    },
    /// Format and reinitialize VeloceVFS with standard POSIX layout.
    Format {
        /// Optional volume name
        #[arg(long)]
        name: Option<String>,
    },
}

pub async fn handle_os_command(cmd: OsCommands, mut client: VeloceClient) -> Result<()> {
    match cmd {
        OsCommands::Status => {
            let status = client.os_status().await?;
            println!("\n🖥️  VeloceOS Userspace Micro-Mini OS Status");
            println!("────────────────────────────────────────────────────────────");
            println!("  OS Name:            {}", status.os_name);
            println!("  Kernel Version:     {}", status.kernel_version);
            println!("  Uptime:             {}s", status.uptime_secs);
            println!("  Virtual Inodes:     {}", status.total_inodes);
            println!("  Used VFS Storage:   {} bytes ({:.2} KB)", status.used_vfs_bytes, status.used_vfs_bytes as f64 / 1024.0);
            println!("  Active Micro-Procs: {}", status.active_micro_procs);
            println!("\n📁 Active Virtual Mounts:");
            for mount in &status.virtual_mounts {
                println!("    • {}", mount);
            }
            println!("────────────────────────────────────────────────────────────\n");
        }

        OsCommands::Vfs { command } => match command {
            VfsSubcommands::Ls { path } => {
                let list = client.vfs_list(&path).await?;
                println!("\n📁 VeloceVFS Directory: {}", list.path);
                println!("{:<12} {:<10} {:<10} {}", "TYPE", "PERMS", "SIZE", "NAME");
                println!("────────────────────────────────────────────────────────────");
                for entry in list.entries {
                    let type_str = match entry.entry_type {
                        veloce_ipc::message::VfsEntryType::Directory => "DIR",
                        veloce_ipc::message::VfsEntryType::File => "FILE",
                        veloce_ipc::message::VfsEntryType::Symlink => "LINK",
                        veloce_ipc::message::VfsEntryType::Device => "DEV",
                        veloce_ipc::message::VfsEntryType::Proc => "PROC",
                    };
                    println!(
                        "{:<12} 0o{:<8o} {:<10} {}",
                        type_str,
                        entry.permissions,
                        format!("{} B", entry.size_bytes),
                        entry.name
                    );
                }
                println!("────────────────────────────────────────────────────────────\n");
            }

            VfsSubcommands::Cat { path } => {
                let res = client.vfs_read(&path).await?;
                print!("{}", String::from_utf8_lossy(&res.data));
            }

            VfsSubcommands::Write { path, content } => {
                let bytes_written = client.vfs_write(&path, content.into_bytes()).await?;
                println!("✓ Wrote {} bytes to {}", bytes_written, path);
            }

            VfsSubcommands::Stat { path } => {
                let stat = client.vfs_stat(&path).await?;
                println!("\n📄 Inode Stat: {}", stat.path);
                println!("  Type:        {:?}", stat.entry_type);
                println!("  Size:        {} bytes", stat.size_bytes);
                println!("  Permissions: 0o{:o}", stat.permissions);
                println!("  Modified:    {}s (UNIX timestamp)\n", stat.modified_at_secs);
            }

            VfsSubcommands::Import { host_path, vfs_path } => {
                let imported = client.vfs_import(&host_path, &vfs_path).await?;
                println!("✓ Imported {} bytes from '{}' to VFS '{}'", imported, host_path, vfs_path);
            }

            VfsSubcommands::Export { vfs_path, host_path } => {
                let exported = client.vfs_export(&vfs_path, &host_path).await?;
                println!("✓ Exported {} bytes from VFS '{}' to host '{}'", exported, vfs_path, host_path);
            }

            VfsSubcommands::Format { name } => {
                let msg = client.vfs_format(name).await?;
                println!("✓ {}", msg);
            }
        },
    }

    Ok(())
}

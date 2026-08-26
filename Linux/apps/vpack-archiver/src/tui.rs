/*!
WinRAR-style Terminal User Interface and Explorer for .vpack archives.
*/

use crate::archive::{VpackArchive, METHOD_DEFLATE};

pub fn render_archive_ui(archive: &VpackArchive) {
    println!("┌───────────────────────────────────────────────────────────────────────────────────┐");
    println!("│ 🗁 VPack Archiver (WinRAR for .vpack) - v4.7.0                                    │");
    println!("├───────────────────────────────────────────────────────────────────────────────────┤");
    println!("│ [A]dd  [E]xtract  [T]est  [V]iew  [I]nfo  [B]enchmark  [S]ign  [Q]uit             │");
    println!("├───────────────────────────────────────────────────────────────────────────────────┤");
    println!("│ Name:        {:<68} │", archive.manifest.package.name);
    println!("│ Version:     {:<68} │", archive.manifest.package.version);
    println!("│ Description: {:<68} │", archive.manifest.package.description);
    println!("│ Entrypoint:  {:<68} │", archive.manifest.runtime.entrypoint);
    println!("├───────────────────────────────────────────────────────────────────────────────────┤");
    println!("│ {:<4} {:<34} {:>10} {:>10} {:>6} {:>8} {:>8} │",
        "Attr", "File Name", "Size", "Packed", "Ratio", "CRC-32", "Method");
    println!("├───────────────────────────────────────────────────────────────────────────────────┤");

    for entry in &archive.central_directory {
        let icon = if entry.path.ends_with(".exe") || entry.path.ends_with(".dll") || entry.path.ends_with(".so") {
            "🗎"
        } else if entry.path.ends_with(".toml") || entry.path.ends_with(".json") || entry.path.ends_with(".txt") {
            "🖹"
        } else {
            "📄"
        };

        let ratio = if entry.uncompressed_size > 0 {
            (1.0 - (entry.compressed_size as f64 / entry.uncompressed_size as f64)) * 100.0
        } else {
            0.0
        };

        let method_str = if entry.method == METHOD_DEFLATE { "Deflate" } else { "Store" };

        let display_name = if entry.path.len() > 34 {
            format!("{}...", &entry.path[..31])
        } else {
            entry.path.clone()
        };

        println!("│ {:<4} {:<34} {:>10} {:>10} {:>5.0}% {:08X} {:>8} │",
            icon,
            display_name,
            entry.uncompressed_size,
            entry.compressed_size,
            ratio.max(0.0),
            entry.crc32,
            method_str
        );
    }

    println!("├───────────────────────────────────────────────────────────────────────────────────┤");
    let total_ratio = if archive.uncompressed_size > 0 {
        (1.0 - (archive.compressed_size as f64 / archive.uncompressed_size as f64)) * 100.0
    } else {
        0.0
    };

    println!("│ Total: {:>2} files | Unpacked: {:>6.2} MB | Packed: {:>6.2} MB | Savings: {:>4.1}%       │",
        archive.central_directory.len(),
        archive.uncompressed_size as f64 / (1024.0 * 1024.0),
        archive.compressed_size as f64 / (1024.0 * 1024.0),
        total_ratio.max(0.0)
    );

    if let Some(pk) = archive.public_key {
        let pk_hex = pk.iter().map(|b| format!("{:02x}", b)).collect::<String>();
        println!("│ Status: ✓ Authenticated Publisher (Ed25519: {}...{})                │", &pk_hex[..8], &pk_hex[56..]);
    } else {
        println!("│ Status: ⚠ Development Build (Unsigned)                                            │");
    }
    println!("└───────────────────────────────────────────────────────────────────────────────────┘");
}

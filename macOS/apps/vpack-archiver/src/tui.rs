/*!
WinRAR & 7-Zip Style Visual Console Explorer for VPack Archives
*/
#![allow(dead_code)]

use chrono::{Local, TimeZone};
use crate::archive::{VpackArchive, FLAG_ENCRYPTED, FLAG_SIGNED};

pub fn render_archive_ui(archive: &VpackArchive, archive_path_display: &str) {
    let term_width = 85;
    let sep = "─".repeat(term_width);
    let double_sep = "═".repeat(term_width);

    println!("╔{}╗", double_sep);
    println!("║ {:<85} ║", format!("🗁 VPack Archiver (WinRAR Edition) - {}", archive_path_display));
    println!("╠{}╣", sep);
    println!("║ [A]dd  [X]Extract  [E]xtract-Single  [T]est  [V]iew  [I]nfo  [B]enchmark  [Q]uit ║");
    println!("╠{}╣", sep);

    let mut total_orig = 0u64;
    let mut total_packed = 0u64;
    let mut dir_count = 0;
    let mut file_count = 0;

    println!("║ {:<4} {:<32} {:>10} {:>10} {:>6} {:>8} {:<10} ║",
        "Attr", "Name", "Original", "Packed", "Ratio", "CRC-32", "Date Time");
    println!("╠{}╣", sep);

    for entry in &archive.central_directory {
        let is_dir = entry.is_dir;
        let icon = if is_dir {
            dir_count += 1;
            "📁"
        } else {
            file_count += 1;
            total_orig += entry.uncompressed_size;
            total_packed += entry.compressed_size;
            if entry.path.ends_with(".exe") || entry.path.ends_with(".dll") || entry.path.ends_with(".bin") {
                "⚙ "
            } else if entry.path.ends_with(".rs") || entry.path.ends_with(".py") || entry.path.ends_with(".c") || entry.path.ends_with(".js") {
                "🖹 "
            } else if entry.path.ends_with(".zip") || entry.path.ends_with(".tar") || entry.path.ends_with(".vpack") {
                "🗀 "
            } else if entry.path.ends_with(".png") || entry.path.ends_with(".jpg") || entry.path.ends_with(".svg") {
                "🖼 "
            } else {
                "📄"
            }
        };

        let ratio = if entry.uncompressed_size > 0 {
            (1.0 - (entry.compressed_size as f64 / entry.uncompressed_size as f64)) * 100.0
        } else {
            0.0
        };

        let dt_str = if entry.modified_timestamp > 0 {
            if let Some(dt) = Local.timestamp_opt(entry.modified_timestamp, 0).single() {
                dt.format("%Y-%m-%d %H:%M").to_string()
            } else {
                "----/--/-- --:--".into()
            }
        } else {
            "----/--/-- --:--".into()
        };

        let name_display = if entry.path.len() > 32 {
            format!("{}...", &entry.path[..29])
        } else {
            entry.path.clone()
        };

        if is_dir {
            println!("║ {:<4} {:<32} {:>10} {:>10} {:>6} {:>8} {:<10} ║",
                icon, name_display, "<DIR>", "-", "-", "-", dt_str);
        } else {
            println!("║ {:<4} {:<32} {:>10} {:>10} {:>5.0}% {:08X} {:<10} ║",
                icon, name_display, entry.uncompressed_size, entry.compressed_size, ratio.max(0.0), entry.crc32, dt_str);
        }
    }

    println!("╠{}╣", sep);
    let total_savings = if total_orig > 0 {
        (1.0 - (total_packed as f64 / total_orig as f64)) * 100.0
    } else {
        0.0
    };

    println!("║ Total: {} files, {} folders | Orig: {:>6.2} MB | Packed: {:>6.2} MB | Ratio: {:>4.1}% ║",
        file_count,
        dir_count,
        total_orig as f64 / (1024.0 * 1024.0),
        total_packed as f64 / (1024.0 * 1024.0),
        total_savings.max(0.0)
    );

    let status_sec = if (archive.flags & FLAG_ENCRYPTED) != 0 {
        "🔒 Encrypted (AES/Stream)"
    } else if (archive.flags & FLAG_SIGNED) != 0 {
        "✓ Digitally Signed (Ed25519)"
    } else {
        "Standard VPack Archive"
    };

    println!("║ Format: VPK2 (Central Directory at EOF) | Security: {:<40} ║", status_sec);
    println!("╚{}╝", double_sep);
}


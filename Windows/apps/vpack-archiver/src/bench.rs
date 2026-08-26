/*!
Hardware-accelerated benchmark suite for compression, decompression, and CRC-32.
*/

use anyhow::Result;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use std::io::{Read, Write};
use std::time::Instant;

pub fn run_benchmark(size_mb: usize) -> Result<()> {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!(" ⚡ VPack Archiver Compression Benchmark Suite");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!(" Workload: {} MB entropy dataset", size_mb);

    let total_bytes = size_mb * 1024 * 1024;
    let mut data = Vec::with_capacity(total_bytes);
    for i in 0..total_bytes {
        data.push(((i * 31 + (i >> 3)) ^ (i >> 7)) as u8);
    }

    print!(" [1/3] Benchmarking Streaming Deflate (Level 6)... ");
    let start_comp = Instant::now();
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(6));
    encoder.write_all(&data)?;
    let compressed = encoder.finish()?;
    let comp_duration = start_comp.elapsed();
    let comp_speed = (size_mb as f64) / comp_duration.as_secs_f64();
    let ratio = (1.0 - (compressed.len() as f64 / data.len() as f64)) * 100.0;
    println!("✓ {:.2} MB/s", comp_speed);

    print!(" [2/3] Benchmarking Streaming Decompression...    ");
    let start_decomp = Instant::now();
    let mut decoder = DeflateDecoder::new(&compressed[..]);
    let mut decompressed = Vec::with_capacity(data.len());
    decoder.read_to_end(&mut decompressed)?;
    let decomp_duration = start_decomp.elapsed();
    let decomp_speed = (size_mb as f64) / decomp_duration.as_secs_f64();
    println!("✓ {:.2} MB/s", decomp_speed);

    print!(" [3/3] Benchmarking Hardware CRC-32 Checksum...   ");
    let start_crc = Instant::now();
    let crc = crate::archive::crc32_compute(&data);
    let crc_duration = start_crc.elapsed();
    let crc_speed = (size_mb as f64) / crc_duration.as_secs_f64();
    println!("✓ {:.2} MB/s", crc_speed);

    println!("─────────────────────────────────────────────────────────────────");
    println!(" Benchmark Summary:");
    println!("   • Compression Speed:     {:.2} MB/s ({:.3}s)", comp_speed, comp_duration.as_secs_f64());
    println!("   • Decompression Speed:   {:.2} MB/s ({:.3}s)", decomp_speed, decomp_duration.as_secs_f64());
    println!("   • Checksum Rate:         {:.2} MB/s (CRC32: {:08X})", crc_speed, crc);
    println!("   • Space Saved:           {:.2}% ({:.2} MB -> {:.2} MB)",
        ratio,
        data.len() as f64 / (1024.0 * 1024.0),
        compressed.len() as f64 / (1024.0 * 1024.0)
    );
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    Ok(())
}

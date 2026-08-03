// Compare our FLAC vs reference FLAC on a small WAV
fn main() {
    // Generate a 1MB WAV (small enough for quick test)
    let sample_rate: u32 = 44_100;
    let total_samples: usize = 500_000; // ~1MB PCM
    let data_size = total_samples * 2;

    let mut wav = Vec::with_capacity(44 + data_size);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&[1u8, 0]); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&[2u8, 0]);
    wav.extend_from_slice(&[16u8, 0]);
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_size as u32).to_le_bytes());

    for i in 0..total_samples {
        let t = i as f64 / sample_rate as f64;
        let val = (t * 2.0 * std::f64::consts::PI * 440.0).sin() * 0.8 * 32767.0;
        wav.extend_from_slice(&(val as i16).to_le_bytes());
    }

    println!("WAV size: {} bytes", wav.len());

    // Our FLAC
    let flac_compressed = limnifs_core::codec::compress(0x07, &wav).expect("flac compress");
    println!("Our FLAC: {} bytes ({:.1}%)", flac_compressed.len(),
        flac_compressed.len() as f64 / wav.len() as f64 * 100.0);

    // Compare with Brotli on same WAV
    let brotli_compressed = limnifs_core::codec::compress(0x04, &wav).expect("brotli compress");
    println!("Brotli:   {} bytes ({:.1}%)", brotli_compressed.len(),
        brotli_compressed.len() as f64 / wav.len() as f64 * 100.0);

    // Compare with ZSTD
    let zstd_compressed = limnifs_core::codec::compress(0x02, &wav).expect("zstd compress");
    println!("ZSTD:     {} bytes ({:.1}%)", zstd_compressed.len(),
        zstd_compressed.len() as f64 / wav.len() as f64 * 100.0);

    // Compare with LZMA
    let xz_result = limnifs_core::codec::compress(0x03, &wav);
    if let Ok(xz_compressed) = xz_result {
        println!("XZ:       {} bytes ({:.1}%)", xz_compressed.len(),
            xz_compressed.len() as f64 / wav.len() as f64 * 100.0);
    } else {
        println!("XZ:       encode not available");
    }
}

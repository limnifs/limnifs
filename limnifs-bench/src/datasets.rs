//! Dataset catalog — all public, all GHA-reproducible.
//!
//! Categories span: source code, AI models, binaries, synthetic stress tests.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Source,
    AiModel,
    Binary,
    Synthetic,
}

#[derive(Clone, Debug)]
pub struct Dataset {
    pub name: &'static str,
    pub category: Category,
    pub url: Option<&'static str>,
    pub approx_size_mb: usize,
    pub description: &'static str,
}

pub const DATASETS: &[Dataset] = &[
    // --- Source code ---
    Dataset {
        name: "php",
        category: Category::Source,
        url: Some("https://www.php.net/distributions/php-8.3.0.tar.gz"),
        approx_size_mb: 70,
        description: "PHP 8.3.0 source tree (DwarFS's primary benchmark dataset)",
    },
    Dataset {
        name: "python",
        category: Category::Source,
        url: Some("https://www.python.org/ftp/python/3.12.0/Python-3.12.0.tgz"),
        approx_size_mb: 95,
        description: "Python 3.12.0 CPython source tree",
    },
    Dataset {
        name: "linux",
        category: Category::Source,
        url: Some("https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.6.tar.xz"),
        approx_size_mb: 1300,
        description: "Linux 6.6 kernel source tree (stress test)",
    },
    // --- AI models ---
    Dataset {
        name: "gpt2",
        category: Category::AiModel,
        url: Some("https://huggingface.co/openai-community/gpt2/resolve/main/model.safetensors"),
        approx_size_mb: 548,
        description: "GPT-2 117M model weights (safetensors format)",
    },
    Dataset {
        name: "whisper-tiny",
        category: Category::AiModel,
        url: Some("https://huggingface.co/openai/whisper-tiny/resolve/main/model.safetensors"),
        approx_size_mb: 151,
        description: "OpenAI Whisper-tiny speech recognition model weights",
    },
    Dataset {
        name: "resnet50",
        category: Category::AiModel,
        url: Some("https://huggingface.co/microsoft/resnet-50/resolve/main/onnx/model.onnx"),
        approx_size_mb: 102,
        description: "Microsoft ResNet-50 image classification model (ONNX format)",
    },
    // --- Synthetic ---
    Dataset {
        name: "zeros",
        category: Category::Synthetic,
        url: None,
        approx_size_mb: 100,
        description: "100 MB of all-zero bytes (maximum compressibility)",
    },
    Dataset {
        name: "random",
        category: Category::Synthetic,
        url: None,
        approx_size_mb: 100,
        description: "100 MB of random bytes (zero compressibility — store test)",
    },
    Dataset {
        name: "tiny-files",
        category: Category::Synthetic,
        url: None,
        approx_size_mb: 50,
        description: "50,000 tiny 1 KB files (inode/metadata stress)",
    },
    Dataset {
        name: "repetitive",
        category: Category::Synthetic,
        url: None,
        approx_size_mb: 100,
        description: "100 MB of repetitive text (pattern compression stress)",
    },
    // --- CSV / structured text (FSST baseline) ---
    Dataset {
        name: "taxi-csv",
        category: Category::Source,
        url: Some("https://d37ci6vzurychx.cloudfront.net/trip-data/yellow_tripdata_2024-01.csv"),
        approx_size_mb: 100,
        description: "NYC Taxi & Limousine Commission trip data (CSV, repetitive column headers \
                      + structured numeric data — FSST preprocessor baseline)",
    },
    Dataset {
        name: "csv-synthetic",
        category: Category::Synthetic,
        url: None,
        approx_size_mb: 50,
        description: "50 MB synthetic CSV with repeated column values (FSST target workload)",
    },
    // --- FITS / scientific images (Rice++ baseline) ---
    Dataset {
        name: "fits-synthetic",
        category: Category::Synthetic,
        url: None,
        approx_size_mb: 50,
        description: "50 MB synthetic FITS-like 16-bit pixel data with smooth gradients \
                      (Rice++ target workload)",
    },
    // --- PCM audio (FLAC baseline) ---
    Dataset {
        name: "wav-synthetic",
        category: Category::Synthetic,
        url: None,
        approx_size_mb: 50,
        description: "50 MB synthetic 16-bit PCM WAV with sine-wave audio (FLAC target workload)",
    },
];

/// Find a dataset by name.
pub fn find(name: &str) -> Option<&'static Dataset> {
    DATASETS.iter().find(|d| d.name == name)
}

/// Ensure a dataset is available on disk. Downloads or generates as needed.
/// Returns the path to the dataset directory.
pub fn ensure(dataset: &Dataset, cache_dir: &Path) -> std::io::Result<PathBuf> {
    let ds_dir = cache_dir.join(dataset.name);

    // Check if already prepared
    if ds_dir.exists() && ds_dir.read_dir()?.next().is_some() {
        return Ok(ds_dir);
    }

    std::fs::create_dir_all(&ds_dir)?;

    match dataset.category {
        Category::Source => {
            let url = dataset.url.unwrap();
            println!("  Downloading {} from {}…", dataset.name, url);
            let tarball = cache_dir.join(format!("{}.tar", dataset.name));
            download(url, &tarball)?;
            println!("  Extracting…");
            extract_tarball(&tarball, &ds_dir)?;
            let _ = std::fs::remove_file(&tarball);
        }
        Category::AiModel => {
            let url = dataset.url.unwrap();
            println!("  Downloading {} from {}…", dataset.name, url);
            let filename = url.rsplit('/').next().unwrap_or("model");
            let dest = ds_dir.join(filename);
            download(url, &dest)?;
        }
        Category::Binary => {
            // Use this workspace's release build
            let release = Path::new("../target/release");
            if release.exists() {
                copy_dir(release, &ds_dir)?;
            }
        }
        Category::Synthetic => {
            println!("  Generating {}…", dataset.name);
            generate_synthetic(dataset.name, &ds_dir)?;
        }
    }

    Ok(ds_dir)
}

fn download(url: &str, dest: &Path) -> std::io::Result<()> {
    let status = Command::new("curl")
        .args(["-L", "-o"])
        .arg(dest)
        .arg(url)
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other(format!("curl failed for {url}")));
    }
    Ok(())
}

fn extract_tarball(tarball: &Path, dest: &Path) -> std::io::Result<()> {
    let status = Command::new("tar")
        .args(["-xf"])
        .arg(tarball)
        .args(["-C"])
        .arg(dest)
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other("tar extraction failed"));
    }
    Ok(())
}

fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    let status = Command::new("cp")
        .args(["-R"])
        .arg(src)
        .arg(dest)
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other("cp failed"));
    }
    Ok(())
}

fn generate_synthetic(name: &str, dir: &Path) -> std::io::Result<()> {
    match name {
        "zeros" => {
            std::fs::write(dir.join("zeros.bin"), vec![0u8; 100 * 1024 * 1024])?;
        }
        "random" => {
            use std::io::Write;
            let mut f = std::fs::File::create(dir.join("random.bin"))?;
            let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
            let mut buf = [0u8; 4096];
            for _ in 0..25_600 {
                // xorshift64* — fast, statistically decent, no std dep
                for chunk in buf.chunks_mut(8) {
                    state ^= state >> 12;
                    state ^= state << 25;
                    state ^= state >> 27;
                    let out = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
                    chunk.copy_from_slice(&out.to_le_bytes()[..chunk.len()]);
                }
                f.write_all(&buf)?;
            }
        }
        "tiny-files" => {
            std::fs::create_dir_all(dir.join("files"))?;
            let content = b"tiny file content\n";
            for i in 0..50_000 {
                std::fs::write(dir.join("files").join(format!("{i}.txt")), content)?;
            }
        }
        "repetitive" => {
            let text = "The quick brown fox jumps over the lazy dog. ".repeat(2381); // ~100 KB
            let mut content = String::with_capacity(100 * 1024 * 1024);
            for _ in 0..1000 {
                content.push_str(&text);
            }
            std::fs::write(dir.join("repetitive.txt"), content)?;
        }
        "csv-synthetic" => {
            // Structured CSV with strong column redundancy — the
            // workload FSST is designed for. Five columns of
            // repeated category labels + numeric values.
            use std::io::Write;
            let mut f = std::fs::File::create(dir.join("data.csv"))?;
            writeln!(f, "id,region,product,quantity,price")?;
            let regions = ["north", "south", "east", "west", "central"];
            let products = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta"];
            // ~50 MB target. Each row is ~70 bytes; need ~750k rows.
            for i in 0..750_000u64 {
                let region = regions[(i as usize) % regions.len()];
                let product = products[(i as usize) % products.len()];
                let qty = (i % 1000) + 1;
                let price = ((i as f64) * 0.07).fract() * 100.0;
                writeln!(f, "{i},{region},{product},{qty},{price:.2}")?;
            }
        }
        "fits-synthetic" => {
            // Synthetic astronomical-image-like data: 16-bit pixels
            // with smooth local gradients + sparse high-noise regions.
            // 50 MB = 25 M pixels × 2 bytes each. Stored as a raw
            // .bin (the categorizer detects by .fits extension OR by
            // the file_categorizer when wired with content sniffing).
            use std::io::Write;
            let mut f = std::fs::File::create(dir.join("image.fits"))?;
            // Minimal FITS header (2880 bytes).
            let mut header = vec![b' '; 2880];
            let copy = |buf: &mut [u8], off: usize, rec: &str| {
                let bytes = rec.as_bytes();
                let n = bytes.len().min(80);
                buf[off..off + n].copy_from_slice(&bytes[..n]);
            };
            copy(&mut header, 0, "SIMPLE  = T");
            copy(&mut header, 80, "BITPIX  = 16");
            copy(&mut header, 160, "NAXIS   = 2");
            copy(&mut header, 240, "NAXIS1  = 5000");
            copy(&mut header, 320, "NAXIS2  = 5000");
            copy(&mut header, 400, "END");
            f.write_all(&header)?;
            // 25 M pixels of smooth-gradient 16-bit data.
            let mut state: u64 = 0x1234_5678_9ABC_DEF0;
            let mut buf = vec![0u8; 1 << 16]; // 64 KiB write buffer
            let total_pixels = 25_000_000usize;
            let mut written = 0usize;
            let mut idx = 0u64;
            while written < total_pixels * 2 {
                let chunk_pixels = (buf.len() / 2).min(total_pixels - written / 2);
                for i in 0..chunk_pixels {
                    // Smooth gradient: pixel = base + small variation.
                    let base = ((idx + i as u64) / 8) & 0xFFFF;
                    let noise = (state >> 56) & 0x0F; // small jitter
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    let pixel = (base ^ noise) & 0xFFFF;
                    let off = i * 2;
                    buf[off] = (pixel >> 8) as u8;
                    buf[off + 1] = pixel as u8;
                }
                let n = chunk_pixels * 2;
                f.write_all(&buf[..n])?;
                written += n;
                idx += chunk_pixels as u64;
            }
        }
        "wav-synthetic" => {
            // Clean PCM audio: 50 MB WAV at 44.1 kHz, 16-bit mono.
            // Pure 440 Hz sine wave — highly predictable, which is
            // exactly what FLAC's LPC predictor is designed for.
            use std::io::Write;
            let sample_rate = 44_100u32;
            let channels = 1u8;
            let bits = 16u8;
            let total_samples = 25_000_000usize / (channels as usize * bits as usize / 8);
            let data_size = total_samples * 2;
            let mut f = std::fs::File::create(dir.join("audio.wav"))?;
            // RIFF/WAVE header.
            f.write_all(b"RIFF")?;
            f.write_all(&(36 + data_size as u32).to_le_bytes())?;
            f.write_all(b"WAVE")?;
            f.write_all(b"fmt ")?;
            f.write_all(&16u32.to_le_bytes())?;
            f.write_all(&1u16.to_le_bytes())?; // PCM
            f.write_all(&[channels, 0])?;
            f.write_all(&sample_rate.to_le_bytes())?;
            f.write_all(&(sample_rate * channels as u32 * bits as u32 / 8).to_le_bytes())?;
            f.write_all(&[(channels * bits / 8), 0])?;
            f.write_all(&[bits, 0])?;
            f.write_all(b"data")?;
            f.write_all(&(data_size as u32).to_le_bytes())?;
            // Clean 440 Hz sine at 0.8 amplitude. LPC predictor should
            // model this near-perfectly (residuals near zero).
            let mut buf = vec![0u8; 1 << 16];
            let mut written = 0usize;
            let mut idx = 0u64;
            while written < data_size {
                let chunk_samples = (buf.len() / 2).min(total_samples - written / 2);
                for i in 0..chunk_samples {
                    let t = (idx + i as u64) as f64 / sample_rate as f64;
                    let val = (t * 2.0 * std::f64::consts::PI * 440.0).sin() * 0.8 * 26000.0;
                    let sample = val as i16;
                    let off = i * 2;
                    buf[off] = (sample & 0xFF) as u8;
                    buf[off + 1] = ((sample >> 8) & 0xFF) as u8;
                }
                let n = chunk_samples * 2;
                f.write_all(&buf[..n])?;
                written += n;
                idx += chunk_samples as u64;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Measure the total size of all files under a directory.
pub fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                total += dir_size(&p);
            } else if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

/// Count files under a directory.
pub fn file_count(path: &Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                count += file_count(&p);
            } else {
                count += 1;
            }
        }
    }
    count
}

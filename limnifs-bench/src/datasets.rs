//! Dataset catalog — all public, all GHA-reproducible.
//!
//! Categories span: source code, AI models, binaries, synthetic stress tests.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, serde::Serialize)]
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
            let chunk = [0xABu8; 4096];
            for _ in 0..25_600 {
                // ~100 MB
                f.write_all(&chunk)?;
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

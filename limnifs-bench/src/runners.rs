//! Benchmark runners — `LimniFS` via library calls, external tools via subprocess.

#![allow(unsafe_code)]
#![warn(clippy::pedantic)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::metrics::OperationResult;
use crate::resource::ResourceSnapshot;

pub struct WorkspacePaths {
    pub cache_dir: PathBuf,
    pub work_dir: PathBuf,
}

impl WorkspacePaths {
    pub fn new(workspace: &Path) -> Self {
        Self {
            cache_dir: workspace.join(".scratch").join("bench-datasets"),
            work_dir: workspace.join(".scratch").join("bench-work"),
        }
    }
}

/// Run `LimniFS` create via direct library call.
///
/// Writes the manifest, all slabs, and the optional metadata sidecar
/// to `work`, mirroring what the `limni limn` CLI does. Without these
/// on disk, downstream extract/verify would fail.
///
/// `profile_name` selects the built-in `WriteConfig` from
/// `limnifs_write::profile::select`. The format string emitted on
/// every `OperationResult` is `limnifs:{profile_name}` so multi-
/// profile reports can distinguish rows. The image filename is
/// `limnifs-{profile_name}.lim` so profiles do not collide.
pub fn limnifs_create(
    source: &Path,
    work: &Path,
    iterations: usize,
    profile_name: &str,
) -> Vec<OperationResult> {
    let mut results = Vec::with_capacity(iterations);
    let image = work.join(format!("limnifs-{profile_name}.lim"));
    let format_tag = format!("limnifs:{profile_name}");
    let config = match limnifs_write::profile::select(profile_name) {
        Some(c) => c,
        None => {
            eprintln!("  [limnifs:{profile_name}] unknown profile");
            return results;
        }
    };

    for i in 0..iterations {
        let _ = std::fs::remove_file(&image);
        let before = ResourceSnapshot::now();
        let start = Instant::now();
        let artifact = limnifs_write::write_directory_with_config(source, &config);
        let elapsed = start.elapsed();
        let after = ResourceSnapshot::now();

        match artifact {
            Ok(a) => {
                let manifest_size = a.bytes.len() as u64;
                let mut total_size = manifest_size;

                for slab in &a.slabs {
                    let slab_name = slab.locator.strip_prefix("file:").unwrap_or(&slab.locator);
                    let slab_path = work.join(slab_name);
                    if let Err(e) = std::fs::write(&slab_path, &slab.bytes) {
                        eprintln!("  [{format_tag}] create iteration {i}: slab write failed: {e}");
                        results.push(OperationResult::failure(&format_tag, "create", elapsed));
                        continue;
                    }
                    total_size += slab.bytes.len() as u64;
                }

                if let Some(sidecar) = &a.metadata_sidecar {
                    let name = sidecar
                        .locator
                        .strip_prefix("file:")
                        .unwrap_or(&sidecar.locator);
                    let sidecar_path = work.join(name);
                    if let Err(e) = std::fs::write(&sidecar_path, &sidecar.bytes) {
                        eprintln!(
                            "  [{format_tag}] create iteration {i}: metadata sidecar write failed: {e}"
                        );
                        results.push(OperationResult::failure(&format_tag, "create", elapsed));
                        continue;
                    }
                    total_size += sidecar.bytes.len() as u64;
                }

                let _ = std::fs::write(&image, &a.bytes);
                results.push(OperationResult::measure(
                    &format_tag, "create", before, after, elapsed, total_size, 1,
                ));
            }
            Err(e) => {
                eprintln!("  [{format_tag}] create iteration {i} failed: {e}");
                results.push(OperationResult::failure(&format_tag, "create", elapsed));
            }
        }
    }
    results
}

/// Run `LimniFS` extract via the limni binary (subprocess — extract is in the CLI).
pub fn limnifs_extract(
    image: &Path,
    work: &Path,
    iterations: usize,
    input_size: u64,
    profile_name: &str,
) -> Vec<OperationResult> {
    let limni = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("limni")))
        .unwrap_or_else(|| PathBuf::from("limni"));

    let format_tag = format!("limnifs:{profile_name}");
    let mut results = Vec::with_capacity(iterations);
    let dest = work.join(format!("extract_limnifs_{profile_name}"));

    for _ in 0..iterations {
        let _ = std::fs::remove_dir_all(&dest);
        let _ = std::fs::create_dir_all(&dest);
        let before = ResourceSnapshot::now();
        let before_children = ResourceSnapshot::children();
        let start = Instant::now();
        let status = Command::new(&limni)
            .args(["extract"])
            .arg(image)
            .arg(&dest)
            .status();
        let elapsed = start.elapsed();
        let after_children = ResourceSnapshot::children();
        let after = ResourceSnapshot::now();

        match status {
            Ok(s) if s.success() => {
                let user = after.user_secs - before.user_secs
                    + (after_children.user_secs - before_children.user_secs);
                let sys = after.system_secs - before.system_secs
                    + (after_children.system_secs - before_children.system_secs);
                let rss = after.rss_bytes.max(after_children.rss_bytes);
                let mut r = OperationResult::measure(
                    &format_tag, "extract", before, after, elapsed, input_size, 1,
                );
                r.cpu_user_secs = user.max(0.0);
                r.cpu_system_secs = sys.max(0.0);
                r.peak_rss_bytes = rss;
                results.push(r);
            }
            _ => {
                results.push(OperationResult::failure(&format_tag, "extract", elapsed));
            }
        }
    }
    results
}

/// Run `LimniFS` verify via the limni binary.
pub fn limnifs_verify(image: &Path, iterations: usize, profile_name: &str) -> Vec<OperationResult> {
    let limni = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("limni")))
        .unwrap_or_else(|| PathBuf::from("limni"));

    let format_tag = format!("limnifs:{profile_name}");
    let mut results = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        let status = Command::new(&limni).args(["verify"]).arg(image).status();
        let elapsed = start.elapsed();
        match status {
            Ok(s) if s.success() => {
                results.push(OperationResult::success(&format_tag, "verify", elapsed, 0));
            }
            _ => results.push(OperationResult::failure(&format_tag, "verify", elapsed)),
        }
    }
    results
}

/// Benchmark `DwarFS` create (mkdwarfs), if available.
pub fn dwarfs_create(source: &Path, work: &Path, iterations: usize) -> Vec<OperationResult> {
    run_external(
        "mkdwarfs",
        &["-l6", "--no-history"],
        source,
        work,
        "dwarfs",
        "create",
        "test.dwarfs",
        iterations,
    )
}

pub fn dwarfs_extract(
    image: &Path,
    work: &Path,
    iterations: usize,
    input_size: u64,
) -> Vec<OperationResult> {
    use crate::resource::ResourceSnapshot;

    let dest = work.join("extract_dwarfs");
    let mut results = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let _ = std::fs::remove_dir_all(&dest);
        let before = ResourceSnapshot::children();
        let start = Instant::now();
        let status = Command::new("dwarfsextract")
            .arg("-i")
            .arg(image)
            .arg("-o")
            .arg(&dest)
            .status();
        let elapsed = start.elapsed();
        let after = ResourceSnapshot::children();
        match status {
            Ok(s) if s.success() => {
                let mut r = OperationResult::success("dwarfs", "extract", elapsed, input_size);
                r.cpu_user_secs = (after.user_secs - before.user_secs).max(0.0);
                r.cpu_system_secs = (after.system_secs - before.system_secs).max(0.0);
                r.peak_rss_bytes = after.rss_bytes.max(before.rss_bytes);
                results.push(r);
            }
            _ => results.push(OperationResult::failure("dwarfs", "extract", elapsed)),
        }
    }
    results
}

/// `SquashFS`
pub fn squashfs_create(source: &Path, work: &Path, iterations: usize) -> Vec<OperationResult> {
    use crate::resource::ResourceSnapshot;
    let mut results = Vec::with_capacity(iterations);
    let image = work.join("squashfs.squashfs");
    for _ in 0..iterations {
        let _ = std::fs::remove_file(&image);
        let before = ResourceSnapshot::children();
        let start = Instant::now();
        let status = Command::new("mksquashfs")
            .arg(source)
            .arg(&image)
            .args([
                "-noappend",
                "-comp",
                "zstd",
                "-Xcompression-level",
                "1",
                "-no-progress",
            ])
            .status();
        let elapsed = start.elapsed();
        let after = ResourceSnapshot::children();
        match status {
            Ok(s) if s.success() => {
                let size = std::fs::metadata(&image).map(|m| m.len()).unwrap_or(0);
                let mut r = OperationResult::success("squashfs", "create", elapsed, size);
                r.cpu_user_secs = (after.user_secs - before.user_secs).max(0.0);
                r.cpu_system_secs = (after.system_secs - before.system_secs).max(0.0);
                r.peak_rss_bytes = after.rss_bytes.max(before.rss_bytes);
                results.push(r);
            }
            _ => results.push(OperationResult::failure("squashfs", "create", elapsed)),
        }
    }
    results
}

pub fn squashfs_extract(
    image: &Path,
    work: &Path,
    iterations: usize,
    input_size: u64,
) -> Vec<OperationResult> {
    use crate::resource::ResourceSnapshot;

    let dest = work.join("extract_sqfs");
    let mut results = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let _ = std::fs::remove_dir_all(&dest);
        let before = ResourceSnapshot::children();
        let start = Instant::now();
        let status = Command::new("unsquashfs")
            .arg("-d")
            .arg(&dest)
            .args(["-no-progress"])
            .arg(image)
            .status();
        let elapsed = start.elapsed();
        let after = ResourceSnapshot::children();
        match status {
            Ok(s) if s.success() => {
                let mut r = OperationResult::success("squashfs", "extract", elapsed, input_size);
                r.cpu_user_secs = (after.user_secs - before.user_secs).max(0.0);
                r.cpu_system_secs = (after.system_secs - before.system_secs).max(0.0);
                r.peak_rss_bytes = after.rss_bytes.max(before.rss_bytes);
                results.push(r);
            }
            _ => results.push(OperationResult::failure("squashfs", "extract", elapsed)),
        }
    }
    results
}

/// tar + zstd
pub fn tar_zstd_create(source: &Path, work: &Path, iterations: usize) -> Vec<OperationResult> {
    use crate::resource::ResourceSnapshot;
    let mut results = Vec::with_capacity(iterations);
    let archive = work.join("test.tar.zst");
    for _ in 0..iterations {
        let _ = std::fs::remove_file(&archive);
        let before = ResourceSnapshot::children();
        let start = Instant::now();
        let status = Command::new("tar")
            .args(["-cf"])
            .arg(&archive)
            .args(["--use-compress-program=zstd -1"])
            .args(["-C"])
            .arg(source.parent().unwrap_or(Path::new(".")))
            .arg(source.file_name().unwrap_or_default())
            .status();
        let elapsed = start.elapsed();
        let after = ResourceSnapshot::children();
        match status {
            Ok(s) if s.success() => {
                let size = std::fs::metadata(&archive).map(|m| m.len()).unwrap_or(0);
                let mut r = OperationResult::success("tar+zstd", "create", elapsed, size);
                r.cpu_user_secs = (after.user_secs - before.user_secs).max(0.0);
                r.cpu_system_secs = (after.system_secs - before.system_secs).max(0.0);
                r.peak_rss_bytes = after.rss_bytes.max(before.rss_bytes);
                results.push(r);
            }
            _ => results.push(OperationResult::failure("tar+zstd", "create", elapsed)),
        }
    }
    results
}

pub fn tar_zstd_extract(
    archive: &Path,
    work: &Path,
    iterations: usize,
    input_size: u64,
) -> Vec<OperationResult> {
    let dest = work.join("extract_tar");
    let mut results = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let _ = std::fs::remove_dir_all(&dest);
        let _ = std::fs::create_dir_all(&dest);
        let start = Instant::now();
        let status = Command::new("tar")
            .args(["-xf"])
            .arg(archive)
            .args(["--use-compress-program=zstd -d"])
            .args(["-C"])
            .arg(&dest)
            .status();
        let elapsed = start.elapsed();
        match status {
            Ok(s) if s.success() => results.push(OperationResult::success(
                "tar+zstd", "extract", elapsed, input_size,
            )),
            _ => results.push(OperationResult::failure("tar+zstd", "extract", elapsed)),
        }
    }
    results
}

// Helpers

fn run_external(
    tool: &str,
    flags: &[&str],
    source: &Path,
    work: &Path,
    format: &str,
    op: &str,
    image_name: &str,
    iterations: usize,
) -> Vec<OperationResult> {
    use crate::resource::ResourceSnapshot;

    if which(tool).is_none() {
        return Vec::new();
    }
    let mut results = Vec::with_capacity(iterations);
    let image = work.join(image_name);
    for _ in 0..iterations {
        let _ = std::fs::remove_file(&image);
        let before = ResourceSnapshot::children();
        let start = Instant::now();
        let status = Command::new(tool)
            .arg("-i")
            .arg(source)
            .arg("-o")
            .arg(&image)
            .args(flags)
            .status();
        let elapsed = start.elapsed();
        let after = ResourceSnapshot::children();
        match status {
            Ok(s) if s.success() => {
                let size = std::fs::metadata(&image).map(|m| m.len()).unwrap_or(0);
                let mut r = OperationResult::success(format, op, elapsed, size);
                r.cpu_user_secs = (after.user_secs - before.user_secs).max(0.0);
                r.cpu_system_secs = (after.system_secs - before.system_secs).max(0.0);
                r.peak_rss_bytes = after.rss_bytes.max(before.rss_bytes);
                results.push(r);
            }
            _ => results.push(OperationResult::failure(format, op, elapsed)),
        }
    }
    results
}

fn run_external_extract(
    tool: &str,
    flags: &[&str],
    image: &Path,
    work: &Path,
    format: &str,
    op: &str,
    input_size: u64,
    iterations: usize,
) -> Vec<OperationResult> {
    use crate::resource::ResourceSnapshot;

    if which(tool).is_none() {
        return Vec::new();
    }
    let dest = work.join(format!("extract_{format}"));
    let mut results = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let _ = std::fs::remove_dir_all(&dest);
        let before = ResourceSnapshot::children();
        let start = Instant::now();
        let status = Command::new(tool)
            .args(flags)
            .arg(image)
            .arg(&dest)
            .status();
        let elapsed = start.elapsed();
        let after = ResourceSnapshot::children();
        match status {
            Ok(s) if s.success() => {
                let mut r = OperationResult::success(format, op, elapsed, input_size);
                r.cpu_user_secs = (after.user_secs - before.user_secs).max(0.0);
                r.cpu_system_secs = (after.system_secs - before.system_secs).max(0.0);
                r.peak_rss_bytes = after.rss_bytes.max(before.rss_bytes);
                results.push(r);
            }
            _ => results.push(OperationResult::failure(format, op, elapsed)),
        }
    }
    results
}

fn which(tool: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let full = dir.join(tool);
        if full.is_file() {
            return Some(full);
        }
    }
    None
}

// ---------------------------------------------------------------------
// Single-file operations: extract_one, locate_one, read_random.
// ---------------------------------------------------------------------

/// Snapshot resources around a subprocess invocation; aggregate
/// self + children CPU and peak RSS.
fn measure_subprocess(
    format: &str,
    operation: &str,
    status: std::io::Result<std::process::ExitStatus>,
    elapsed: Duration,
    before: ResourceSnapshot,
    before_children: ResourceSnapshot,
    after: ResourceSnapshot,
    after_children: ResourceSnapshot,
    output_size: u64,
) -> OperationResult {
    let user = (after.user_secs - before.user_secs)
        + (after_children.user_secs - before_children.user_secs);
    let sys = (after.system_secs - before.system_secs)
        + (after_children.system_secs - before_children.system_secs);
    let rss = after.rss_bytes.max(after_children.rss_bytes);
    match status {
        Ok(s) if s.success() => {
            let mut r =
                OperationResult::measure(format, operation, before, after, elapsed, output_size, 1);
            r.cpu_user_secs = user.max(0.0);
            r.cpu_system_secs = sys.max(0.0);
            r.peak_rss_bytes = rss;
            r
        }
        _ => OperationResult::failure(format, operation, elapsed),
    }
}

/// Run a single-file extraction benchmark. Each format uses its OWN
/// image file (limnifs.lim, test.dwarfs, etc.) — the caller passes
/// the per-format image paths via `images`.
#[allow(clippy::too_many_lines)]
pub fn extract_one(
    images: &std::collections::HashMap<&str, PathBuf>,
    target_path: &str,
    work: &Path,
    iterations: usize,
    formats: &[&str],
) -> Vec<OperationResult> {
    let mut results = Vec::with_capacity(iterations * formats.len());
    let dest = work.join("extract_one_out");

    for _ in 0..iterations {
        for &format in formats {
            let image = match images.get(format) {
                Some(p) => p,
                None => continue,
            };
            let before = ResourceSnapshot::now();
            let before_children = ResourceSnapshot::children();
            let start = Instant::now();
            let status: std::io::Result<std::process::ExitStatus> = match format {
                "limnifs" => {
                    let limni = std::env::current_exe()
                        .ok()
                        .and_then(|p| p.parent().map(|d| d.join("limni")))
                        .unwrap_or_else(|| PathBuf::from("limni"));
                    Command::new(&limni)
                        .args(["cat"])
                        .arg(image)
                        .arg(target_path)
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()
                }
                "dwarfs" => {
                    let _ = std::fs::remove_file(&dest);
                    Command::new("dwarfsextract")
                        .args(["-i"])
                        .arg(image)
                        .args(["-f"])
                        .arg(target_path.trim_start_matches('/'))
                        .args(["-o"])
                        .arg(&dest)
                        .status()
                }
                "squashfs" => {
                    let _ = std::fs::remove_dir_all(&dest);
                    let _ = std::fs::create_dir_all(&dest);
                    Command::new("unsquashfs")
                        .args(["-f", "-d"])
                        .arg(&dest)
                        .arg(image)
                        .arg(target_path.trim_start_matches('/'))
                        .stdout(std::process::Stdio::null())
                        .status()
                }
                "tar+zstd" => Command::new("tar")
                    .args(["-xf"])
                    .arg(image)
                    .args(["-C"])
                    .arg(work)
                    .arg(target_path.trim_start_matches('/'))
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status(),
                _ => continue,
            };
            let elapsed = start.elapsed();
            let after_children = ResourceSnapshot::children();
            let after = ResourceSnapshot::now();
            results.push(measure_subprocess(
                format,
                "extract_one",
                status,
                elapsed,
                before,
                before_children,
                after,
                after_children,
                0,
            ));
        }
    }
    results
}

/// Run a path-resolution-only benchmark. Each format uses its OWN
/// image file.
pub fn locate_one(
    images: &std::collections::HashMap<&str, PathBuf>,
    target_path: &str,
    iterations: usize,
    formats: &[&str],
) -> Vec<OperationResult> {
    let mut results = Vec::with_capacity(iterations * formats.len());

    for _ in 0..iterations {
        for &format in formats {
            let image = match images.get(format) {
                Some(p) => p,
                None => continue,
            };
            let before = ResourceSnapshot::now();
            let before_children = ResourceSnapshot::children();
            let start = Instant::now();
            let status: std::io::Result<std::process::ExitStatus> = match format {
                "limnifs" => {
                    let limni = std::env::current_exe()
                        .ok()
                        .and_then(|p| p.parent().map(|d| d.join("limni")))
                        .unwrap_or_else(|| PathBuf::from("limni"));
                    Command::new(&limni)
                        .args(["stat"])
                        .arg(image)
                        .arg(target_path)
                        .stdout(std::process::Stdio::null())
                        .status()
                }
                "tar+zstd" => Command::new("tar")
                    .args(["-tvf"])
                    .arg(image)
                    .arg(target_path.trim_start_matches('/'))
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status(),
                "dwarfs" | "squashfs" => {
                    // No clean CLI for path-only resolution; require FUSE.
                    // Skip — return a synthetic failure.
                    Ok(std::process::ExitStatus::default())
                }
                _ => continue,
            };
            let elapsed = start.elapsed();
            let after_children = ResourceSnapshot::children();
            let after = ResourceSnapshot::now();
            results.push(measure_subprocess(
                format,
                "locate_one",
                status,
                elapsed,
                before,
                before_children,
                after,
                after_children,
                0,
            ));
        }
    }
    results
}

/// Run a single full-file sequential read benchmark. This is the
/// realistic measurement for "random file access" workloads: each
/// read is a fresh `limni cat` invocation (cold cache per read for
/// fair comparison with FUSE-mounted alternatives).
///
/// We do N reads of M bytes each at random offsets, but each is a
/// single `limni cat --offset --length` subprocess. The per-call
/// cost is the metric; total time = N × `per_call`.
pub fn read_random(
    image: &Path,
    target_path: &str,
    file_size: u64,
    read_size: u64,
    num_reads: usize,
    iterations: usize,
) -> Vec<OperationResult> {
    let mut results = Vec::with_capacity(iterations);
    let limni = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("limni")))
        .unwrap_or_else(|| PathBuf::from("limni"));

    // Deterministic random offsets via splitmix64. Clamp to file size.
    let mut state: u64 = 0xA5A5_5A5A_5A5A_5A5A;
    let max_offset = file_size.saturating_sub(read_size);
    let offsets: Vec<u64> = (0..num_reads)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % max_offset.max(1)
        })
        .collect();

    for _ in 0..iterations {
        let before = ResourceSnapshot::now();
        let before_children = ResourceSnapshot::children();
        let start = Instant::now();
        let mut last_status: std::io::Result<std::process::ExitStatus> =
            Ok(std::process::ExitStatus::default());
        for &offset in &offsets {
            last_status = Command::new(&limni)
                .args(["cat"])
                .arg(image)
                .arg(target_path)
                .args(["--offset"])
                .arg(offset.to_string())
                .args(["--length"])
                .arg(read_size.to_string())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if !matches!(last_status, Ok(ref s) if s.success()) {
                break;
            }
        }
        let elapsed = start.elapsed();
        let after_children = ResourceSnapshot::children();
        let after = ResourceSnapshot::now();
        let mut r = measure_subprocess(
            "limnifs",
            "read_random",
            last_status,
            elapsed,
            before,
            before_children,
            after,
            after_children,
            read_size * num_reads as u64,
        );
        r.items_processed = num_reads as u64;
        results.push(r);
    }
    results
}

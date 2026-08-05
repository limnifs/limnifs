//! limnifs-bench — State-of-the-art Rust benchmark suite for `LimniFS`.
//!
//! Usage:
//!   limnifs-bench download --all
//!   limnifs-bench download --datasets php,gpt2
//!   limnifs-bench run --quick
//!   limnifs-bench run --all
//!   limnifs-bench run --category ai-model
//!   limnifs-bench run --datasets php,python

// Benchmark binary uses libc::getrusage to measure CPU+RSS.
// Allow unsafe in the resource module.
#![allow(unsafe_code)]
#![allow(warnings)]

mod datasets;
mod metrics;
mod report;
mod resource;
mod runners;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "limnifs-bench",
    about = "State-of-the-art benchmark suite for LimniFS"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Download {
        #[arg(long)]
        all: bool,
        #[arg(long, value_delimiter = ',')]
        datasets: Option<Vec<String>>,
    },
    Run {
        #[arg(long)]
        all: bool,
        #[arg(long)]
        quick: bool,
        #[arg(long)]
        category: Option<String>,
        #[arg(long, value_delimiter = ',')]
        datasets: Option<Vec<String>>,
        #[arg(long, default_value = "3")]
        iterations: usize,
        /// Comma-separated list of LimniFS profile names to exercise
        /// (e.g., `balanced,max-write,max-ratio`). Each profile runs
        /// its own create/verify/extract pass; external formats run
        /// once per dataset.
        #[arg(long, value_delimiter = ',')]
        profile: Option<Vec<String>>,
    },
}

fn main() {
    let cli = Cli::parse();
    let workspace = find_workspace();
    let paths = runners::WorkspacePaths::new(&workspace);

    match cli.command {
        Command::Download { all, datasets } => {
            let names: Vec<String> = if all {
                datasets::DATASETS
                    .iter()
                    .map(|d| d.name.to_string())
                    .collect()
            } else if let Some(ds) = datasets {
                ds
            } else {
                eprintln!("Specify --all or --datasets <names>");
                std::process::exit(1);
            };

            std::fs::create_dir_all(&paths.cache_dir).expect("create cache dir");
            for name in &names {
                if let Some(ds) = datasets::find(name) {
                    let cat = category_str(ds.category);
                    println!(
                        "[{cat}] {} (~{} MB) — {}",
                        ds.name, ds.approx_size_mb, ds.description
                    );
                    match datasets::ensure(ds, &paths.cache_dir) {
                        Ok(path) => println!("  Ready at {}", path.display()),
                        Err(e) => eprintln!("  Failed: {e}"),
                    }
                } else {
                    eprintln!("  Unknown dataset: {name}");
                }
            }
        }
        Command::Run {
            all,
            quick,
            category,
            datasets: ds_names,
            iterations,
            profile,
        } => {
            run_benchmarks(
                &workspace, &paths, all, quick, category, ds_names, iterations, profile,
            );
        }
    }
}

fn run_benchmarks(
    workspace: &std::path::Path,
    paths: &runners::WorkspacePaths,
    all: bool,
    quick: bool,
    category: Option<String>,
    ds_names: Option<Vec<String>>,
    iterations: usize,
    profiles: Option<Vec<String>>,
) {
    let iters = if quick { 1 } else { iterations };

    let selected: Vec<&datasets::Dataset> = if quick {
        datasets::DATASETS
            .iter()
            .filter(|d| d.category == datasets::Category::Synthetic)
            .collect()
    } else if all {
        datasets::DATASETS.iter().collect()
    } else if let Some(cat) = &category {
        let cat = match cat.as_str() {
            "source" => datasets::Category::Source,
            "ai-model" => datasets::Category::AiModel,
            "binary" => datasets::Category::Binary,
            "synthetic" => datasets::Category::Synthetic,
            other => {
                eprintln!("Unknown category: {other}");
                std::process::exit(1);
            }
        };
        datasets::DATASETS
            .iter()
            .filter(|d| d.category == cat)
            .collect()
    } else if let Some(names) = ds_names {
        names.iter().filter_map(|n| datasets::find(n)).collect()
    } else {
        eprintln!("Specify --all, --quick, --category, or --datasets");
        std::process::exit(1);
    };

    if selected.is_empty() {
        eprintln!("No datasets selected. Use 'limnifs-bench download --all' first.");
        std::process::exit(1);
    }

    std::fs::create_dir_all(&paths.cache_dir).expect("create cache dir");
    std::fs::create_dir_all(&paths.work_dir).expect("create work dir");

    let mut all_summaries: Vec<metrics::BenchmarkSummary> = Vec::new();
    let sep = "=".repeat(70);

    for ds in &selected {
        println!("\n{sep}");
        let cat = category_str(ds.category);
        println!("[{cat}] {} — {}", ds.name, ds.description);
        println!("{sep}");

        let source = match datasets::ensure(ds, &paths.cache_dir) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("  Failed to prepare: {e}");
                continue;
            }
        };

        let source_dir = find_source_dir(&source);
        let input_size = datasets::dir_size(&source_dir);
        let file_count = datasets::file_count(&source_dir);
        println!(
            "  Input: {:.1} MB, {} files",
            input_size as f64 / 1_048_576.0,
            file_count
        );

        let ds_work = paths.work_dir.join(ds.name);
        std::fs::create_dir_all(&ds_work).expect("create work dir");

        // Resolve LimniFS profiles to exercise. Default = ["balanced"].
        let profile_names: Vec<String> = match &profiles {
            Some(p) if !p.is_empty() => p.clone(),
            _ => vec!["balanced".to_string()],
        };

        let primary_profile = profile_names[0].clone();

        // LimniFS create + verify + extract, once per requested profile.
        for profile_name in &profile_names {
            println!("\n  --- profile: {profile_name} ---");
            print!("  [limnifs:{profile_name}] create… ");
            let results = runners::limnifs_create(&source_dir, &ds_work, iters, profile_name);
            if let Some(s) = metrics::BenchmarkSummary::from_results(
                ds.name,
                ds.category,
                &results,
                input_size,
            ) {
                println!(
                    "{:.3}s, {:.1} MB, {:.1}%",
                    s.median_seconds, s.output_size_mb, s.ratio_percent
                );
                all_summaries.push(s);
            } else {
                println!("FAILED");
            }

            let limni_image = ds_work.join(format!("limnifs-{profile_name}.lim"));
            if limni_image.exists() {
                print!("  [limnifs:{profile_name}] verify… ");
                let results = runners::limnifs_verify(&limni_image, iters, profile_name);
                if let Some(s) = metrics::BenchmarkSummary::from_results(
                    ds.name,
                    ds.category,
                    &results,
                    input_size,
                ) {
                    println!("{:.3}s", s.median_seconds);
                    all_summaries.push(s);
                }

                print!("  [limnifs:{profile_name}] extract… ");
                let results =
                    runners::limnifs_extract(&limni_image, &ds_work, iters, input_size, profile_name);
                if let Some(s) = metrics::BenchmarkSummary::from_results(
                    ds.name,
                    ds.category,
                    &results,
                    input_size,
                ) {
                    println!("{:.3}s ({:.0} MB/s)", s.median_seconds, s.throughput_mbps);
                    all_summaries.push(s);
                }
            }
        }

        // DwarFS
        if which("mkdwarfs") {
            print!("  [DwarFS] create… ");
            let results = runners::dwarfs_create(&source_dir, &ds_work, iters);
            if let Some(s) =
                metrics::BenchmarkSummary::from_results(ds.name, ds.category, &results, input_size)
            {
                println!("{:.3}s, {:.1} MB", s.median_seconds, s.output_size_mb);
                all_summaries.push(s);

                let dwarfs_image = ds_work.join("test.dwarfs");
                if dwarfs_image.exists() && which("dwarfsextract") {
                    print!("  [DwarFS] extract… ");
                    let results =
                        runners::dwarfs_extract(&dwarfs_image, &ds_work, iters, input_size);
                    if let Some(s) = metrics::BenchmarkSummary::from_results(
                        ds.name,
                        ds.category,
                        &results,
                        input_size,
                    ) {
                        println!("{:.3}s", s.median_seconds);
                        all_summaries.push(s);
                    }
                }
            } else {
                println!("FAILED");
            }
        } else {
            println!("  [DwarFS] not installed — skipping");
        }

        // SquashFS
        if which("mksquashfs") {
            print!("  [SquashFS] create… ");
            let results = runners::squashfs_create(&source_dir, &ds_work, iters);
            if let Some(s) =
                metrics::BenchmarkSummary::from_results(ds.name, ds.category, &results, input_size)
            {
                println!("{:.3}s, {:.1} MB", s.median_seconds, s.output_size_mb);
                all_summaries.push(s);

                let sqfs_image = ds_work.join("squashfs.squashfs");
                if sqfs_image.exists() && which("unsquashfs") {
                    print!("  [SquashFS] extract… ");
                    let results =
                        runners::squashfs_extract(&sqfs_image, &ds_work, iters, input_size);
                    if let Some(s) = metrics::BenchmarkSummary::from_results(
                        ds.name,
                        ds.category,
                        &results,
                        input_size,
                    ) {
                        println!("{:.3}s", s.median_seconds);
                        all_summaries.push(s);
                    }
                }
            } else {
                println!("FAILED");
            }
        } else {
            println!("  [SquashFS] not installed — skipping");
        }

        // tar + zstd
        if which("tar") {
            print!("  [tar+zstd] create… ");
            let results = runners::tar_zstd_create(&source_dir, &ds_work, iters);
            if let Some(s) =
                metrics::BenchmarkSummary::from_results(ds.name, ds.category, &results, input_size)
            {
                println!("{:.3}s, {:.1} MB", s.median_seconds, s.output_size_mb);
                all_summaries.push(s);

                let tar_archive = ds_work.join("test.tar.zst");
                if tar_archive.exists() {
                    print!("  [tar+zstd] extract… ");
                    let results =
                        runners::tar_zstd_extract(&tar_archive, &ds_work, iters, input_size);
                    if let Some(s) = metrics::BenchmarkSummary::from_results(
                        ds.name,
                        ds.category,
                        &results,
                        input_size,
                    ) {
                        println!("{:.3}s", s.median_seconds);
                        all_summaries.push(s);
                    }
                }
            } else {
                println!("FAILED");
            }
        }

        // Single-file operations: extract_one, locate_one, read_random.
        // Pick a target file from the source tree (first file > 1 KiB).
        if let Some(target) = pick_target_file(&source_dir) {
            let target_rel = target
                .strip_prefix(&source_dir)
                .unwrap_or(&target)
                .to_string_lossy()
                .into_owned();
            let target_str = format!("/{}", target_rel.trim_start_matches('/'));
            let file_size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);

            // Build per-format image path map.
            let mut images: std::collections::HashMap<&str, std::path::PathBuf> =
                std::collections::HashMap::new();
            let limnifs_img = ds_work.join(format!("limnifs-{primary_profile}.lim"));
            let dwarfs_img = ds_work.join("test.dwarfs");
            let squashfs_img = ds_work.join("squashfs.squashfs");
            let tar_img = ds_work.join("test.tar.zst");
            if limnifs_img.exists() {
                images.insert("limnifs", limnifs_img.clone());
            }
            if dwarfs_img.exists() {
                images.insert("dwarfs", dwarfs_img);
            }
            if squashfs_img.exists() {
                images.insert("squashfs", squashfs_img);
            }
            if tar_img.exists() {
                images.insert("tar+zstd", tar_img);
            }

            let formats_present: Vec<&str> = images.keys().copied().collect();
            if !formats_present.is_empty() {
                print!("  [all] extract_one ({target_str})… ");
                let results =
                    runners::extract_one(&images, &target_str, &ds_work, iters, &formats_present);
                let limnifs_med = results
                    .iter()
                    .filter(|r| r.success && r.format == "limnifs")
                    .map(|r| r.elapsed_secs)
                    .collect::<Vec<_>>();
                if let Some(m) = median_of(&limnifs_med) {
                    println!("{:.3} ms (limnifs median)", m * 1000.0);
                } else {
                    println!("(see report)");
                }
                for r in &results {
                    if r.success {
                        let summary = metrics::BenchmarkSummary::from_results(
                            ds.name,
                            ds.category,
                            std::slice::from_ref(r),
                            0,
                        );
                        if let Some(s) = summary {
                            all_summaries.push(s);
                        }
                    }
                }
            }

            // locate_one: only limnifs and tar+zstd (need stat-like CLI).
            let mut locate_formats: Vec<&str> = Vec::new();
            if images.contains_key("limnifs") {
                locate_formats.push("limnifs");
            }
            if images.contains_key("tar+zstd") {
                locate_formats.push("tar+zstd");
            }
            if !locate_formats.is_empty() {
                print!("  [limnifs,tar] locate_one ({target_str})… ");
                let results = runners::locate_one(&images, &target_str, iters, &locate_formats);
                let limnifs_med = results
                    .iter()
                    .filter(|r| r.success && r.format == "limnifs")
                    .map(|r| r.elapsed_secs)
                    .collect::<Vec<_>>();
                if let Some(m) = median_of(&limnifs_med) {
                    println!("{:.3} ms (limnifs median)", m * 1000.0);
                } else {
                    println!("(see report)");
                }
                for r in &results {
                    if r.success {
                        let summary = metrics::BenchmarkSummary::from_results(
                            ds.name,
                            ds.category,
                            std::slice::from_ref(r),
                            0,
                        );
                        if let Some(s) = summary {
                            all_summaries.push(s);
                        }
                    }
                }
            }

            // read_random: limnifs only (needs offset+length API).
            // 100 reads of 4 KiB = 400 KiB total. Reasonable per-call
            // latency measurement without taking minutes.
            if images.contains_key("limnifs") {
                print!("  [limnifs] read_random (100 × 4 KiB)… ");
                let results =
                    runners::read_random(&limnifs_img, &target_str, file_size, 4096, 100, iters);
                let med = results
                    .iter()
                    .filter(|r| r.success)
                    .map(|r| r.elapsed_secs)
                    .collect::<Vec<_>>();
                if let Some(m) = median_of(&med) {
                    let per_call_ms = m * 1000.0 / 100.0;
                    println!("{per_call_ms:.3} ms/call ({m:.3}s total)");
                } else {
                    println!("FAILED");
                }
                if let Some(s) = metrics::BenchmarkSummary::from_results(
                    ds.name,
                    ds.category,
                    &results,
                    4096 * 100,
                ) {
                    all_summaries.push(s);
                }
            }
        } else {
            println!("  [single-file ops] no target file ≥ 1 KiB found — skipping");
        }
    }

    // Generate report
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let report = report::FullReport {
        date: format!("epoch:{now}"),
        platform: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        iterations: iters,
        results: all_summaries,
    };

    let output_dir = workspace.join("benchmarks").join("results");
    std::fs::create_dir_all(&output_dir).expect("create results dir");

    let json_path = output_dir.join(format!("bench_{now}.json"));
    let md_path = output_dir.join(format!("bench_{now}.md"));
    std::fs::write(&json_path, report.to_json()).expect("write json");
    std::fs::write(&md_path, report.to_markdown()).expect("write md");

    println!("\n{sep}");
    println!("BENCHMARK COMPLETE");
    println!("  JSON: {}", json_path.display());
    println!("  Markdown: {}", md_path.display());
    println!("{sep}");
}

fn category_str(cat: datasets::Category) -> &'static str {
    match cat {
        datasets::Category::Source => "source",
        datasets::Category::AiModel => "ai-model",
        datasets::Category::Binary => "binary",
        datasets::Category::Synthetic => "synthetic",
    }
}

fn find_workspace() -> PathBuf {
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        if dir.join("Cargo.toml").exists() {
            return dir;
        }
        if !dir.pop() {
            return PathBuf::from(".");
        }
    }
}

fn find_source_dir(base: &std::path::Path) -> std::path::PathBuf {
    if let Ok(entries) = std::fs::read_dir(base) {
        let dirs: Vec<_> = entries.flatten().filter(|e| e.path().is_dir()).collect();
        if dirs.len() == 1 {
            return dirs[0].path();
        }
    }
    base.to_path_buf()
}

fn which(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(tool).is_file()))
}

/// Median of a list of f64. Empty → None.
fn median_of(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    Some(if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    })
}

/// Pick a benchmark target file from a directory tree.
/// Prefers files > 1 KiB and < 10 MiB so random-read benchmarks
/// have meaningful offsets to choose from.
fn pick_target_file(base: &std::path::Path) -> Option<std::path::PathBuf> {
    fn walk(base: &std::path::Path) -> Option<std::path::PathBuf> {
        for entry in std::fs::read_dir(base).ok()?.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if let Some(found) = walk(&p) {
                    return Some(found);
                }
            } else if let Ok(meta) = entry.metadata() {
                let size = meta.len();
                if (1024..=10 * 1024 * 1024).contains(&size) {
                    return Some(p);
                }
            }
        }
        None
    }
    walk(base)
}

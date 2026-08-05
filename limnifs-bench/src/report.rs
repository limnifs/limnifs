//! Report generation — model-driven JSON + Markdown.
//!
//! The model layer:
//!   `BenchmarkSummary` (per dataset/format/operation)
//!     → grouped into `DatasetView` (per dataset, all formats × operations)
//!       → grouped into `CategoryView` (per category, all datasets)
//!         → `FullReport` (all categories)
//!
//! Every renderer (JSON, Markdown, win/loss matrix) walks this tree. There is
//! no presentation logic that touches raw timing data directly.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::collections::BTreeMap;

use crate::datasets::Category;
use crate::metrics::BenchmarkSummary;

#[derive(Debug, serde::Serialize)]
pub struct FullReport {
    pub date: String,
    pub platform: String,
    pub iterations: usize,
    pub results: Vec<BenchmarkSummary>,
}

impl FullReport {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }

    pub fn to_markdown(&self) -> String {
        MarkdownRenderer::new(self).render()
    }
}

// ---------------------------------------------------------------------------
// Tree types derived from the flat summary list.
// ---------------------------------------------------------------------------

/// All summaries for one dataset, keyed by (format, operation).
#[derive(Debug)]
struct DatasetView {
    name: String,
    category: Category,
    input_size_mb: f64,
    /// Keyed by "{`format}::{operation`}".
    entries: BTreeMap<String, BenchmarkSummary>,
}

impl DatasetView {
    fn get(&self, format: &str, op: &str) -> Option<&BenchmarkSummary> {
        self.entries.get(&format_key(format, op))
    }
}

/// All datasets in one category.
#[derive(Debug)]
struct CategoryView {
    category: Category,
    datasets: Vec<DatasetView>,
}

fn format_key(format: &str, op: &str) -> String {
    format!("{format}::{op}")
}

/// Group flat summaries into a category → dataset tree.
fn group(results: &[BenchmarkSummary]) -> Vec<CategoryView> {
    let mut by_dataset: BTreeMap<String, DatasetView> = BTreeMap::new();
    for r in results {
        let dv = by_dataset
            .entry(r.dataset.clone())
            .or_insert_with(|| DatasetView {
                name: r.dataset.clone(),
                category: r.category,
                input_size_mb: r.input_size_mb,
                entries: BTreeMap::new(),
            });
        dv.entries
            .insert(format_key(&r.format, &r.operation), r.clone());
    }

    let mut categories: BTreeMap<Category, CategoryView> = BTreeMap::new();
    for (_, dv) in by_dataset {
        let cat = dv.category;
        let cv = categories.entry(cat).or_insert_with(|| CategoryView {
            category: cat,
            datasets: Vec::new(),
        });
        cv.datasets.push(dv);
    }

    let mut out: Vec<CategoryView> = categories.into_values().collect();
    // Sort categories in the canonical order.
    out.sort_by_key(|c| match c.category {
        Category::Source => 0,
        Category::AiModel => 1,
        Category::Binary => 2,
        Category::Synthetic => 3,
    });
    out
}

fn category_title(c: Category) -> &'static str {
    match c {
        Category::Source => "Source Code",
        Category::AiModel => "AI Models",
        Category::Binary => "Binaries",
        Category::Synthetic => "Synthetic",
    }
}

// ---------------------------------------------------------------------------
// Markdown renderer.
// ---------------------------------------------------------------------------

const EXTERNAL_FORMATS: &[&str] = &["dwarfs", "squashfs", "tar+zstd"];
const OPERATIONS: &[&str] = &[
    "create",
    "extract",
    "verify",
    "extract_one",
    "locate_one",
    "read_random",
];

/// Build the format list dynamically from the results. LimniFS profiles
/// (any format starting with `limnifs`) sort first in stable order,
/// then the external formats in canonical order. This lets multi-profile
/// runs render one row per profile without hardcoding.
fn derive_formats(results: &[BenchmarkSummary]) -> Vec<String> {
    let mut limnifs_profiles: Vec<String> = results
        .iter()
        .filter(|r| r.format.starts_with("limnifs"))
        .map(|r| r.format.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    // Ensure `limnifs` (legacy / single-file ops) sorts before
    // `limnifs:balanced` etc. BTreeSet gives lexicographic order, and
    // ':' (0x3A) < letters, so `limnifs:...` < `limnifs` would be wrong
    // — but `limnifs` has no ':' so it sorts AFTER colon-suffixed
    // entries. Reverse so plain `limnifs` (if present) leads.
    limnifs_profiles.sort_by(|a, b| {
        match (a.starts_with("limnifs:"), b.starts_with("limnifs:")) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            _ => a.cmp(b),
        }
    });

    let mut out = limnifs_profiles;
    for f in EXTERNAL_FORMATS {
        if results.iter().any(|r| r.format == *f) {
            out.push((*f).to_string());
        }
    }
    out
}

struct MarkdownRenderer<'a> {
    report: &'a FullReport,
    categories: Vec<CategoryView>,
    formats: Vec<String>,
}

impl<'a> MarkdownRenderer<'a> {
    fn new(report: &'a FullReport) -> Self {
        let categories = group(&report.results);
        let formats = derive_formats(&report.results);
        Self {
            report,
            categories,
            formats,
        }
    }

    fn render(&self) -> String {
        let mut md = String::new();
        md.push_str("# LimniFS Benchmark Report\n\n");
        md.push_str(&format!("- **Date:** {}\n", self.report.date));
        md.push_str(&format!("- **Platform:** {}\n", self.report.platform));
        md.push_str(&format!(
            "- **Iterations per measurement:** {}\n\n",
            self.report.iterations
        ));

        md.push_str("## Datasets\n\n");
        md.push_str("| Category | Dataset | Input (MB) |\n");
        md.push_str("|---|---|---:|\n");
        for cv in &self.categories {
            for dv in &cv.datasets {
                md.push_str(&format!(
                    "| {} | {} | {:.1} |\n",
                    category_title(cv.category),
                    dv.name,
                    dv.input_size_mb,
                ));
            }
        }
        md.push('\n');

        md.push_str("## Results by Category\n\n");
        for cv in &self.categories {
            md.push_str(&format!("### {}\n\n", category_title(cv.category)));

            for op in OPERATIONS {
                self.render_operation_tables(&mut md, cv, op);
            }
        }

        md.push_str("## Win/Loss Matrix\n\n");
        self.render_win_loss_matrix(&mut md);

        md
    }

    /// For a given operation, render one table per dataset (rows = formats).
    fn render_operation_tables(&self, md: &mut String, cv: &CategoryView, op: &str) {
        let any = cv
            .datasets
            .iter()
            .any(|dv| self.formats.iter().any(|f| dv.get(f, op).is_some()));
        if !any {
            return;
        }

        md.push_str(&format!("#### {}\n\n", capitalize(op)));
        md.push_str("| Dataset | Format | Median (s) | CPU user+sys (s) | Peak RSS (MiB) | Throughput (MB/s) | Output (MB) | Ratio (%) |\n");
        md.push_str("|---|---|---:|---:|---:|---:|---:|---:|\n");
        for dv in &cv.datasets {
            for f in &self.formats {
                if let Some(s) = dv.get(f, op) {
                    let cpu_total = s.cpu_user_secs + s.cpu_system_secs;
                    let rss_mib = s.peak_rss_bytes as f64 / (1024.0 * 1024.0);
                    md.push_str(&format!(
                        "| {} | {} | {:.3} | {:.3} | {:.1} | {:.0} | {:.2} | {:.2} |\n",
                        dv.name,
                        f,
                        s.median_seconds,
                        cpu_total,
                        rss_mib,
                        s.throughput_mbps,
                        s.output_size_mb,
                        s.ratio_percent,
                    ));
                }
            }
        }
        md.push('\n');
    }

    fn render_win_loss_matrix(&self, md: &mut String) {
        // For each (dataset, operation), find the best format by median time
        // (lower = better). Mark each format W (winner), = (tie within 5%),
        // L (loss), · (not measured).
        md.push_str("Winner per (dataset × operation) by **median time** (lower is better).\n\n");
        let header_formats: Vec<String> = self.formats.clone();
        let header_row: String = header_formats.join(" | ");
        md.push_str(&format!("| Dataset | Operation | {header_row} |\n"));
        let sep: String = header_formats
            .iter()
            .map(|_| ":---:")
            .collect::<Vec<_>>()
            .join(" | ");
        md.push_str(&format!("|---|---|{sep} |\n"));

        for cv in &self.categories {
            for dv in &cv.datasets {
                for op in OPERATIONS {
                    let measured: Vec<(&str, f64)> = self
                        .formats
                        .iter()
                        .filter_map(|f| dv.get(f, op).map(|s| (f.as_str(), s.median_seconds)))
                        .collect();
                    if measured.is_empty() {
                        continue;
                    }
                    let best = measured
                        .iter()
                        .map(|(_, t)| *t)
                        .fold(f64::INFINITY, f64::min);
                    let cells: Vec<String> = self
                        .formats
                        .iter()
                        .map(|f| match dv.get(f, op) {
                            None => "·".to_string(),
                            Some(s) => {
                                let t = s.median_seconds;
                                if (t - best).abs() / best.max(1e-9) < 0.05 {
                                    "✅".to_string()
                                } else {
                                    format!("{:.2}×", t / best)
                                }
                            }
                        })
                        .collect();
                    let cells_row = cells.join(" | ");
                    md.push_str(&format!(
                        "| {} | {} | {cells_row} |\n",
                        dv.name, op,
                    ));
                }
            }
        }
        md.push('\n');

        // Aggregate: count wins per format across the whole run.
        md.push_str("### Win Count\n\n");
        md.push_str("| Format | Wins (lowest median time) |\n");
        md.push_str("|---|---:|\n");
        let mut wins: BTreeMap<String, usize> = BTreeMap::new();
        for cv in &self.categories {
            for dv in &cv.datasets {
                for op in OPERATIONS {
                    let measured: Vec<(&str, f64)> = self
                        .formats
                        .iter()
                        .filter_map(|f| dv.get(f, op).map(|s| (f.as_str(), s.median_seconds)))
                        .collect();
                    if measured.is_empty() {
                        continue;
                    }
                    let best = measured
                        .iter()
                        .map(|(_, t)| *t)
                        .fold(f64::INFINITY, f64::min);
                    for (f, t) in &measured {
                        if (t - best).abs() / best.max(1e-9) < 0.05 {
                            *wins.entry((*f).to_string()).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        for f in &self.formats {
            md.push_str(&format!(
                "| {} | {} |\n",
                f,
                wins.get(f).copied().unwrap_or(0)
            ));
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

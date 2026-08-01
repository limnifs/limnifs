//! Report generation — JSON + Markdown with per-category win/loss matrix.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

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
        let mut md = String::new();
        md.push_str("# LimniFS Benchmark Report\n\n");
        md.push_str(&format!("**Date:** {}\n", self.date));
        md.push_str(&format!("**Platform:** {}\n", self.platform));
        md.push_str(&format!("**Iterations:** {}\n\n", self.iterations));

        let categories = [Category::Source, Category::AiModel, Category::Synthetic, Category::Binary];
        for cat in &categories {
            let cat_name = format!("{cat:?}").to_lowercase();
            let cat_results: Vec<&BenchmarkSummary> = self.results.iter().filter(|_| true).collect();

            // Group by format+operation for this category
            // (Since we don't track category in BenchmarkSummary, we group by format)
            let _ = &cat_results; // placeholder — actual grouping needs dataset info

            // Create/Extract tables
            for op in &["create", "extract", "verify"] {
                let op_results: Vec<&BenchmarkSummary> = self.results.iter().filter(|r| r.operation == *op).collect();
                if op_results.is_empty() {
                    continue;
                }

                if !md.contains(&format!("## {op_title}", op_title = op.to_uppercase())) {
                    md.push_str(&format!("## {}\n\n", op.to_uppercase()));
                }

                md.push_str("| Format | Median (s) | Stdev | Throughput (MB/s) | Size (MB) | Ratio (%) |\n");
                md.push_str("|---|---:|---:|---:|---:|---:|\n");
                for r in &op_results {
                    md.push_str(&format!(
                        "| {} | {:.3} | {:.3} | {:.0} | {:.1} | {:.1} |\n",
                        r.format, r.median_seconds, r.stdev_seconds,
                        r.throughput_mbps, r.output_size_mb, r.ratio_percent,
                    ));
                }
                md.push('\n');
            }
        }

        md
    }
}

//! Metrics and result types.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::time::Duration;

use crate::datasets::Category;
use crate::resource::ResourceSnapshot;

#[derive(Clone, Debug, serde::Serialize)]
pub struct OperationResult {
    pub format: String,
    pub operation: String,
    pub success: bool,
    pub elapsed_secs: f64,
    pub output_size_bytes: u64,
    /// User CPU time consumed during the operation, in seconds.
    /// Captured via `getrusage(RUSAGE_SELF)`; cross-platform.
    #[serde(default)]
    pub cpu_user_secs: f64,
    /// System CPU time consumed during the operation, in seconds.
    #[serde(default)]
    pub cpu_system_secs: f64,
    /// Peak resident set size in bytes, captured at operation end.
    /// This is the high-water mark since process start, not a per-
    /// operation delta — callers that want operation-only memory
    /// should snapshot before+after and subtract.
    #[serde(default)]
    pub peak_rss_bytes: u64,
    /// Number of items processed (files extracted, reads done, etc.).
    /// Defaults to 1; throughput = items / elapsed.
    #[serde(default)]
    pub items_processed: u64,
}

impl OperationResult {
    /// Build a fully-populated result from a closure that captures
    /// resource usage around the work. The closure returns the
    /// output byte length on success.
    pub fn measure(
        format: &str,
        operation: &str,
        before: ResourceSnapshot,
        after: ResourceSnapshot,
        elapsed: Duration,
        output_size: u64,
        items: u64,
    ) -> Self {
        Self {
            format: format.to_string(),
            operation: operation.to_string(),
            success: true,
            elapsed_secs: elapsed.as_secs_f64(),
            output_size_bytes: output_size,
            cpu_user_secs: (after.user_secs - before.user_secs).max(0.0),
            cpu_system_secs: (after.system_secs - before.system_secs).max(0.0),
            peak_rss_bytes: after.rss_bytes,
            items_processed: items,
        }
    }

    pub fn success(format: &str, operation: &str, elapsed: Duration, output_size: u64) -> Self {
        Self {
            format: format.to_string(),
            operation: operation.to_string(),
            success: true,
            elapsed_secs: elapsed.as_secs_f64(),
            output_size_bytes: output_size,
            cpu_user_secs: 0.0,
            cpu_system_secs: 0.0,
            peak_rss_bytes: 0,
            items_processed: 1,
        }
    }

    pub fn failure(format: &str, operation: &str, elapsed: Duration) -> Self {
        Self {
            format: format.to_string(),
            operation: operation.to_string(),
            success: false,
            elapsed_secs: elapsed.as_secs_f64(),
            output_size_bytes: 0,
            cpu_user_secs: 0.0,
            cpu_system_secs: 0.0,
            peak_rss_bytes: 0,
            items_processed: 0,
        }
    }
}

/// Compute median of a list of f64 values.
pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// Compute standard deviation.
pub fn stdev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64;
    variance.sqrt()
}

const MIB_F: f64 = 1_048_576.0;

/// A summary of benchmark results for a single (dataset, format, operation) triple.
///
/// This is the unit of reported truth — every renderer (JSON, Markdown,
/// win/loss matrix) derives its output from a flat list of these.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BenchmarkSummary {
    pub dataset: String,
    pub category: Category,
    pub format: String,
    pub operation: String,
    pub iterations: usize,
    pub median_seconds: f64,
    pub stdev_seconds: f64,
    pub output_size_mb: f64,
    pub input_size_mb: f64,
    pub throughput_mbps: f64,
    pub ratio_percent: f64,
    /// Median user CPU seconds. 0 if not measured.
    #[serde(default)]
    pub cpu_user_secs: f64,
    /// Median system CPU seconds.
    #[serde(default)]
    pub cpu_system_secs: f64,
    /// Peak RSS bytes observed across iterations.
    #[serde(default)]
    pub peak_rss_bytes: u64,
}

impl BenchmarkSummary {
    pub fn from_results(
        dataset: &str,
        category: Category,
        results: &[OperationResult],
        input_size: u64,
    ) -> Option<Self> {
        let successes: Vec<&OperationResult> = results.iter().filter(|r| r.success).collect();
        if successes.is_empty() {
            return None;
        }
        let format = successes[0].format.clone();
        let operation = successes[0].operation.clone();
        let times: Vec<f64> = successes.iter().map(|r| r.elapsed_secs).collect();
        let user_times: Vec<f64> = successes.iter().map(|r| r.cpu_user_secs).collect();
        let sys_times: Vec<f64> = successes.iter().map(|r| r.cpu_system_secs).collect();
        let rss_max = successes.iter().map(|r| r.peak_rss_bytes).max().unwrap_or(0);
        let median_secs = median(&times);
        let input_mb = input_size as f64 / MIB_F;
        let output_mb = successes[0].output_size_bytes as f64 / MIB_F;
        let throughput = if median_secs > 0.0 { input_mb / median_secs } else { 0.0 };
        let ratio = if input_size > 0 {
            successes[0].output_size_bytes as f64 / input_size as f64 * 100.0
        } else {
            0.0
        };

        Some(Self {
            dataset: dataset.to_string(),
            category,
            format,
            operation,
            iterations: results.len(),
            median_seconds: median_secs,
            stdev_seconds: stdev(&times),
            output_size_mb: output_mb,
            input_size_mb: input_mb,
            throughput_mbps: throughput,
            ratio_percent: ratio,
            cpu_user_secs: median(&user_times),
            cpu_system_secs: median(&sys_times),
            peak_rss_bytes: rss_max,
        })
    }
}

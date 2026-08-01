//! Metrics and result types.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

use std::time::Duration;

#[derive(Clone, Debug, serde::Serialize)]
pub struct OperationResult {
    pub format: String,
    pub operation: String,
    pub success: bool,
    pub elapsed_secs: f64,
    pub output_size_bytes: u64,
}

impl OperationResult {
    pub fn success(format: &str, operation: &str, elapsed: Duration, output_size: u64) -> Self {
        Self {
            format: format.to_string(),
            operation: operation.to_string(),
            success: true,
            elapsed_secs: elapsed.as_secs_f64(),
            output_size_bytes: output_size,
        }
    }

    pub fn failure(format: &str, operation: &str, elapsed: Duration) -> Self {
        Self {
            format: format.to_string(),
            operation: operation.to_string(),
            success: false,
            elapsed_secs: elapsed.as_secs_f64(),
            output_size_bytes: 0,
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

/// A summary of benchmark results for a single (format, operation) pair.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BenchmarkSummary {
    pub format: String,
    pub operation: String,
    pub iterations: usize,
    pub median_seconds: f64,
    pub stdev_seconds: f64,
    pub output_size_mb: f64,
    pub input_size_mb: f64,
    pub throughput_mbps: f64,
    pub ratio_percent: f64,
}

impl BenchmarkSummary {
    pub fn from_results(results: &[OperationResult], input_size: u64) -> Option<Self> {
        let successes: Vec<&OperationResult> = results.iter().filter(|r| r.success).collect();
        if successes.is_empty() {
            return None;
        }
        let format = successes[0].format.clone();
        let operation = successes[0].operation.clone();
        let times: Vec<f64> = successes.iter().map(|r| r.elapsed_secs).collect();
        let median_secs = median(&times);
        let input_mb = input_size as f64 / 1_048_576.0;
        let output_mb = successes[0].output_size_bytes as f64 / 1_048_576.0;
        let throughput = if median_secs > 0.0 { input_mb / median_secs } else { 0.0 };
        let ratio = if input_size > 0 {
            successes[0].output_size_bytes as f64 / input_size as f64 * 100.0
        } else {
            0.0
        };

        Some(Self {
            format,
            operation,
            iterations: results.len(),
            median_seconds: median_secs,
            stdev_seconds: stdev(&times),
            output_size_mb: output_mb,
            input_size_mb: input_mb,
            throughput_mbps: throughput,
            ratio_percent: ratio,
        })
    }
}

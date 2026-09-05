//! Progress reporting seam.
//!
//! OCP: the pipeline emits events through this module and never
//! knows about terminals, rates, or formatting; consumers install
//! any [`ProgressSink`] without the writer changing. MECE: progress
//! lives here and nowhere else. The registry is process-wide
//! (matching the writer's existing thread-local cache precedent)
//! and optional — absent a sink, `emit_file` is a single relaxed
//! load on the fast path.

use std::path::Path;
use std::sync::Arc;

/// Receives progress events from the write pipeline.
pub trait ProgressSink: Send + Sync {
    /// A file was accepted for packing (after stat, before
    /// compression completes).
    fn on_file(&self, path: &Path, bytes: u64);
}

impl<F> ProgressSink for F
where
    F: Fn(&Path, u64) + Send + Sync,
{
    fn on_file(&self, path: &Path, bytes: u64) {
        self(path, bytes)
    }
}

static SINK: std::sync::RwLock<Option<Arc<dyn ProgressSink>>> = std::sync::RwLock::new(None);

/// Install the process-wide sink (replaces any previous one).
pub fn set_sink(sink: Arc<dyn ProgressSink>) {
    *SINK.write().expect("progress sink lock poisoned") = Some(sink);
}

/// Remove the process-wide sink.
pub fn clear_sink() {
    *SINK.write().expect("progress sink lock poisoned") = None;
}

/// Emit a file event; a no-op when no sink is installed.
pub fn emit_file(path: &Path, bytes: u64) {
    // Clone the Arc out of the lock so sink code never runs under
    // it (an emitting sink must not deadlock a concurrent set_sink).
    let sink = SINK.read().expect("progress sink lock poisoned").clone();
    if let Some(sink) = sink {
        sink.on_file(path, bytes);
    }
}

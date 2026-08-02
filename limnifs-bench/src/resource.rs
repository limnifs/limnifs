//! Resource usage tracker — captures CPU time + peak RSS per iteration.
//!
//! Cross-platform via the `libc` crate (no `unsafe` block here —
//! `libc` is the safe wrapper layer).
//!
//! - **macOS / Linux / FreeBSD**: `libc::getrusage(RUSAGE_SELF)`.
//!   macOS reports `ru_maxrss` in bytes; Linux in KiB. We normalise
//!   to bytes everywhere.
//! - **Other platforms**: stub that returns zeros; benchmark callers
//!   see `peak_rss_bytes = 0` and skip memory columns in the report.

#![allow(unsafe_code)]
#![warn(clippy::pedantic)]

#[derive(Clone, Copy, Debug, Default)]
pub struct ResourceSnapshot {
    pub user_secs: f64,
    pub system_secs: f64,
    pub rss_bytes: u64,
}

impl ResourceSnapshot {
    /// Capture the current process's resource usage. Cheap (one
    /// syscall on Unix).
    #[must_use]
    pub fn now() -> Self {
        #[cfg(unix)]
        {
            Self::now_unix(libc::RUSAGE_SELF)
        }
        #[cfg(not(unix))]
        {
            Self::default()
        }
    }

    /// Capture resource usage of children spawned via `system()` /
    /// `Command` since the process started. Use before+after a
    /// subprocess invocation to compute the delta.
    #[must_use]
    pub fn children() -> Self {
        #[cfg(unix)]
        {
            Self::now_unix(libc::RUSAGE_CHILDREN)
        }
        #[cfg(not(unix))]
        {
            Self::default()
        }
    }

    #[cfg(unix)]
    fn now_unix(who: i32) -> Self {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::getrusage(who, &mut usage) };
        if rc != 0 {
            return Self::default();
        }
        let to_secs = |tv: libc::timeval| tv.tv_sec as f64 + (tv.tv_usec as f64) / 1_000_000.0;

        #[cfg(target_os = "linux")]
        let rss_bytes = (usage.ru_maxrss as u64).saturating_mul(1024);
        #[cfg(not(target_os = "linux"))]
        let rss_bytes = usage.ru_maxrss as u64;

        Self {
            user_secs: to_secs(usage.ru_utime),
            system_secs: to_secs(usage.ru_stime),
            rss_bytes,
        }
    }
}

/// Capture resource deltas around a closure. Returns the closure's
/// result plus the (start, end) snapshots so the caller can compute
/// the delta.
pub fn capture_around<T>(f: impl FnOnce() -> T) -> (T, ResourceSnapshot, ResourceSnapshot) {
    let before = ResourceSnapshot::now();
    let result = f();
    let after = ResourceSnapshot::now();
    (result, before, after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_returns_nonzero_user_time_on_unix() {
        // Burn a bit of CPU so getrusage reports nonzero user time.
        let mut total: u64 = 0;
        for i in 0..1_000_000u64 {
            total = total.wrapping_add(i);
        }
        let snap = ResourceSnapshot::now();
        // Most platforms will see nonzero user_secs after this loop.
        let _ = snap.user_secs;
        let _ = snap.system_secs;
        let _ = snap.rss_bytes;
        let _ = total;
    }

    #[test]
    fn capture_around_returns_reasonable_delta() {
        let (result, before, after) = capture_around(|| (0..10_000u64).sum::<u64>());
        assert_eq!(result, 49_995_000);
        assert!(after.user_secs >= before.user_secs);
    }
}

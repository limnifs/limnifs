//! Regression for the `metadata_externalize_threshold` override's RAISE
//! direction (issue #187 follow-up): the inline-vs-sidecar decision was
//! silently clamped to the default reader ceiling (1 MiB, spec §5.3)
//! regardless of config, so a caller could never inline a blob past it —
//! the documented per-image override was a lie for self-contained image
//! contracts (e.g. tebako's single-file images, which cannot carry a
//! metadata sidecar). The clamp is gone: the configured threshold is the
//! decision, in both directions.

use limnifs_write::{write_directory_with_config, WriteConfig};

/// Deterministic xorshift so entry names carry real entropy (constant
/// names would compress to nothing and never overshoot the ceiling).
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Grow `dir` by `batch` entries under 96 rotating subdirs.
fn add_entries(dir: &std::path::Path, rng: &mut XorShift, from: u32, batch: u32) -> u32 {
    for i in from..from + batch {
        let sub = dir.join(format!("d{:04x}", rng.next() % 96));
        std::fs::create_dir_all(&sub).expect("mkdir");
        std::fs::write(
            sub.join(format!(
                "file-{i:05}-{:016x}{:016x}.bin",
                rng.next(),
                rng.next()
            )),
            i.to_le_bytes(),
        )
        .expect("write");
    }
    from + batch
}

#[test]
fn raised_threshold_inlines_past_the_default_reader_ceiling() {
    let dir = std::env::temp_dir().join(format!("limnifs-inline-raise-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");

    // Self-calibrating fixture: batches until the DEFAULT config
    // externalizes — that sidecar IS the proof the blob overshoots the
    // stock threshold (1 MiB − 24 KiB). Hard cap names the remedy.
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    let mut total = 0;
    let default_sidecar = loop {
        total = add_entries(&dir, &mut rng, total, 6_000);
        let artifact =
            write_directory_with_config(&dir, &WriteConfig::default_v0_1()).expect("default write");
        if let Some(sidecar) = artifact.metadata_sidecar {
            break sidecar;
        }
        assert!(
            total < 48_000,
            "{total} entries still inline under the default threshold — \
             raise the fixture's per-entry entropy"
        );
    };
    assert!(
        default_sidecar.bytes.len() > 1024 * 1024 - 24 * 1024,
        "the fixture must overshoot the stock threshold: {} bytes",
        default_sidecar.bytes.len()
    );

    // The raise: the same tree with the threshold above the blob inlines
    // — no sidecar, and the blob lands inside the manifest bytes.
    let mut config = WriteConfig::default_v0_1();
    config.defaults.metadata_externalize_threshold = 64 * 1024 * 1024;
    let raised = write_directory_with_config(&dir, &config).expect("raised write");
    assert!(
        raised.metadata_sidecar.is_none(),
        "a raised threshold inlines the blob (was clamped to the 1 MiB \
         reader default regardless of config)"
    );

    // The lower direction is untouched: a tiny threshold externalizes
    // even this tree.
    let mut config = WriteConfig::default_v0_1();
    config.defaults.metadata_externalize_threshold = 1024;
    let lowered = write_directory_with_config(&dir, &config).expect("lowered write");
    assert!(
        lowered.metadata_sidecar.is_some(),
        "a lowered threshold still externalizes"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

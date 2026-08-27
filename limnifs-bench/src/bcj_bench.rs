//! BCJ real-workload benchmark (TODO.remaining item 1).
//!
//! Packs a directory of REAL ELF x86-64 executables twice:
//!
//! - **plain** — empty categorizers: every file takes the FastCDC
//!   chunk path with the plain codec tournament (LZ4/ZSTD/...).
//! - **bcj** — categorizers enabled: the built-in `Executable`
//!   categorizer routes ELF x86-64 files to whole-file BCJ-x86+LZ4
//!   (tournament keeps whichever is smaller, so BCJ can never lose).
//!
//! Reports the ratio of slab bytes and the per-file wins. The TODO's
//! ≥20% target is evaluated against real binaries (Linux CI runs
//! this against `/usr/bin`; locally, pass any dir of ELF files).

use std::path::{Path, PathBuf};

use limnifs_write::config::CategorizerConfig;
use limnifs_write::{write_directory_with_config, WriteConfig};

/// Collect up to `budget_bytes` of ELF64 x86-64 files from `dir`
/// into a scratch tree (hard-linked where possible, copied as
/// fallback), so the packer sees only real executables.
fn stage_elf_tree(dir: &Path, scratch: &Path, budget_bytes: u64) -> usize {
    let mut staged = 0usize;
    let mut budget = budget_bytes;
    let entries = match std::fs::read_dir(dir) {
        Ok(it) => it.flatten().collect::<Vec<_>>(),
        Err(_) => return 0,
    };
    // Deterministic order.
    let mut paths: Vec<PathBuf> = entries.into_iter().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if budget == 0 {
            break;
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let len = meta.len();
        if len < 4096 || len > 64 * 1024 * 1024 {
            continue; // tiny files prove nothing; huge ones blow CI budget
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        // ELF64 little-endian x86-64: \x7fELF, class=2, data=1, machine=0x3E.
        if bytes.len() < 20
            || &bytes[..4] != b"\x7fELF"
            || bytes[4] != 2
            || bytes[5] != 1
            || u16::from_le_bytes([bytes[18], bytes[19]]) != 0x3E
        {
            continue;
        }
        let name = path.file_name().unwrap_or_default();
        let dest = scratch.join(name);
        if std::fs::hard_link(&path, &dest).is_err() {
            let _ = std::fs::copy(&path, &dest);
        }
        budget = budget.saturating_sub(len);
        staged += 1;
    }
    staged
}

fn slab_bytes(artifact: &limnifs_write::WriteArtifact) -> u64 {
    artifact.slabs.iter().map(|s| s.bytes.len() as u64).sum()
}

/// Run the A/B over `dir`'s ELF files. Returns
/// `Some((plain_bytes, bcj_bytes, file_count))` when at least one
/// ELF was staged; `None` when the directory yields nothing (non-
/// Linux hosts without ELF trees).
///
/// # Panics
/// Panics if a pack fails — a benchmark that cannot run must fail
/// loudly.
#[must_use]
pub fn run(dir: &Path, scratch_root: &Path) -> Option<(u64, u64, usize)> {
    let scratch = scratch_root.join("bcj");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("mkdir scratch");
    let staged = stage_elf_tree(dir, &scratch, 256 * 1024 * 1024);
    if staged == 0 {
        eprintln!(
            "bcj-bench: no ELF64 x86-64 files found in {}",
            dir.display()
        );
        return None;
    }

    // plain: empty categorizers → chunk path, plain tournament.
    let mut plain_cfg = WriteConfig::default_v0_1();
    plain_cfg.defaults.text_codec = "lz4".into();
    plain_cfg.defaults.binary_codec = "lz4".into();
    plain_cfg.dictionaries.enabled = false;
    let plain = write_directory_with_config(&scratch, &plain_cfg).expect("plain pack");

    // bcj: any non-empty categorizers gate enables the default
    // registry's Executable categorizer (ELF x86-64 → BCJ-x86+LZ4).
    let mut bcj_cfg = WriteConfig::default_v0_1();
    bcj_cfg.defaults.text_codec = "lz4".into();
    bcj_cfg.defaults.binary_codec = "lz4".into();
    bcj_cfg.dictionaries.enabled = false;
    bcj_cfg.categorizers.push(CategorizerConfig {
        name: "bcj-bench-gate".into(),
        extensions: vec![],
        magic_bytes: vec![],
        codec: "lz4".into(),
        max_size: None,
        enabled: true,
    });
    let bcj = write_directory_with_config(&scratch, &bcj_cfg).expect("bcj pack");

    let plain_bytes = slab_bytes(&plain);
    let bcj_bytes = slab_bytes(&bcj);
    let _ = std::fs::remove_dir_all(&scratch);
    Some((plain_bytes, bcj_bytes, staged))
}

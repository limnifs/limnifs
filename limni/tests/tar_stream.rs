//! `limn --from-tar` and `limni tar` end-to-end (TODO.features/05).
//!
//! Library-level round trip: build a tar with the `tar` crate,
//! pack it with [`limnifs_write::stream::StreamWriter`] exactly as
//! the CLI does, then walk the resulting artifact's metadata tree
//! and compare every entry back to the tar's contents. Nothing is
//! materialised on disk beyond the artifact bytes.

#![cfg(feature = "tar")]

use std::collections::BTreeMap;
use std::io::Read;

use limnifs_write::stream::StreamWriter;
use limnifs_write::WriteConfig;

/// The CLI's `limn --from-tar` loop, lifted verbatim enough to be
/// the contract under test.
fn pack_tar<R: Read>(tar_bytes: &mut R) -> limnifs_write::WriteArtifact {
    // The CLI mmaps the archive; the test reads it into memory —
    // both hand the writer entry ranges over a byte buffer.
    let mut tar_bytes_owned = Vec::new();
    tar_bytes
        .read_to_end(&mut tar_bytes_owned)
        .expect("read tar");
    let tar_bytes = tar_bytes_owned.as_slice();
    let mut archive = tar::Archive::new(tar_bytes);
    let config = WriteConfig::default_v0_1();
    let mut writer = StreamWriter::new(&config).expect("default config");
    for entry in archive.entries().expect("entries") {
        let mut entry = entry.expect("entry");
        let mtime_ns = entry.header().mtime().unwrap_or(0) * 1_000_000_000;
        let name = entry
            .path()
            .expect("path")
            .to_str()
            .expect("utf-8")
            .trim_end_matches('/')
            .to_owned();
        match entry.header().entry_type() {
            tar::EntryType::Directory => writer.add_dir(&name, mtime_ns).expect("dir"),
            tar::EntryType::Symlink => {
                let target = entry.link_name().expect("link").expect("target");
                writer
                    .add_symlink(&name, &target.to_string_lossy(), mtime_ns)
                    .expect("symlink");
            }
            tar::EntryType::Regular => {
                // Mirror the CLI: random-access entries stage for the
                // parallel flush; entries whose size is overridden by
                // a PAX extension fall back to the streaming read.
                let raw_size = entry.header().size().unwrap_or(u64::MAX);
                let declared = entry.size();
                if raw_size == declared {
                    let start = usize::try_from(entry.raw_file_position()).expect("offset");
                    let end = start + usize::try_from(declared).expect("size");
                    let data = tar_bytes.get(start..end).expect("entry range");
                    writer
                        .stage_file(&name, mtime_ns, data)
                        .expect("staged file");
                } else {
                    writer
                        .add_file(&name, mtime_ns, &mut entry)
                        .expect("streamed file");
                }
            }
            other => panic!("unsupported entry type {other:?}"),
        }
    }
    writer.finish().expect("finish")
}

/// One entry of the expected tree: content or symlink target.
#[derive(Debug, PartialEq, Eq)]
enum Expected {
    File(Vec<u8>),
    Dir,
    Symlink(String),
}

fn build_tar(entries: &[(&str, Expected)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, expected) in entries {
        let mut header = tar::Header::new_gnu();
        match expected {
            Expected::File(data) => {
                header.set_entry_type(tar::EntryType::Regular);
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                builder
                    .append_data(&mut header, name, data.as_slice())
                    .expect("append file");
            }
            Expected::Dir => {
                header.set_entry_type(tar::EntryType::Directory);
                header.set_size(0);
                header.set_mode(0o755);
                let dir = format!("{name}/");
                builder
                    .append_data(&mut header, dir, std::io::empty())
                    .expect("append dir");
            }
            Expected::Symlink(target) => {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                header.set_mode(0o777);
                header.set_link_name(target).expect("link name");
                builder
                    .append_data(&mut header, name, std::io::empty())
                    .expect("append symlink");
            }
        }
    }
    builder.into_inner().expect("tar bytes")
}

/// Walk sink that gathers every entry's expected form.
struct CollectSink<'s> {
    out: BTreeMap<String, Expected>,
    store: &'s limnifs_core::slab_store::SlabStore,
}

impl limnifs_core::live_tree::LiveTreeSink for CollectSink<'_> {
    fn on_directory(&mut self, path: &std::path::Path) -> Result<(), limnifs_core::CoreError> {
        if !path.as_os_str().is_empty() {
            self.out
                .insert(path.to_string_lossy().into_owned(), Expected::Dir);
        }
        Ok(())
    }
    fn on_regular_file(
        &mut self,
        path: &std::path::Path,
        inode: &limnifs_core::Inode,
    ) -> Result<(), limnifs_core::CoreError> {
        let data =
            limnifs_core::live_tree::file_plaintext(inode, Some(self.store)).expect("plaintext");
        self.out
            .insert(path.to_string_lossy().into_owned(), Expected::File(data));
        Ok(())
    }
    fn on_symlink(
        &mut self,
        path: &std::path::Path,
        target: &str,
    ) -> Result<(), limnifs_core::CoreError> {
        self.out.insert(
            path.to_string_lossy().into_owned(),
            Expected::Symlink(target.to_owned()),
        );
        Ok(())
    }
}

/// Walk the artifact's metadata tree via the CLI's own read path
/// (`limnifs_core`), collecting file bytes and symlink targets.
fn collect_tree(artifact: &limnifs_write::WriteArtifact) -> BTreeMap<String, Expected> {
    use limnifs_core::{
        parse_feature_flags_section, parse_manifest_header, parse_metadata_reference,
        ManifestCursor,
    };

    let mut cursor = ManifestCursor::new(&artifact.bytes);
    parse_manifest_header(&mut cursor).expect("header");
    parse_feature_flags_section(&mut cursor).expect("flags");
    let meta_ref = parse_metadata_reference(&mut cursor).expect("metadata reference");
    let inline = meta_ref
        .inline_metadata
        .as_ref()
        .expect("test trees are small; metadata stays inline");
    let mut blob_cursor = ManifestCursor::new(inline);
    let blob = limnifs_core::parse_metadata_blob(&mut blob_cursor).expect("blob");

    // Slab-backed files resolve through an in-memory store built
    // from the artifact's slab bytes.
    let slab_bytes: Vec<Vec<u8>> = artifact.slabs.iter().map(|s| s.bytes.clone()).collect();
    let store = limnifs_core::slab_store::SlabStore::from_bytes(slab_bytes).expect("slabs");
    let mut sink = CollectSink {
        out: BTreeMap::new(),
        store: &store,
    };
    let root = blob
        .root_inode_number()
        .expect("stream images have a root dir");
    limnifs_core::live_tree::walk_live_tree(&blob, root, &mut sink).expect("walk");
    sink.out
}

fn expected_map(entries: &[(&str, Expected)]) -> BTreeMap<String, Expected> {
    let mut out: BTreeMap<String, Expected> = entries
        .iter()
        .map(|(name, e)| ((*name).to_owned(), clone_expected(e)))
        .collect();
    // Implicit parents materialise as directories in the image even
    // when the tar carries no entry for them.
    for (name, _) in entries {
        let components: Vec<&str> = name.split('/').collect();
        for depth in 1..components.len() {
            out.entry(components[..depth].join("/"))
                .or_insert(Expected::Dir);
        }
    }
    out
}

fn clone_expected(e: &Expected) -> Expected {
    match e {
        Expected::File(d) => Expected::File(d.clone()),
        Expected::Dir => Expected::Dir,
        Expected::Symlink(t) => Expected::Symlink(t.clone()),
    }
}

#[test]
fn tar_in_tree_out_round_trip() {
    let entries: Vec<(&str, Expected)> = vec![
        (
            "README.md",
            Expected::File(b"tar streaming round trip\n".to_vec()),
        ),
        ("docs", Expected::Dir),
        ("docs/guide.md", Expected::File(b"docs content\n".to_vec())),
        ("docs/latest", Expected::Symlink("guide.md".to_owned())),
        ("src/sub", Expected::Dir),
        (
            "src/sub/data.txt",
            Expected::File(vec![0x41; 8192]), // stays inline
        ),
        (
            "src/big.bin",
            Expected::File(pseudo_random(11, 600 * 1024)), // slabs
        ),
    ];
    let tar_bytes = build_tar(&entries);
    let artifact = pack_tar(&mut tar_bytes.as_slice());

    let actual = collect_tree(&artifact);
    let expected = expected_map(&entries);
    assert_eq!(actual, expected);
}

#[test]
fn same_tar_packs_identically() {
    let entries: Vec<(&str, Expected)> = vec![
        ("a.txt", Expected::File(b"same bytes\n".to_vec())),
        ("dir", Expected::Dir),
        ("dir/b.bin", Expected::File(pseudo_random(3, 700 * 1024))),
    ];
    let tar_bytes = build_tar(&entries);
    let a = pack_tar(&mut tar_bytes.as_slice()).bytes;
    let b = pack_tar(&mut tar_bytes.as_slice()).bytes;
    assert_eq!(a, b);
}

fn pseudo_random(seed: u64, count: usize) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        out.push(u8::try_from(state >> 56).expect("fits u8"));
    }
    out
}

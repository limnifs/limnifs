//! `LimniFS` core reader: manifest header + section parsers, plus the
//! drop-store primitives (slab header, drop record, slab index).
//!
//! Source of truth: `limnifs/spec` §3 (drop store), §5 (manifest
//! sections), and the bit-level files under `bit-level/`.
//!
//! Architecture: every parser takes a [`cursor::ManifestCursor`] and
//! returns the parsed value plus the cursor advanced past the section.
//! Bounds checks live in one place (the cursor); each parser focuses
//! on the structural invariants of its own section. Adding a new
//! section is a new module + a new parser function — no edits to
//! existing parsers ([OCP](https://en.wikipedia.org/wiki/Open%E2%80%93closed_principle)).
//!
//! ## Module map
//!
//! | Module | Owns |
//! |---|---|
//! | [`cursor`] | [`ManifestCursor`] — bounded reader over `&[u8]` |
//! | [`error`] | [`CoreError`] |
//! | [`header`] | [`ManifestHeader`] + [`parse_manifest_header`] |
//! | [`feature_flags`] | [`FeatureFlag`], [`FeatureFlags`] + [`parse_feature_flags_section`] |
//! | [`metadata_reference`] | [`MetadataReference`] + [`parse_metadata_reference`] |
//! | [`metadata`] | [`MetadataBlob`] + [`parse_metadata_blob`] |
//! | [`inode`] | [`Inode`], [`ContentHandle`] + [`parse_inode`] |
//! | [`directory_node`] | [`DirectoryNode`], [`DirEntry`] + [`parse_directory_node`] |
//! | [`slab_index`] | [`SlabIndex`], [`SlabIndexEntry`] + [`parse_slab_index`] |
//! | [`history`] | [`HistoryEntry`], [`History`] + [`parse_history`] |
//! | [`merkle`] | [`SectionHashes`], [`compute_merkle_root`] |
//! | [`slab`] | [`SlabHeader`] + [`parse_slab_header`] |
//! | [`slab_reader`] | [`SlabView`] + [`parse_slab`] — locate and read drop plaintexts |
//! | [`drop_record`] | [`DropRecord`] + [`parse_drop_record`] |
//! | [`locator`] | [`LocatorEntry`] + [`parse_locator_entry`] |

#![deny(unsafe_code)]
#![warn(clippy::pedantic)]

pub mod aead;
pub mod aead_ops;
pub mod categorization_policy;
pub mod chunking_config;
pub mod codec;
pub mod compression_tournament_config;
pub mod crypto;
pub mod cursor;
pub mod delta_linkage;
pub mod dictionary_section;
pub mod directory_node;
pub mod dms_policy;
pub mod dms_scheme;
pub mod drop_record;
pub mod ec_params;
pub mod ec_repair;
pub mod ec_scheme;
pub mod encryption_descriptor;
pub mod epoch;
pub mod error;
pub mod feature_flags;
pub mod fetch;
pub mod gf256;
pub mod header;
pub mod history;
#[cfg(feature = "http")]
pub mod http_locator;
pub mod inode;
#[cfg(feature = "http")]
pub mod ipfs_locator;
#[cfg(feature = "key-wrap")]
pub mod key_wrap;
pub mod locator;
pub mod merkle;
pub mod metadata;
pub mod metadata_reference;
pub mod reed_solomon;
#[cfg(feature = "http")]
pub mod s3_locator;
pub mod shamir;
#[cfg(feature = "signing")]
pub mod signing;
pub mod slab;
pub mod slab_index;
pub mod slab_reader;
pub mod slab_store;

pub use cursor::ManifestCursor;
pub use directory_node::{parse_directory_node, DirEntry, DirectoryNode, DIRECTORY_NODE_VERSION};
pub use dms_policy::{
    parse_dms_policy, parse_dms_policy_with_ceilings, DmsPolicy, ShareRecord,
    DEFAULT_HINT_MAX_BYTES, DEFAULT_SHARE_DATA_MAX_BYTES, DMS_POLICY_SECTION_VERSION,
    DMS_SCHEME_EXTENDED, DMS_SCHEME_SHAMIR, MAX_TOTAL_SHARES,
};
pub use drop_record::{
    parse_drop_record, parse_drop_record_with_ceiling, DropRecord, DROP_RECORD_LEN,
};
pub use ec_params::{
    parse_ec_params, EcOverride, EcParams, DEFAULT_EC_POLYNOMIAL, EC_PARAMS_SECTION_VERSION,
    MAX_SHARDS,
};
pub use error::CoreError;
pub use feature_flags::{
    parse_feature_flags_section, FeatureFlag, FeatureFlags, FEATURE_FLAGS_SECTION_VERSION,
};
pub use header::{parse_manifest_header, ManifestHeader};
pub use history::{
    parse_history, parse_history_with_ceiling, History, HistoryEntry, HistoryOp,
    DEFAULT_HISTORY_PARAMS_MAX_BYTES, HISTORY_SECTION_VERSION, OP_EXTENDED,
};
pub use inode::{
    parse_inode, parse_inode_with_ceiling, ContentHandle, Inode, SliceRef, XAttr,
    DEFAULT_INLINE_DATA_MAX_BYTES, INODE_FIXED_PREFIX_LEN, INODE_FLAG_ATIME,
    INODE_FLAG_INLINE_DATA, INODE_FLAG_RESERVED_MASK, INODE_FLAG_SHARED_INLINE, S_IFBLK, S_IFCHR,
    S_IFDIR, S_IFIFO, S_IFLNK, S_IFMT, S_IFREG, S_IFSOCK,
};
pub use locator::{
    parse_locator_entries, parse_locator_entries_with_ceiling, parse_locator_entry,
    parse_locator_entry_with_ceiling, LocatorEntry, DEFAULT_LOCATOR_MAX_URI_BYTES,
    LOCATOR_LENGTH_PREFIX_LEN, MIN_LOCATOR_URI_BYTES,
};
pub use merkle::{
    compute_merkle_root, hash_empty_section, hash_section, section_hashes_minimal, SectionHashes,
    MERKLE_DOMAIN_SEPARATOR,
};
pub use metadata::{
    dir_node_hash, parse_metadata_blob, parse_metadata_blob_with_ceiling, MetadataBlob,
};
pub use metadata_reference::{
    parse_metadata_reference, parse_metadata_reference_with_ceilings, MetadataReference,
    DEFAULT_INLINE_METADATA_MAX_BYTES, METADATA_REFERENCE_SECTION_VERSION,
    METADATA_REFERENCE_SECTION_VERSION_2,
};
pub use slab::{
    parse_slab_header, parse_slab_header_with_ceiling, SlabHeader, CRYPTO_HINT_EXTENDED,
    DEFAULT_SLAB_MAX_BYTES, EC_DESCRIPTOR_EXTENDED, SLAB_FORMAT_VERSION, SLAB_HEADER_LEN,
};
pub use slab_index::{
    parse_slab_index, parse_slab_index_with_ceiling, SlabIndex, SlabIndexEntry,
    SLAB_INDEX_SECTION_VERSION,
};
pub use slab_reader::{parse_slab, SlabView};

#[cfg(feature = "http")]
pub use http_locator::HttpLocator;
#[cfg(feature = "http")]
pub use s3_locator::{S3Locator, DEFAULT_S3_ENDPOINT};

pub use limnifs_format::{ManifestRoot, MANIFEST_HEADER_LEN};

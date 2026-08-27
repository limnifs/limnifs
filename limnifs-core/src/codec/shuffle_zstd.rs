//! Shuffle+ZSTD composite codec (id 0x0E): byte-shuffle then ZSTD.
//!
//! Same transposition idea as `shuffle+lz4`, with ZSTD's stronger
//! entropy stage for numeric arrays that LZ4's match finder handles
//! poorly. Wire format is the shared composite pipeline:
//! `[filtered_len: u32 LE][zstd(filtered_bytes)]`, with the shuffle's
//! self-describing `[tag][item_size]` prefix inside the filtered
//! stream.

use crate::codec::composite;
use crate::codec::CODEC_SHUFFLE_ZSTD;

/// Shuffle+ZSTD composite as an instance of
/// [`FilterCodecComposite`](crate::codec::composite::FilterCodecComposite).
pub type ShuffleZstdCodec = composite::FilterCodecComposite<omnizip_filters::shuffle::ByteShuffle>;

/// ByteShuffle (default item_size) + ZSTD.
#[must_use]
pub fn shuffle_zstd() -> ShuffleZstdCodec {
    ShuffleZstdCodec::new(
        omnizip_filters::shuffle::ByteShuffle::new(4),
        crate::codec::CODEC_ZSTD,
        CODEC_SHUFFLE_ZSTD,
        "shuffle+zstd",
        512,
    )
}

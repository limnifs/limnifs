//! Bitshuffle+LZ4 composite codec (id 0x0F): bit-transpose then LZ4.
//!
//! BitShuffle moves the high-correlation bits of numeric data into
//! contiguous byte lanes; LZ4 then compresses the long same-bit runs.
//! Suited to low-entropy integer/bitmap-like arrays. Wire format is
//! the shared composite pipeline with the filter's self-describing
//! prefix inside the filtered stream.

use crate::codec::composite;
use crate::codec::CODEC_BITSHUFFLE_LZ4;

/// Bitshuffle+LZ4 composite as an instance of
/// [`FilterCodecComposite`](crate::codec::composite::FilterCodecComposite).
pub type BitshuffleLz4Codec = composite::FilterCodecComposite<omnizip_filters::shuffle::BitShuffle>;

/// BitShuffle (default item_size) + LZ4.
#[must_use]
pub fn bitshuffle_lz4() -> BitshuffleLz4Codec {
    BitshuffleLz4Codec::new(
        omnizip_filters::shuffle::BitShuffle::new(8),
        crate::codec::CODEC_LZ4,
        CODEC_BITSHUFFLE_LZ4,
        "bitshuffle+lz4",
        512,
    )
}

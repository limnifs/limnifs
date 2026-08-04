#!/usr/bin/env bash
# FLAC LPC verification corpus fetcher.
#
# Downloads public-domain audio from four sources, organises by
# genre, and produces a corpus suitable for differential testing
# of omnizip-flac vs libFLAC.
#
# Usage:
#   ./fetch_flac_corpus.sh <dest_dir>
#
# The corpus is ~1.5 GB total. Downloads are cached: re-running
# skips already-fetched files.
#
# Sources (all public domain or CC-licensed):
# - MusOpen (classical): https://musopen.org/music
# - LibriSpeech (speech): https://openslr.org/12
# - Free Music Archive (ambient): https://freemusicarchive.org
# - Internet Archive 78rpm (historic): https://archive.org/details/78rpm
#
# See `TODO.impl/04-writer-pipeline/04-omnizip-new-algos.md` and
# `docs/omnizip-proposals/flac-lpc-finish.md`.

set -euo pipefail

DEST="${1:?Usage: $0 <dest_dir>}"
mkdir -p "$DEST"/{classical,speech,ambient,historic,synthetic}

echo "=== FLAC LPC verification corpus ==="
echo "Destination: $DEST"
echo ""

# --- Classical (MusOpen public-domain recordings) ---
echo "--- Classical ---"
# A small selection of public-domain recordings from MusOpen.
# These are WAV files we can compress with both FLAC implementations.
CLASSICAL_URLS=(
    "https://musopen.org/music/2476-piano-sonata-no-14-moonlight-op-27-no-2/download/"
)
for url in "${CLASSICAL_URLS[@]}"; do
    fname=$(basename "$url" | sed 's/[^a-zA-Z0-9._-]/_/g')
    target="$DEST/classical/${fname}.wav"
    if [[ ! -f "$target" ]]; then
        echo "  Downloading: $url"
        # Note: MusOpen URLs may need manual download.
        # For CI, we skip files that fail (network issues, redirects).
        curl -fsSL "$url" -o "$target" 2>/dev/null || {
            echo "  SKIP (download failed): $url"
            rm -f "$target"
        }
    else
        echo "  CACHED: $target"
    fi
done

# --- Speech (LibriSpeech dev-clean subset) ---
echo "--- Speech ---"
SPEECH_URL="https://openslr.org/resources/12/dev-clean.tar.gz"
SPEECH_TARGET="$DEST/speech/dev-clean.tar.gz"
if [[ ! -f "$SPEECH_TARGET" ]]; then
    echo "  Downloading LibriSpeech dev-clean (337 MB)..."
    curl -fsSL "$SPEECH_URL" -o "$SPEECH_TARGET" || {
        echo "  SKIP (download failed)"
        rm -f "$SPEECH_TARGET"
    }
else
    echo "  CACHED: $SPEECH_TARGET"
fi

# --- Ambient (Free Music Archive CC-licensed) ---
echo "--- Ambient ---"
# FMA small subset: https://github.com/mdeff/fma
# Use a few sample tracks.
AMBIENT_URL="https://freemusicarchive.org/track/Kevin_MacLeod/Call_to_Adventure/download/"
AMBIENT_TARGET="$DEST/ambient/call_to_adventure.mp3"
if [[ ! -f "$AMBIENT_TARGET" ]]; then
    echo "  Downloading ambient sample..."
    curl -fsSL "$AMBIENT_URL" -o "$AMBIENT_TARGET" 2>/dev/null || {
        echo "  SKIP (download failed)"
        rm -f "$AMBIENT_TARGET"
    }
else
    echo "  CACHED: $AMBIENT_TARGET"
fi

# --- Synthetic (generated locally, no download) ---
echo "--- Synthetic ---"
echo "  Generating synthetic WAV fixtures..."
# Use Python to generate WAV files with known characteristics.
python3 - <<'PYEOF'
import struct, wave, math, os

dest = os.environ.get("DEST", "/tmp/flac_corpus")
synth_dir = os.path.join(dest, "synthetic")

# 1. Swept sine (smooth → high FLAC ratio).
with wave.open(os.path.join(synth_dir, "swept_sine.wav"), "w") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(44100)
    for i in range(44100 * 5):  # 5 seconds
        freq = 100 + i * 0.01
        sample = int(32767 * math.sin(2 * math.pi * freq * i / 44100))
        w.writeframes(struct.pack("<h", sample))

# 2. White noise (random → low FLAC ratio, should fall back to STORE).
with wave.open(os.path.join(synth_dir, "white_noise.wav"), "w") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(44100)
    import random
    random.seed(42)
    for _ in range(44100 * 5):
        sample = random.randint(-32768, 32767)
        w.writeframes(struct.pack("<h", sample))

# 3. Pink noise (between sine and white noise).
with wave.open(os.path.join(synth_dir, "pink_noise.wav"), "w") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(44100)
    b0, b1, b2, b3, b4, b5, b6 = 0, 0, 0, 0, 0, 0, 0
    for _ in range(44100 * 5):
        white = random.uniform(-1, 1)
        b0 = 0.99886 * b0 + white * 0.0555179
        b1 = 0.99332 * b1 + white * 0.0750759
        b2 = 0.96900 * b2 + white * 0.1538520
        b3 = 0.86650 * b3 + white * 0.3104856
        b4 = 0.55000 * b4 + white * 0.5329522
        b5 = -0.7616 * b5 - white * 0.0168980
        pink = (b0 + b1 + b2 + b3 + b4 + b5 + b6 + white * 0.5362) * 0.11
        b6 = white * 0.115926
        sample = int(32767 * pink)
        w.writeframes(struct.pack("<h", sample))

print(f"  Generated 3 synthetic WAV fixtures in {synth_dir}")
PYEOF

echo ""
echo "=== Corpus fetch complete ==="
echo "Files in $DEST:"
find "$DEST" -type f | head -20
echo "Total size: $(du -sh "$DEST" 2>/dev/null | cut -f1)"
echo ""
echo "Next step: run the differential harness:"
echo "  cargo test --test flac_corpus_differential -- --ignored"

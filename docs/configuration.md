# Configuration reference

LimniFS's tuning surface lives in `WriteConfig`. Two ways to reach it:

- **CLI** — `limni limn --profile <name>` selects one of nine built-in
  profiles, with `--text-codec` and `--chunk-size` overrides.
- **Library** — `limnifs_write::write_directory_with_config` takes a
  full `WriteConfig`, deserializable from TOML. Every field below.

The TOML example shows every section with its defaults; omit what you
don't care about.

```toml
[defaults]
text_codec = "brotli"          # codec for text-classified chunks
binary_codec = "lz4"           # codec for binary-classified chunks
metadata_codec = "brotli"      # metadata blob codec
metadata_quality = 5           # brotli quality (steps to 2 on large blobs)
metadata_externalize_threshold = 1024000  # 1 MiB − 24 KiB
shared_inline = true           # dedupe identical inline payloads
max_drop_size = 4194304        # whole-file drop cap (0 = unlimited)
seekable_drops = true          # emit LMSK seekable containers
inline_threshold = 4096        # payloads this small inline into metadata

[tournament]
codecs = ["store", "lz4", "zstd", "brotli"]  # tried fast → slow
min_size_threshold = 256       # below this, use the class codec directly
skip_for_binary = true         # binary chunks skip the tournament
short_circuit_threshold = 250  # per-mille; accept once a codec hits 25%

[chunking]
name = "fastcdc"
avg_chunk_size = 262144
min_chunk_size = 65536
max_chunk_size = 1048576

[dictionaries]
enabled = true
min_class_size = 100           # drops needed before training
max_dict_size = 65536
trainer = "frequency"

# User categorizers run BEFORE the built-in registry (issue #196).
[[categorizers]]
name = "my-wav-rule"
extensions = [".wav"]
magic_bytes = ["52494646"]     # hex, optional
codec = "flac"
max_size = 0                   # 0 = unlimited
enabled = true

[codec_tunables.zstd]
quality = 2                    # see the tier map below

[codec_tunables.brotli]
quality = 11
window = 22

[codec_tunables.lzma]
lc = 3
lp = 0
pb = 2
dict_size_mb = 64
use_optimal_parser = true

[codec_tunables.bzip2]
block_size_kb = 900

[codec_tunables.ppmd7]
order = 6
memory_budget_mb = 256

[encryption]
aead = "chacha20-poly1305"
key_wrap = "x25519-hkdf"
```

## Built-in profiles

| Profile | Optimizes | Notes |
|---|---|---|
| `balanced` (default) | all-round | The v0.1 defaults |
| `max-ratio` | smallest output | Brotli q11 + zstd L19 tournament, all categorizers, no seekable containers (a container's independent frames cost 1-3%) |
| `max-speed` | create throughput | Skip chunking, whole-file drops |
| `competitive` | benchmark posture | Tournament-first |
| `max-read` / `max-write` | read / write path | Read: seekable everywhere |
| `max-write-rw` / `max-read-rw` / `balanced-rw` | RW layer variants | Staging-store tuned |

## The zstd tier map

`codec_tunables.zstd.quality` maps to omnizip levels. Since omnizip
0.21.12, **every level ≥ L3 runs the optimal parser** — roughly 16×
slower at LimniFS chunk sizes for ~4% tighter output. The default
therefore sits at `quality = 2` (`Fastest`, L1/L2), the only fast
tier left:

| quality | level | parser band |
|---|---|---|
| 0–2 | Fastest (L1/L2) | fast — **default** |
| 3–5 | Fast (L3/L4) | optimal parser |
| 6–11 | Default (L6) | optimal parser |
| 12–21 | Better (L12) | optimal parser |
| 22+ | Best (L22) | optimal parser |

Raising the default into the parser band is a conscious speed/ratio
trade — measure with the `createperf` canary
(`cargo run --release -p limnifs-bench -- createperf`). The
`max-ratio` profile sets 19 deliberately. Each codec's quality knob
is independent (`zstd.quality` never reads brotli's scale); a pinned
test in `limnifs-core/src/codec/zstd.rs` guards the map.

## Verifying images

```bash
limni verify image.lim          # manifest structure + report
limni verify image.lim --deep   # + every drop decompressed and
                                #   BLAKE3-checked against its
                                #   content-addressed id (one
                                #   extraction pass)
```

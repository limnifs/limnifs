# 04 — ZSTD dictionary training

- **Status:** in_progress (codec layer done; writer integration pending)
- **Phase:** 2
- **Depends on:** 04-ppmd-quality-wiring, 01-spec (dictionary section)
- **Design refs:** §6, 2026-throughput-roadmap.md §3, Collet 2018 (FastCover)
- **Priority:** P1

## Goal

`WriteConfig::dictionaries.enabled` is a no-op in the writer. The
codec-layer plumbing already exists:

- `limnifs-core::codec::zstd_dict::train_dictionary` wraps
  `omnizip_zstd::train_dictionary` (FrequencyTrainer by default).
- `limnifs-core::codec::zstd_dict::compress_with_dict` /
  `decompress_with_dict` wrap omnizip's dict-aware codec.
- `limnifs-core::dictionary_section::{DictionarySection, Dictionary,
  encode_dictionary_section, parse_dictionary_section}` is the wire
  format.
- `DropRecord.dict_id` (u8, `NO_DICT = 0xFF` sentinel) is already in
  the v0.2 drop record.

What's missing is **writer integration**: collect samples per content
class, train one dictionary per class, and emit drops with `dict_id`
populated.

## Design

### Writer pipeline change

Current:
```
walk → chunk → compress (parallel) → write slab
```

New (when `dictionaries.enabled`):
```
walk → chunk → collect plaintext per class (sequential)
            → train dict per class (sequential, omnizip FrequencyTrainer)
            → compress-with-dict (parallel, omnizip compress_with_dict)
            → write slab + dictionary section
```

The dictionary is trained **before** parallel compress starts, so it
must be a sequential pre-pass. Backwards-compatible: if no dictionary
is trained, drops use `dict_id = NO_DICT` and the dictionary section
is omitted.

### Trainer choice

`omnizip_zstd::dict_trainer` exposes two trainers:

- `FrequencyTrainer` — top-K substrings by frequency × length.
  Deterministic, fast, captures obvious common substrings. Default.
- `FastCoverTrainer` — dmer-frequency scoring per FastCover
  (Facebook 2018). Better ratio on corpora with distributed
  redundancy (mixed JSON, source trees, log lines). Opt-in via
  `DictionaryConfig.trainer = "fastcover"`.

Adding a new trainer is one `impl DictTrainer` — no edits to the
compress/decompress paths (OCP).

### Dictionary id allocation

Per-image, per-codec. Ids `0x00..=0xFE` are valid; `0xFF` is
`NO_DICT`. The writer allocates ids in the order dictionaries are
trained: text-zstd → 0, binary-zstd → 1, etc. The dictionary section
records `(dict_id, codec_id, dict_bytes)` triples.

### Decompression

The slab reader already parses `DropRecord.dict_id`. The decoder
needs to:

1. Parse the dictionary section once per image (cache on `SlabStore`).
2. For each drop with `dict_id != NO_DICT`, look up the dictionary
   and pass it to the codec's dict-aware decompress.

## Notes

- The FrequencyTrainer returns the **raw dict bytes**, not a
  serialized `ZstdDictionary`. Wrap with
  `ZstdDictionary::from_raw(id, &bytes)` at compress time.
- Dictionary is most effective for many-small-file workloads
  (config files, maildirs, source trees). The benchmark harness
  has a `tiny-files` dataset that exercises this.
- Don't train a dict on a single sample — frequency scoring needs
  repetition across samples. Min `dictionaries.min_class_size` is
  100 drops by default.

## Acceptance

- [ ] `WriteContext` collects plaintext samples per `(class, codec)`
      during the walk.
- [ ] When `dictionaries.enabled = true` and a class has ≥
      `min_class_size` drops, train one dictionary per class.
- [ ] Emit a `dictionary_section` after the slab index when at least
      one dictionary was trained.
- [ ] Drop records for dict-compressed drops carry their `dict_id`.
- [ ] `cat` of a dict-compressed file produces the original bytes
      (reader resolves the dict from the section).
- [ ] `limnifs-bench` `tiny-files` dataset shows ≥ 20% ratio
      improvement vs no-dict baseline when `dictionaries.enabled =
      true`.

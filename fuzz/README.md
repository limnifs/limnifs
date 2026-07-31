# Fuzz targets

LimniFS ships 9 cargo-fuzz targets covering the manifest and slab
parsers. The fuzzer's job is to verify that no input — however
adversarial — causes a parser to panic or run away.

## Targets

| Target | Parser |
|---|---|
| `manifest_header` | `parse_manifest_header` |
| `feature_flags` | `parse_feature_flags_section` |
| `metadata_reference` | `parse_metadata_reference` |
| `slab_index` | `parse_slab_index` |
| `history` | `parse_history` |
| `metadata_blob` | `parse_metadata_blob` (with DoS guards) |
| `slab_header` | `parse_slab_header` |
| `drop_record` | `parse_drop_record` (uses fuzzed header) |
| `locator_entry` | `parse_locator_entry` |

## Run locally

```sh
cd fuzz
cargo +nightly fuzz run manifest_header -- -max_len=65536
```

Press Ctrl-C to stop. Crash artifacts land in `artifacts/<target>/`.

## Nightly CI

[`.github/workflows/nightly-fuzz.yml`](../.github/workflows/nightly-fuzz.yml)
runs every target for 10 minutes (configurable via workflow_dispatch
input). Crashes are uploaded as workflow artifacts.

## Adding a new target

1. Add `[[bin]]` to `fuzz/Cargo.toml`.
2. Create `fuzz/fuzz_targets/<name>.rs`.
3. Add the target name to the matrix in `nightly-fuzz.yml`.
4. (Optional) Add seed corpus from conformance vectors.

## Crash handling

Per the campaign rule "every crash becomes a permanent regression
vector before the fix merges": crashes found in CI are downloaded,
added as `#[test]` cases under `fuzz/seeds/`, and the fix must keep
those seeds passing.

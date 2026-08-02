# 14: Skip-tournament-for-binary

## Status: IMPLEMENTED

## Scope

Today the writer's tournament tries the configured codec and
falls back to STORE if it doesn't beat plain. For binary content,
this means trying LZ4 (fast) vs STORE — pointless, since LZ4
always beats STORE on structured binary.

Add a `skip_for_binary` flag to the tournament config: when set,
binary chunks always go to LZ4 (or whatever `default_binary_codec`
is set to), no tournament.

## Why

The tournament logic has measurable overhead (compress + measure
length + compare). For binary chunks, it's wasted because LZ4
is always going to win. Skipping the tournament for binary
saves CPU and time.

## Design

### Config flag

```rust
pub struct CompressionTournamentConfig {
    pub codecs: Vec<u8>,
    pub min_size_threshold: u32,
    /// Skip tournament for binary class — use default_binary_codec directly.
    pub skip_for_binary: bool,  // default: true
}
```

### Writer change

```rust
let class = classifier.classify(chunk);
let codec_id = if config.tournament.skip_for_binary && class == Class::Binary {
    config.default_binary_codec
} else {
    run_tournament(chunk, class, &config)
};
```

## Implementation

1. Add `skip_for_binary` to `CompressionTournamentConfig`
2. Update tournament logic in `limnifs-write/src/lib.rs`
3. Defaults to `true` (v0.1 behaviour already does this implicitly)
4. Specs: verify binary chunks go to LZ4 without tournament

## Related files

- `limnifs-write/src/config.rs`
- `limnifs-write/src/lib.rs`
- `limnifs-core/src/compression_tournament_config.rs`

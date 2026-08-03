# 04 — Per-codec tunables registry (OCP refactor)

- **Status:** pending
- **Phase:** 2
- **Depends on:** 04-ppmd-quality-wiring (current implementation)
- **Design refs:** 2026-throughput-roadmap.md §9
- **Priority:** P1

## Goal

Today's `CodecTunables` (in `limnifs-core::codec`) is a flat
struct with a field per codec family:

```rust
pub struct CodecTunables {
    pub quality: u8,
    pub ppmd_order: u8,
    pub ppmd7_budget: usize,
    pub ppmd8_budget: usize,
    pub bzip2_block_kb: u32,
    pub lzma_dict_mb: u32,
}
```

Adding a new codec with tunable parameters requires editing this
struct, which violates OCP. Each new field also bloats the struct
for codecs that don't read it.

Refactor to per-codec tunables, registered alongside the codec
itself.

## Design

### Trait

```rust
/// Per-codec tunable parameters. Each codec declares its own
/// `Tunables` associated type; the registry stores them as
/// `Box<dyn Any>` keyed by codec id.
pub trait Codec: Send + Sync {
    type Tunables: Clone + Send + Sync + 'static;

    fn compress_with_tunables(
        &self,
        plaintext: &[u8],
        tunables: &Self::Tunables,
    ) -> Result<Vec<u8>, CoreError>;
    // ...
}
```

Each codec defines its own tunables type:

```rust
// in ppmd.rs
#[derive(Clone, Debug, Default)]
pub struct Ppmd7Tunables {
    pub order: u8,
    pub budget: usize,
}

impl Codec for PpmdCodec {
    type Tunables = Ppmd7Tunables;
    fn compress_with_tunables(&self, plaintext: &[u8], t: &Ppmd7Tunables) -> Result<...> {
        let order = if t.order > 0 { t.order } else { self.order };
        // ...
    }
}
```

### Profile-side mapping

Profile config declares tunables per codec by name:

```toml
[codec_tunables.ppmd7]
order = 6
memory_budget_mb = 256

[codec_tunables.brotli]
quality = 11
window = 24
```

`WriteConfig::codec_tunables` becomes a `BTreeMap<String,
TomlValue>`; the dispatcher looks up `tunables[codec_name]` and
deserialises into the codec's `Tunables` type. Unknown codec
names are an error.

### Registry change

`CodecRegistry::compress_with_tunables` takes `&dyn Any` instead
of a fixed struct:

```rust
pub fn compress_with_tunables(
    &self,
    id: u8,
    plaintext: &[u8],
    tunables: &dyn Any,  // each codec downcasts to its Tunables
) -> Result<Vec<u8>, CoreError>;
```

### Migration

Backwards-compatible:
1. Keep `CodecTunables` (the flat struct) as a deprecated alias.
2. Build a `BTreeMap<String, Box<dyn Any>>` from the flat struct
   in `WriteConfig::to_core_tunables_map()`.
3. Once all callers migrate, remove the flat struct.

## Notes

- The current flat-struct approach is fine for 6 codec families.
  It becomes painful at 15+. Today's registry has 18 codecs; the
  pain is real but not blocking.
- `Box<dyn Any>` is heavier than the flat struct (heap allocation
  per call), but tunables lookup happens once per chunk, not per
  byte. Negligible.
- The trait change `type Tunables` is a breaking change for
  external `Codec` impls. LimniFS controls all impls today, so
  this is fine. If/when external impls are supported, gate behind
  a v2 trait.

## Acceptance

- [ ] `Codec::Tunables` associated type exists.
- [ ] All 18 registered codecs declare their `Tunables` type.
- [ ] `WriteConfig` codec_tunables map keyed by codec name.
- [ ] All existing `tunables_*` tests still pass.
- [ ] A new codec can be added with tunables WITHOUT editing
      `CodecTunables` (the proof of OCP).

## Why LimniFS cares

- Today adding a new codec's tunables requires editing 4 sites
  (struct, `Default`, `to_core_tunables`, the codec's
  `compress_with_tunables`). After this refactor, only 1 site
  (the codec's own file).
- Profiles become self-documenting: `[codec_tunables.zstd]` makes
  it clear which codec owns each knob.
- Sets up cleanly for user-defined codecs (future).

## Effort estimate

2 days:
- 1 day: trait change + migrate 18 impls.
- 1 day: profile TOML migration + tests.

## Related

- `04-ppmd-quality-wiring.md` — the current (flat-struct)
  implementation this refactors.
- 2026-throughput-roadmap.md §9 (Code Quality).

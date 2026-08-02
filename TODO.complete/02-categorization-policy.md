# 02: CategorizationPolicy manifest section

## Status: IMPLEMENTED

## Scope

Add a new manifest section `categorization_policy` that records the
file categorizer rules used at write time. The reader can use this
to route content for decompression or to verify the writer's policy.

## Why

Without a recorded policy, the reader cannot reconstruct how the
writer routed files. For reproducibility (a core LimniFS value),
the policy must be self-describing: any reader can open any image
without external config.

## Design

### Manifest section

```rust
pub const CATEGORIZATION_POLICY_SECTION_VERSION: u8 = 1;

#[derive(Debug, Clone)]
pub struct CategorizationPolicy {
    pub version: u8,
    pub rules: Vec<CategorizerRule>,
}

#[derive(Debug, Clone)]
pub struct CategorizerRule {
    pub name: String,
    pub extensions: Vec<String>,
    pub magic_bytes: Vec<u8>,
    pub codec: u8,
    pub max_size: Option<u32>,
    pub enabled: bool,
}
```

### Wire format

```
+--------------------+  1 byte: section_version = 1
| version            |
+--------------------+  4 bytes LE: rule_count
| rule_count         |
+--------------------+  per rule:
| rule[0]            |
| ...                |
+--------------------+

rule[i] layout:
+--------------------+  1 byte: name_len
| name_len           |
+--------------------+  name_len bytes: name (UTF-8)
| name               |
+--------------------+  4 bytes LE: ext_count
| ext_count          |
+--------------------+  per extension:
| ext[j]             |
+--------------------+  4 bytes LE: magic_len
| magic_len          |
+--------------------+  magic_len bytes: magic
| magic              |
+--------------------+  1 byte: codec_id
| codec_id           |
+--------------------+  1 byte: flags (bit 0 = enabled, bit 1 = has_max_size)
| flags              |
+--------------------+  (if has_max_size) 4 bytes LE: max_size
| max_size           |
+--------------------+
```

### API

```rust
// In limnifs-core::categorization_policy
pub fn parse_categorization_policy(cur: &mut ManifestCursor) -> Result<CategorizationPolicy, CoreError>;
pub fn encode_categorization_policy(policy: &CategorizationPolicy, out: &mut Vec<u8>);

// In limnifs-write::config
impl WriteConfig {
    pub fn to_categorization_policy(&self) -> CategorizationPolicy;
}
```

## Implementation

1. New module `limnifs-core/src/categorization_policy.rs`
2. Add `CategorizationPolicy` + `CategorizerRule` types
3. Add `parse_categorization_policy` + `encode_categorization_policy`
4. Wire into `ManifestRoot` (alongside other sections)
5. Add `WriteConfig::to_categorization_policy()` conversion
6. Write the policy in the manifest in `assemble()`
7. Specs: round-trip, edge cases (empty rules, max_size=0)

## Related files

- `limnifs-core/src/lib.rs` (module declarations)
- `limnifs-core/src/cursor.rs` (ManifestCursor)
- `limnifs-format/src/manifest.rs` (section list)
- `limnifs-write/src/lib.rs` (assemble)
- New: `limnifs-core/src/categorization_policy.rs`

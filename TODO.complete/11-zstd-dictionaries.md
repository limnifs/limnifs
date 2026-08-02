# 11: ZSTD dictionary compression

## Status: IMPLEMENTED

## Scope

Add a two-pass writer pipeline that:
1. First pass: chunk all files, compress with chosen codec
2. Second pass: collect chunks per class, train ZSTD dict per class
3. Re-compress qualifying chunks with `compress_with_dict`

Wire it into the writer via:
- Extended `DropRecord` with `dict_id` field
- New manifest section `dictionary_section` storing trained dicts
- `DropRecordExtension` for dict_id reference

## Why

omnizip 0.11.1 ships `zstd::train_dictionary()` and
`compress_with_dict()`. ZSTD dictionaries dramatically improve
compression on small chunks with similar content (JSON, CSV,
HTML, source code).

Expected: 3.6% on repetitive text (vs 4.2% for PPMd), even better
when many chunks share the same vocabulary.

## Design

### DropRecord extension

Current DropRecord (37 bytes):
```rust
pub struct DropRecord {
    pub drop_id: [u8; 32],         //  32
    pub codec_id: u8,              //  1
    pub compressed_offset: u32,    //  4
    pub compressed_len: u32,       //  4
    pub plaintext_len: u32,        //  4
    pub flags: u8,                 //  1
}                                  // Total: 46 bytes
```

Note: the actual current DropRecord is 42 bytes (DROP_RECORD_LEN const).
Let me keep that and add a dict_id byte only when the dictionary
extension is enabled — flag bit controls presence.

New: Use `flags` bit 0 (`DROP_FLAG_HAS_DICT = 0x01`) to indicate
the dict_id byte follows. Otherwise, DropRecord is unchanged.

```
if flags & DROP_FLAG_HAS_DICT:
    + 1 byte: dict_id (0xFF = none, 0..254 = dictionary index)
```

### Manifest section: dictionary_section

```rust
pub const DICTIONARY_SECTION_VERSION: u8 = 1;

#[derive(Debug, Clone)]
pub struct DictionarySection {
    pub version: u8,
    pub dicts: Vec<Dictionary>,
}

#[derive(Debug, Clone)]
pub struct Dictionary {
    pub codec_id: u8,        // 0x02 = ZSTD
    pub class_id: u8,        // 0 = text, 1 = code, 2 = binary, etc.
    pub data: Vec<u8>,
}
```

### Wire format

```
+--------------------+  1 byte: section_version = 1
| version            |
+--------------------+  1 byte: dict_count
| dict_count         |
+--------------------+  per dict:
| dict[i]            |
+--------------------+

dict[i] layout:
+--------------------+  1 byte: codec_id
| codec_id           |
+--------------------+  1 byte: class_id
| class_id           |
+--------------------+  4 bytes LE: dict_len
| dict_len           |
+--------------------+  dict_len bytes: trained dictionary
| dict_data          |
+--------------------+
```

### Writer two-pass pipeline

```rust
fn write_directory_with_config(root: &Path, config: &WriteConfig) -> Result<WriteArtifact, WriteError> {
    // Pass 1: chunk + compress with codec
    let mut chunks = chunk_all_files(root, &config)?;
    let mut drops = Vec::new();
    for chunk in &chunks {
        let drop = compress_chunk_v1(chunk, &config)?;
        drops.push(drop);
    }

    // Pass 2: train dicts per class
    if config.dictionaries.enabled {
        let dicts = train_dictionaries(&chunks, &config)?;
        // Re-compress qualifying chunks with dict
        for drop in drops.iter_mut() {
            if drop.codec_id == CODEC_ZSTD && drop.plaintext_len >= config.dictionaries.min_class_size {
                let dict = &dicts[drop.class_id];
                drop.compressed = zstd::compress_with_dict(&drop.plaintext, dict)?;
                drop.flags |= DROP_FLAG_HAS_DICT;
                drop.dict_id = drop.class_id;
            }
        }
        // Emit dictionary section
        return Ok(WriteArtifact { drops, dicts: Some(dicts), .. });
    }

    Ok(WriteArtifact { drops, dicts: None, .. })
}
```

## Implementation

1. Add `DROP_FLAG_HAS_DICT` constant
2. Update `DropRecord` parser/encoder to handle the optional dict_id byte
3. New `limnifs-core/src/dictionary_section.rs`
4. Add `DictionarySection` type + parser + encoder
5. Update `WriteContext` to train dicts after pass 1
6. Wire `WriteConfig::dictionaries` into the pipeline
7. Update `DropRecord` reader to pass dict_id to `decompress_with_dict`
8. Specs: round-trip with dict, without dict, mixed

## Related files

- `limnifs-core/src/drop_record.rs`
- `limnifs-format/src/manifest.rs` (section list)
- `limnifs-write/src/lib.rs` (assemble)
- New: `limnifs-core/src/dictionary_section.rs`

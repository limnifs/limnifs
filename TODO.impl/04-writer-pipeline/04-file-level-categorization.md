---
Component: 04-writer-pipeline
Task: 04-file-level-categorization
Status: done (2026-08-02, sessions 34-35)
Depends on: 04-classifier-seine
Unblocks: 04-specialized-codecs
Source: docs/dwarfs-multicodec-investigation.md (Tier 3)
Fix landed: limnifs-write/src/file_categorizer/ — FileCategorizer
  trait + FileCategorizerRegistry (OCP). Three skeleton categorizers
  shipped: pcm_audio, fits, csv_text. Wired into process_file
  before FastCDC. default_registry() is a OnceLock static.
---

# 04-file-level-categorization — Run categorizers before FastCDC

## Problem

LimniFS's pipeline is:

```
file → walk → FastCDC chunks (256 KiB avg) → per-chunk classify → codec
```

By the time our classifier sees data, **file context is gone**. A
FITS header at the start of the file ends up in only the first
chunk; subsequent chunks look like generic Binary. Same for WAV:
only the first chunk has the RIFF/WAVE header.

DwarFS's pipeline is:

```
file → categorize (whole file) → category-specific compressor
```

The categorizer sees the whole file, can parse file headers, and
can route the entire file to a specialized codec without chunking.

## Approach

Add a file-level categorization stage *before* FastCDC. If a
categorizer claims the file, route the whole file (as one drop) to
the codec it selected. Otherwise fall through to FastCDC.

```rust
fn process_file(pf: &PendingFile) -> ChunkedFileResult {
    // Try file-level categorizers first.
    if let Some(special) = FILE_CATEGORIZERS.categorize(&pf.path, &pf.data) {
        // FLAC for PCM, ricepp for FITS, FSST-Brotli for CSV, etc.
        let compressed = special.compress(&pf.data);
        return single_drop(pf, compressed, special.codec_id());
    }
    // Fall through to FastCDC + per-chunk classify (current path).
    fastcdc_and_classify(pf)
}
```

## Trade-offs

**Pro:**
- Unlocks specialized codecs (FLAC, ricepp, FSST).
- File header parsed once, params passed to codec.
- Better ratio on file types that don't share content across files
  (audio, images, scientific data).

**Con:**
- We lose CDC dedup on files that go through specialized codecs.
  The file becomes one drop content-addressed by its whole-file
  BLAKE3. For source code this would be a regression (two Linux
  tarballs share 95% of drops via CDC); for audio/images it's
  irrelevant (no two WAV files share content).

Mitigation: the file-level categorizer opts in per category. Source
code, text, and binary classes still go through FastCDC. Only
audio/image/specialized categories bypass it.

## Implementation sketch

1. New module `limnifs-write/src/file_categorizer.rs` with a trait:
   ```rust
   pub trait FileCategorizer: Sync {
       fn categorize(&self, path: &Path, data: &[u8]) -> Option<Categorization>;
   }
   
   pub struct Categorization {
       pub codec_id: u8,
       pub codec_params: Vec<u8>,  // codec-specific (PCM params, FITS bitpix, etc.)
   }
   ```
2. Registry of file categorizers (OCP — adding a categorizer is one
   file + register call):
   ```
   file_categorizers/
   ├── mod.rs                       (registry)
   ├── fits.rs                      (FITS magic + bitpix extraction)
   ├── pcm_audio.rs                 (WAV/AIFF + sample format)
   └── csv_text.rs                  (.csv/.tsv extension + content sniff)
   ```
3. `process_file` calls the registry before FastCDC. If any
   categorizer claims the file, the whole file becomes one drop.
4. FastCDC path unchanged for everything else.

## Acceptance criteria

- File categorizer registry exists and is OCP (new categorizer =
  new file + register call, no edits to dispatch code).
- At least one file categorizer shipped (start with FITS or WAV
  since the win is biggest there).
- Existing benchmarks (PHP source, synthetic) show no regression —
  they should still go through FastCDC unchanged.
- New benchmark dataset for the specialized codec's file type
  shows the expected ratio win.

## Dependencies

This task unblocks `04-specialized-codecs.md`. The two should ship
together: file-level categorization alone (without specialized
codecs) doesn't help.

## Out of scope

- Multi-codec trial (DwarFS's `--compressor=luck`). Defer to a
  separate task.
- Hotness/access-frequency categorization (DwarFS's
  `hotness_categorizer`). That's for tiered-storage scenarios we
  don't have yet.

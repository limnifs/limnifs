# 08 — Categorizer early-exit optimisation

- **Priority:** P1
- **Side:** LimniFS
- **Est. effort:** 1h

## Problem

The categorizer registry tries each categorizer in order
(fits → pcm_audio → csv_text → executable). Each categorizer
checks magic bytes. For a source-code file, all four categorizers
are tried before falling through to FastCDC. The ELF check
(4-byte magic) should bail after the first mismatched byte, but
the registry's loop overhead (4 function calls + trait dispatch)
adds up on trees with millions of files.

## Fix

1. Hoist the first-byte check into the registry loop: peek at
   `data[0]` and skip categorizers whose first magic byte doesn't
   match. E.g., only `fits` and `executable` start with `\x7F`;
   `pcm_audio` starts with `R` (RIFF); `csv_text` starts with
   printable ASCII.
2. A `first_byte_filter: fn(u8) -> bool` on each categorizer lets
   the registry skip non-matching categorizers without a function
   call.

## Expected impact

- **Create speed on source trees**: 5–10% improvement (saves 3 out
  of 4 categorizer calls on typical source files).

## Acceptance

- [ ] Registry uses first-byte filter to skip categorizers.
- [ ] Benchmark: source-tree create speed improves measurably.

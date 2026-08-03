# Proposal: FLAC — finish the LPC encoder

**Filed by:** LimniFS
**omnizip-rs crate:** `omnizip-flac`
**Severity:** feature gap (TODO 98 marked fixed; verify no regression on diverse corpus)

## Problem

`omnizip-flac` has had multiple rounds of LPC work (Phase 1, 2,
2B, 3 per the omnizip-rs commit log). TODO 98 was recently marked
fixed. LimniFS still sees FLAC routing disabled in production
because earlier revisions produced output that, while valid, lost
to general-purpose codecs on some audio fixtures.

The LimniFS `pcm_audio` categorizer is **off by default** today.
We want to enable it but need confidence the FLAC encoder wins on
a broad corpus, not just the synthetic sine waves used in
omnizip-flac's own tests.

## Proposed verification suite

Build a 200-track audio corpus spanning:

| Genre | Source | Approx size |
|---|---|---|
| Classical | MusOpen public-domain WAVs | 500 MB |
| Speech | LibriSpeech dev-clean | 200 MB |
| Ambient | Free Music Archive CC-licensed | 300 MB |
| Pop | Internet Archive 78rpm collection | 200 MB |
| Synthetic | swept sine, white noise, pink noise | 50 MB |

For each track, compare:

1. FLAC via `omnizip-flac` (current revision).
2. FLAC via `libFLAC` reference (CLI binary, run as subprocess for
   testing only — not in source tree).
3. Plain LZ4 (LimniFS's binary fallback).
4. Plain ZSTD L12 (LimniFS's high-ratio binary).

FLAC wins iff `omnizip-flac` ratio is within 5% of `libFLAC` AND
beats both LZ4 and ZSTD L12 by ≥ 10%.

## What LimniFS proposes

1. LimniFS contributes the corpus-fetching script + differential
   harness under `tests/audio_corpus/` (MIT-licensed).
2. omnizip-rs runs the harness on each FLAC revision and posts
   results to the TODO 98 thread.
3. Once FLAC wins on ≥ 90% of the corpus, LimniFS enables the
   `pcm_audio` categorizer by default in `balanced` and `max-ratio`
   profiles.

## Why LimniFS cares

WAV/AIFF content (audio samples, podcast archives, music libraries)
is a real workload. Today LimniFS routes them through ZSTD which
gives ~60% ratio; FLAC should give ~50%. That's a meaningful win
on archival audio.

## Effort estimate

- LimniFS side: 2 days (corpus script + harness).
- omnizip-rs side: TBD depending on FLAC revisions needed.

## Related

- omnizip-rs TODOs 98, 99, 100 (FLAC-related).
- libFLAC reference: https://github.com/xiph/flac

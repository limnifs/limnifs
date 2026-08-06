# 15 — Streaming directory walk

- **Priority:** P1
- **Side:** LimniFS
- **Est. effort:** 4h

## Problem

`write_directory_with_config` walks the tree sequentially in `WriteContext::walk`,
collects all `PendingFile`s into a Vec, then dispatches them to rayon workers.
For source trees with millions of files, the walk finishes before any compress
starts — wasted opportunity to overlap I/O with CPU.

## Fix

Convert `walk` into a producer that emits files onto a bounded crossbeam
channel. N rayon workers consume from the channel and call `process_file`.
The walk and the compress run concurrently.

For trees where the walk is fast (warm cache, few files), the channel is
a no-op (just dispatches). For trees where the walk is slow (cold cache,
deep nesting, millions of small files), this hides walk latency behind
compress.

## Expected impact

- 10–30% on huge source trees (>100K files)
- No change on small trees

## Acceptance

- [ ] `write_directory_with_config` overlaps walk + compress
- [ ] Output bytes unchanged
- [ ] Benchmark: deep-tree create improves measurably

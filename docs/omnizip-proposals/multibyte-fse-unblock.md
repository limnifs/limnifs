# Proposal: Multi-byte FSE — decouple from differential harness

**Filed by:** LimniFS
**omnizip-rs crate:** `omnizip-zstd`
**Severity:** unblock TODO 84

## Problem

`omnizip-rs TODO.complete/84-multibyte-fse.md` proposes a level-2
FSE decode table that processes 2–4 input bytes per state
transition, for ~30% throughput gain. The TODO is **blocked** on
TODO 87 (differential harness), with this rationale:

> The current FSE decoder has subtle correctness corner cases...
> Multi-byte decode doubles the surface area for bugs; we need the
> differential harness to validate.

LimniFS agrees that differential testing is essential. But the
multi-byte decoder can be **landed first** as a parallel
implementation, validated against the *existing* scalar decoder
(rather than the C reference), and only enabled by default once
the differential harness confirms byte-identical output.

## Proposed sequencing

### Phase 1 — Parallel implementation (unblocked)

Implement `interleaved::decode` alongside `fse::decode`. Validate
exclusively against the scalar decoder:

```rust
#[cfg(test)]
mod differential_tests {
    fn check_against_scalar(input: &[u8], table: &Table) {
        let scalar = fse::decode(input, table);
        let interleaved = interleaved::decode(input, table);
        assert_eq!(scalar, interleaved, "FSE divergence on input of {} bytes", input.len());
    }

    // Proptest: random inputs of length 0..=64KB, random tables
    // within ZSTD's FSE parameter space.
    #[test]
    fn random_inputs_match_scalar() { /* ... */ }

    // Calgary corpus + Enwik8 chunks + Silesia chunks.
    #[test]
    fn real_corpus_matches_scalar() { /* ... */ }
}
```

The scalar decoder is the oracle. Multi-byte is correct iff it
agrees with scalar on every input the test suite covers.

Dispatch stays scalar-by-default:

```rust
pub fn decode(input: &[u8], table: &Table) -> Vec<u8> {
    #[cfg(feature = "multibyte-fse")]
    if input.len() >= MULTIBYTE_THRESHOLD {
        return interleaved::decode(input, table);
    }
    scalar::decode(input, table)
}
```

### Phase 2 — Enable by default (needs differential harness)

Once omnizip-rs TODO 87 lands, run the differential harness
against the C reference. If multi-byte agrees with C, flip the
default. If not, fix bugs first.

## Why this unblocking works

The TODO's blocking rationale is "subtle correctness corner cases."
Those corner cases exist relative to the C reference, but the
*scalar decoder is already shipping as the production decoder* —
its behaviour is the de-facto LimniFS-compatible spec. Multi-byte
matching scalar is therefore sufficient for LimniFS to adopt; the
differential harness is the cherry on top for parity testing
against `zstd` CLI.

## Acceptance (Phase 1)

- [ ] `interleaved::decode` exists with a level-2 lookup table.
- [ ] Differential tests against scalar pass on Calgary, Silesia,
      Enwik8 chunks, and 10⁶ random inputs (proptest).
- [ ] Throughput improvement ≥ 20% on ZSTD level-19 Enwik8 decode.
- [ ] Behind a `multibyte-fse` feature flag (default off).

## Why LimniFS cares

ZSTD is LimniFS's primary text codec in `balanced` and `max-read`
profiles. 20% faster ZSTD decode = 20% faster `cat` on
text-heavy images. LimniFS would enable the feature for the
`max-read` profile and benchmark the win.

## Effort estimate

5–7 days (same as omnizip-rs's estimate; the unblocking is
procedural, not technical):
- 3 days: level-2 table generator + decode loop.
- 2 days: proptest differential harness against scalar.
- 1 day: Silesia/Enwik8 benchmark.
- 1 day: code review + cleanup.

## Related

- omnizip-rs TODO 84.
- ACM (2024). *Efficient and Portable ANS Encoding for Multi-Byte
  Integer Sequences.* https://dl.acm.org/doi/10.1145/3712285.3759825
- Kosolobov (2022). *Efficiency of ANS Entropy Encoders.*
  https://arxiv.org/pdf/2201.02514

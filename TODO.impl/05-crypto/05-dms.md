# 05 — Dead man's switch

- **Status:** done — Shamir in limnifs-core/src/shamir.rs; time-lock variant deferred on hardware-drift calibration
- **Phase:** 3
- **Depends on:** 05-key-wrap-hpke
- **Design refs:** §8 (DMS), §16 (open question 3)

## Goal

DMS primitives: (a) Shamir k-of-n escrow split/collect of the image key with
named custodians and release conditions; (b) time-lock puzzle seal/solve
(iterated squaring) if calibration question resolves.

## Notes

- Policy serialization is manifest schema (01); this crate provides the math only.
- v1 likely Shamir-only; time-lock gated on the hardware-drift calibration story (design §16.3).
- `limni dms status|solve|collect` consumes these APIs (10).

## Acceptance

- k-of-n vectors for k=2..5: any k shares reconstruct; k−1 fail; escrow metadata survives flatten/turnover (vector).

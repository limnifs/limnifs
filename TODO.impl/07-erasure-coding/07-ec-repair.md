# 07 — EC repair

- **Status:** done — limnifs-core/src/ec_repair.rs (offline reconstruction)
- **Phase:** 3
- **Depends on:** 07-reed-solomon-slabs, 08-locator-trait-registry
- **Design refs:** §8, §10.2 (IPFS churn)

## Goal

Detect degraded slabs (locator failures during read), reconstruct from
surviving shards, and re-emit missing shards to locators (`limni repair`).

## Notes

- Repair is a background/CLI operation, never on the hot read path — reads fail over to surviving shards transparently instead.
- Re-emitted shards verify against recorded shard hashes before upload.

## Acceptance

- Kill-shard vector: image remains fully readable with m shards absent; repair restores full redundancy; locator audit confirms.

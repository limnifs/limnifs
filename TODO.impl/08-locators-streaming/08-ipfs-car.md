# 08 — IPFS locator and CAR interop

- **Status:** done — limnifs-core/src/ipfs_locator.rs (gateway + CARv1 codec, behind http feature)
- **Phase:** 3
- **Depends on:** 08-locator-trait-registry
- **Design refs:** §10.2 (IPFS scale), §4 (multihash-compatible DropIds)

## Goal

`ipfs:` locator (gateway + kubo RPC), `export-car`/`import-car` between
`.limni` and IPLD CAR, multihash bridging of `DropId`s.

## Notes

- Overlay chains pin as DAGs (delta DAG = version DAG); EC slabs survive churn (with 07).
- CAR mapping spec lives in 01-spec (SSOT); this crate implements it.

## Acceptance

- CAR round-trip vector: export → import resolves byte-identical tree and identical `ManifestRoot`.

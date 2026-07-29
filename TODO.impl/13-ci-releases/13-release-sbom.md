# 13 — Release pipeline, SBOM, license scan

- **Status:** pending
- **Phase:** 1
- **Depends on:** 13-actions-matrix
- **Design refs:** §11 (supply chain), §1 (GPL exclusion)

## Goal

Tag-driven releases: reproducible `limni` builds per platform, SBOM
(CycloneDX) per artifact, hard-fail license scan (GPL-3/AGPL anywhere in the
dependency tree = release abort), sigstore keyless signing of artifacts,
crates.io publish in dependency order.

## Notes

- Reproducibility proven in CI: build twice from the same tag, byte-compare,
  publish checksums.
- License scan covers `limnifs-frozen2` separately (vendored MIT/Apache
  allowed; GPL never) — component 09's boundary enforced mechanically.
- Release notes generated from task files marked done since the previous tag
  (SSOT for history, root README §4).

## Acceptance

- A dry-run tag produces byte-identical artifacts across two runs, SBOM +
  signatures attached; a branch adding a GPL-3 dep fails the scan — linked
  run as evidence.

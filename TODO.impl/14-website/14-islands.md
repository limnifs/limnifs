# 14 — Vue islands

- **Status:** pending
- **Phase:** 1 (TierDiagram), 2 (DropViz)
- **Depends on:** 14-site-skeleton; DropViz also needs 04-chunking-fastcdc semantics from spec
- **Design refs:** §3, §6

## Goal

The interactive layer: `SiteHeader`, `SiteSearch` (Pagefind), `ThemeToggle`,
`Reveal`, `CopyButton`, `InstallTabs` (ported patterns from rnpgp.org), plus
`TierDiagram` (interactive pipeline explorer: hover a tier → its codecs,
latency, and policy light up) and, as a stretch, `DropViz` (WASM: drop a file,
client-side FastCDC + BLAKE3 chunks it into drops with dedup counts — the
identity rule demoed in the browser).

## Notes

- Hydration discipline: islands hydrate `client:visible` except header/search
  (`client:load`); everything works with JS disabled (progressive
  enhancement, as rnpgp.org).
- `prefers-reduced-motion` respected everywhere (Reveal's pattern is the
  model).
- DropViz must use the *spec's* FastCDC parameters and BLAKE3 — it is a
  living conformance demo, not a toy approximation; if it can't match the
  spec it doesn't ship.

## Acceptance

- Each island has an e2e CDP check (rnpgp.org's `test:e2e` pattern); JS-off
  snapshot test passes; CI green (link the run).

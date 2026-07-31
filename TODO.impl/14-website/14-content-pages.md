# 14 — Content pages (home, about, install, docs, blog)

- **Status:** done — index, about, format, scenarios, adapters, docs, blog pages
- **Phase:** 1
- **Depends on:** 14-site-skeleton
- **Design refs:** §3, §12; design doc §1 (lineage), §15 (phases)

## Goal

The five v1 pages with real copy: home (5 differentiators, tier diagram,
audience cards), about (rnpgp.org/about structure: origin/mission, ecosystem
table of the 5 org repos, use cases, spec-first stance, partners, get-started
steps, "The name & the mark" — λίμνη, L-I-M-N-I acronym, limnology
vocabulary, light/dark logo panels), install, docs (spec rendered from pinned
`limnifs/spec` tag via a content loader), blog collection scaffold.

## Notes

- Copy is semantically driven: page text uses drop/slab/tier/turnover
  vocabulary exactly as defined in design §3 — the site teaches the model.
- Docs pipeline: build-time fetch of the pinned spec tag (like rnpgp.org's
  `fetch-sources` pattern), never hand-copied (SSOT).
- Honest status labeling: pre-Phase-1 features marked "in design" / "planned"
  — the site never claims unshipped capability.

## Acceptance

- All pages render with zero lychee errors; about page reviewed against
  rnpgp.org/about for structure parity; docs page shows the pinned spec tag
  in the footer; CI green (link the run).

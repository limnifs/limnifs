# 14 — website (limnifs.org)

The public face: limnifs.org, built with the same stack and conventions as
[rnpgp.org](https://www.rnpgp.org) (reference implementation at
`~/src/rnp/rnpgp.org/`): **Astro 7 + Vite 8 + Tailwind 4 (`@tailwindcss/vite`)
+ Vue 3 islands**, static, deployed to GitHub Pages by Actions.

- **Phase:** 1 (skeleton can start once the design doc is public-ready)
- **Repo:** `limnifs/limnifs.org`
- **Design refs:** §3 (vocabulary — the site's design system is built on it), §12 (audiences)

## Responsibilities (MECE)

**Owns:**

- The site: homepage, about, install/download, docs (rendered from `limnifs/spec`
  at a pinned tag), blog, 404, RSS, sitemap.
- The design system: tokens, fonts, components, Vue islands.
- Site CI: build, lychee link checks, Pagefind search index, e2e via CDP,
  GitHub Pages deploy (reusing 13's org-level workflows).

**Does NOT own:** spec content (01 — the site *renders* it), release
artifacts (13 — the site *links* them), product code.

## Design system — "The Limnologist's Field Notes"

Same bones as rnpgp.org ("The Cryptographer's Paper": IBM Plex self-hosted,
light-first with full dark mode, `card`/`mono-label`/`band-*` semantic
classes, §-numbered sections with `PageHero` + `SectionHeader` eyebrows), but
the palette *is* the architecture:

| Token | Color role | Meaning |
|---|---|---|
| `epilimnion` | bright teal/cyan accent | hot tier, fast ingest |
| `thermocline` | mid blue gradient band | the deepening boundary |
| `hypolimnion` | deep navy (dark surfaces) | cold tier, maximum density |
| `gold` | highlight/CTA | kept from rnpgp.org family |
| hero gradient | teal → deep navy, top→bottom | a depth profile of a lake |

Typography: IBM Plex Sans (text/display) + Plex Mono (code, hashes, labels),
self-hosted. Hashes and `DropId`s are always mono — the site's texture should
feel like a field notebook full of lake soundings and hash strings.

## Pages (v1)

- **Home** — hero ("limn your image" + depth-gradient), the 5 differentiators
  (architecture 03-comparison §3), tier diagram, install strip, audience cards.
- **About** — modeled on rnpgp.org/about: why LimniFS exists (DwarFS lineage,
  GPL-free break), ecosystem table (the 5 repos), use cases (tebako, CI
  artifacts, containers, archival, IPFS), standards/spec-first stance,
  partners, get-started steps, and "The name & the mark" last: λίμνη, the
  L-I-M-N-I acronym, and the limnology vocabulary (with light/dark logo
  panels).
- **Install** — per-platform instructions (Vue `InstallTabs` island).
- **Docs** — spec rendered from pinned `limnifs/spec` tag + CLI man pages.
- **Blog** — Markdown/AsciiDoc collection (release notes, design notes).

## Vue islands (v1)

`SiteHeader`, `SiteSearch` (Pagefind), `ThemeToggle`, `Reveal`, `CopyButton`,
`InstallTabs` — same set as rnpgp.org — plus LimniFS-specific:
`TierDiagram` (interactive epilimnion→hypolimnion pipeline explorer),
`DropViz` (stretch: drop a local file, watch client-side FastCDC+BLAKE3
chunk it into drops — the format's core idea demoed in the browser, WASM).

## Tasks

- [14-site-skeleton.md](14-site-skeleton.md)
- [14-content-pages.md](14-content-pages.md)
- [14-islands.md](14-islands.md)

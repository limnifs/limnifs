# 14 — Site skeleton and deploy

- **Status:** pending
- **Phase:** 1
- **Depends on:** none (design doc content feeds 14-content-pages)
- **Design refs:** §3; rnpgp.org reference at `~/src/rnp/rnpgp.org/`

## Goal

`limnifs/limnifs.org` scaffolded: Astro 7 + `@astrojs/vue` 7 + Tailwind 4 via
`@tailwindcss/vite` (Vite 8), `BaseLayout`, `PageHero`, `SectionHeader`,
`SiteFooter`, global.css `@theme` tokens (epilimnion/thermocline/hypolimnion/
gold), self-hosted IBM Plex, light+dark themes, sitemap + robots + RSS stubs.

## Notes

- Follow rnpgp.org's `astro.config.mjs` shape (`site`, `trailingSlash`,
  `compressHTML: true`, vue + sitemap integrations, shiki dual themes).
- Scripts mirror rnpgp.org: `dev/build/preview`, `postbuild` Pagefind,
  `check:links` (lychee), `test:e2e` (CDP).
- GitHub Pages deploy via Actions, consuming org reusable workflows (13).
- AGENTS.md written at the same time (rnpgp.org's is the model) — this repo
  will also be agent-maintained.

## Acceptance

- `npm run build` green in CI on linux + macOS; deploys to Pages on main;
  lychee clean; Lighthouse ≥ 95 performance on the empty shell (link the run).

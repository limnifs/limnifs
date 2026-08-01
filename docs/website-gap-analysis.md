# Website gap analysis — limnifs.github.io

**Date:** 2026-08-01
**Goal:** Identify what's missing for a "big splash" launch

## Current state

The website has a solid spec section (16 MDX pages covering the full
format) but is missing the content that actually converts visitors into
users: a compelling homepage, quick-start guides, interactive demos,
download links, and community content.

### What exists (15 pages)

| Page | Status |
|---|---|
| `index.astro` (homepage) | Basic; needs hero + CTA |
| `about.astro` | Present |
| `blog/index.astro` | Index only; 1 post (v0.1.0 release — just added) |
| `format/index.astro` + `manifest.astro` | High-level format overview |
| `spec/` (16 pages) | Comprehensive format spec |
| `scenarios/index.astro` | Use-case scenarios |
| `adapters/index.astro` | Language adapters info |
| `docs/index.astro` | Docs landing |

### What's missing (the gaps)

## P0 — must-have for launch

### 1. Homepage redesign

The homepage needs to make a visitor understand LimniFS in 10 seconds:
- Hero: one-sentence pitch + `cargo install limni` install command
- "Why LimniFS" 3-card grid: Pure Rust / Content-Addressed / Faster than DwarFS
- Quick start: 4 terminal commands (create → verify → ls → extract)
- Benchmark teaser: "1.6x faster than DwarFS" with chart
- Codec portfolio table
- CTA: "Get started" + "Read the spec"

### 2. Download / install page

`/install`:
- `cargo install limni` (primary)
- Pre-built binaries from GitHub releases (linux x86_64/aarch64, macOS)
- Build from source instructions
- Docker image (future)

### 3. Quick start guide

`/docs/quick-start`:
- Create your first `.lim` image
- Verify, list, extract
- Mount via FUSE
- Compress with different codecs
- Seal with a master key

### 4. Blog infrastructure

The blog index exists but has no RSS feed, no post layout, and only 1
post. Needs:
- `BlogPost.astro` layout (referenced by the release post but not yet
  created as a component)
- RSS feed (`/rss.xml`)
- Post listing with date, tags, excerpt

## P1 — high-impact for adoption

### 5. Interactive demo

A WASM-powered playground where visitors can:
- Drag-drop a small directory
- See it packed into a `.lim` image (compiled LimniFS to WASM)
- Browse the image tree
- See the compression ratios per file

This is the "wow factor" that makes people share the link.

### 6. API documentation

rustdoc-generated API docs hosted at `/api/`:
- `limnifs-core` trait docs
- `Codec` trait + `CodecRegistry`
- `ManifestCursor` + parsers
- Example code for library consumers

### 7. Comparison page (expanded)

The spec already has `spec/comparison.mdx` but it needs to be a
top-level `/compare/` page with:
- Side-by-side feature matrix (tar, SquashFS, DwarFS, LimniFS)
- Interactive benchmark chart (Chart.js or D3)
- "When to choose LimniFS" decision tree

### 8. CLI reference

`/docs/cli/`:
- Every `limni` subcommand documented
- `limni limn`, `limni ls`, `limni cat`, `limni extract`, `limni verify`,
  `limni mount`, `limni inspect`, `limni seal`, etc.
- Auto-generated from clap's `--help` output

## P2 — polish and depth

### 9. Architecture deep-dive

`/docs/architecture/`:
- Three-layer model (drop store / metadata / manifest) with diagrams
- Content-addressing flow (BLAKE3 → DropId → slab → representation)
- Codec registry design (OCP pattern)
- Seine classifier internals
- FastCDC chunking algorithm

### 10. Tutorial series

`/docs/tutorials/`:
- "Your first LimniFS image" (beginner)
- "Choosing the right codec" (intermediate)
- "Encrypting and sealing images" (intermediate)
- "Remote slabs with HTTP locators" (advanced)
- "Erasure coding for fault tolerance" (advanced)
- "Building a LimniFS-powered CDN" (expert)

### 11. Community page

`/community/`:
- GitHub Discussions link
- Contributing guide
- Architecture decision records (ADRs)
- Roadmap (link to TODO.roadmap)

### 12. Visual identity

- Logo / wordmark for LimniFS
- Consistent color palette across all pages
- Favicon set
- Open Graph image for social sharing
- Syntax-highlighted code blocks with a LimniFS theme

## P3 — go further (the "splash")

### 13. Live benchmark dashboard

A CI-driven page that runs benchmarks on every release and shows:
- LimniFS vs DwarFS vs SquashFS vs tar.gz
- Create/extract/size across multiple corpora
- Interactive charts with historical trends

### 14. Format visualization (DropViz)

An interactive visualization of a `.lim` image's internal structure:
- The three-layer model rendered as nested boxes
- Click a drop → see its BLAKE3 hash, codec, compression ratio
- Click a directory node → see its Merkle hash
- Overlay the manifest's Merkle root computation

### 15. Ecosystem page

`/ecosystem/`:
- omnizip-rs (codec ports)
- tebako (packaging integration)
- limnifs-py (Python reference reader)
- limnifs-frozen2 (DwarFS migration)
- Community adapters (Ruby, TS, Python FFI bindings)

### 16. Video content

Short (2-3 min) screencasts:
- "LimniFS in 60 seconds"
- "Create, verify, extract"
- "Mount a .lim image"
- "Seal with encryption"

Hosted on YouTube, embedded on the website.

## Implementation plan

| Priority | Items | Effort | Impact |
|---|---|---|---|
| P0 | Homepage, install, quick-start, blog infra | 2 days | Launch-ready |
| P1 | Demo, API docs, comparison, CLI ref | 1 week | Adoption-ready |
| P2 | Architecture, tutorials, community, visual identity | 2 weeks | Deep engagement |
| P3 | Benchmark dashboard, DropViz, ecosystem, video | Ongoing | "Big splash" |

## The vision

The LimniFS website should be the destination that makes a developer
think: *"This is the filesystem format I've been waiting for."*

It needs to:
1. **Explain** what LimniFS is in 10 seconds (hero)
2. **Prove** it works (interactive demo + benchmarks)
3. **Teach** how to use it (quick start + tutorials)
4. **Document** every detail (spec + API + CLI)
5. **Inspire** what's possible (scenarios + ecosystem + roadmap)

The spec section is already excellent. The homepage and quick-start are
the highest-leverage improvements — they're the first thing every visitor
sees.

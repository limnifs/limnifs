# 01 — Spec restructure plan (multi-file, onion-layered, bit-level)

- **Status:** pending (blocked on this plan's approval)
- **Phase:** 0
- **Depends on:** [2026-07-29-wire-format-pivot.md](../../docs/superpowers/specs/2026-07-29-wire-format-pivot.md) D6 (accepted)
- **Design refs:** pivot decision D6; original design doc §5 (now
  superseded for the metadata layer).

## Goal

Restructure SPEC.md (the current single 1359-line file) into a
multi-file document organized in onion layers. Every fixed-width
type specified down to bit position. Educational; readers enter at
the depth they need.

## Onion layers (information exposure)

| Layer | Audience | Content |
|---|---|---|
| **0 Orientation** | Everyone (5-minute read) | README, how-to-read, glossary, conformance summary |
| **1 Concepts** | Architects, evaluators | overview, identity, three layers, representations, versioning, distribution, derivations |
| **2 Wire format** | Implementers (section level) | drop store, metadata, manifest, locators, Merkle B-tree |
| **3 Bit-level** | Implementers (byte/bit level) | every fixed-width type, byte-offset table, bit-position diagram |
| **4 Algorithms** | Implementers (operation level) | read path, build path, deepen, delta, flatten, turnover, verify |
| **5 Conformance** | Conformance engineers, adapter authors | vector classes, test format, reference reader contract |
| **6 Reference** | Everyone (look-up) | registries (TOML), multi-language adapter guides, appendices, decision records |

A reader who only reads Layer 0 understands what LimniFS is and how to
use it. A reader who reads Layers 0–2 can navigate a `.lim` file. A
reader who reads Layer 3 can implement a parser. A reader who reads
Layer 4 can implement a full reader/writer.

## File tree (proposed)

```
limnifs/spec/
├── README.md                          # L0 entry point
├── 00-how-to-read.md                  # L0 reading guide
├── 01-glossary.md                     # L0 terms
├── 02-conformance-summary.md          # L0 conformance overview
├── concepts/                          # L1
│   ├── 10-overview.md
│   ├── 11-identity.md
│   ├── 12-layers.md
│   ├── 13-representations.md
│   ├── 14-versioning.md
│   ├── 15-distribution.md
│   └── 16-derivations.md
├── wire-format/                       # L2
│   ├── 20-file-layout.md
│   ├── 21-drop-store.md
│   ├── 22-metadata.md
│   ├── 23-manifest.md
│   ├── 24-locators.md
│   └── 25-merkle-btree.md
├── bit-level/                         # L3
│   ├── 30-slab-header.md
│   ├── 31-drop-record.md
│   ├── 32-representation.md
│   ├── 33-inode.md
│   ├── 34-merkle-btree-node.md
│   ├── 35-manifest-header.md
│   ├── 36-manifest-sections.md
│   ├── 37-locator-entry.md
│   ├── 38-history-entry.md
│   └── 39-tree-op.md
├── algorithms/                        # L4
│   ├── 40-read-path.md
│   ├── 41-build-path.md
│   ├── 42-deepen.md
│   ├── 43-delta-build.md
│   ├── 44-flatten.md
│   ├── 45-turnover.md
│   ├── 46-verify.md
│   └── 47-merkle-root.md
├── conformance/                       # L5
│   ├── 50-vectors.md
│   ├── 51-test-format.md
│   └── 52-reference-reader.md
├── registries/                        # L6 data
│   ├── README.md
│   ├── aead.toml
│   ├── codec.toml
│   ├── locator.toml
│   ├── classifier.toml
│   └── feature-flags.toml
├── multi-language/                    # L6 adapters
│   ├── 60-adapter-paths.md
│   ├── 61-ruby-adapter.md
│   ├── 62-typescript-adapter.md
│   └── 63-python-reference.md
├── appendices/                        # L6 reference
│   ├── A-references.md
│   ├── B-change-log.md
│   ├── C-decision-records.md
│   └── D-open-questions.md
├── schema/                            # DEPRECATED (FlatBuffers; kept per never-delete rule)
│   ├── DEPRECATED.md
│   ├── types.fbs
│   └── manifest.fbs
└── SPEC.md                            # Redirect to README.md (backward compat)
```

Total: ~40 files. Each focused (target 200–500 lines max per file).

## Migration plan

The migration is incremental — each step is a PR. The current SPEC.md
remains the source of truth until the migration completes; then it
becomes a redirect.

1. **Seed structure** — create the directory tree and Layer 0 files
   (README.md, 00-how-to-read.md, 01-glossary.md,
   02-conformance-summary.md). The current SPEC.md stays as the
   authoritative source during migration.
2. **Migrate Layer 1 (concepts)** — port from current SPEC.md §1
   (Foundational invariants), §2 (Terminology). Split identity /
   layers / representations / versioning / distribution / derivations
   into separate concept files. Cross-reference Layer 2.
3. **Migrate Layer 2 (wire format)** — port from §3 (Drop store), §4
   (Filesystem metadata), §5 (Manifest). Add new section: deterministic
   Merkle B-tree (per pivot D2).
4. **Author Layer 3 (bit-level)** — new content. Every fixed-width
   type gets a byte-offset table and bit-position diagram. This is
   the most exacting work; expected to be ~10 files × 200–400 lines.
5. **Migrate Layer 4 (algorithms)** — port from §6 (Two-level
   addressing prose), §8 (Derivation operations prose).
6. **Migrate Layer 5 (conformance)** — port from §19 (Conformance).
7. **Populate Layer 6 (registries + multi-language + appendices)** —
   port from §10–§14 (registry content into TOML files); write
   multi-language adapter guides (Ruby, TS, Python reference); port
   appendices.
8. **Replace SPEC.md** — replace content with a redirect to README.md
   (backward compat for existing links).
9. **Deprecate FlatBuffers schema** — `schema/DEPRECATED.md` (per
   pivot D1; never-delete rule keeps files).

## Acceptance

- Every fixed-width type in the spec has a Layer 3 file with byte-offset
  table and bit-position diagram. No "TBD" or "to be specified" in
  Layer 3.
- The Python reference reader implements the format from Layer 3 alone
  (spec-sufficiency oracle; verified by conformance).
- At least one adapter (Ruby OR TypeScript) implements the format from
  Layer 3 alone — validates the multi-language spec-first story.
- The old SPEC.md either redirects to README.md or is clearly marked
  as historical/superseded.
- Conformance vectors cover at least the 10 classes from current §19.1.
- All cross-references between files resolve (no broken markdown
  links; checked by CI).

## Open questions (resolve during step 1)

- **Diagrams in Layer 3**: ASCII art (universal, diffs cleanly) vs
  formal notation (RFC-style byte layout) vs both? Recommendation:
  ASCII primary, formal notation where unambiguous.
- **Cross-reference style**: markdown links (resilient to renumbering)
  vs section numbers (stable across restructures) vs both?
  Recommendation: markdown links primary; section numbers in titles.
- **Single-document output**: should the multi-file spec be available
  as a single PDF/printable document (generated from the source)?
  Recommendation: yes, via a build step that concatenates files in
  reading order. Defer to follow-up task.

## Out of scope

- Implementation of the format (that's `01-wire-format.md`, a separate
  task opening once the restructure is approved).
- Conformance vector authoring (that's `02-conformance`).
- Adapter implementations (those are external contributions; the spec
  enables them, doesn't write them).

# 10 — cli

`limni`: the single binary users touch. Thin UX layer over the crates — no
format, crypto, or merge logic lives here.

- **Phase:** 1
- **Crate:** `limni`
- **Design refs:** §3 (vocabulary), §6–§10 (commands map to components)

## Responsibilities (MECE)

**Owns:**

- Subcommand UX: `limni limn` (build), `ls`/`cat`/`stat` (inspect), `shell` (interactive read-only exploration), `verify` (hash+AEAD+signature), `deepen` (tier policy), `merge --flatten` / `turnover`, `export-car`/`import-car`, `dms status|solve|collect`, `mount`/`unmount` (delegates to 11).
- **`limni shell`** — interactive REPL that drops the user inside a `.lim` image for read-only exploration. No mount, no FUSE, no root required. Supports POSIX-like navigation (`cd`, `ls`, `cat`, `stat`, `find`, `grep`, `pwd`, `du`) plus LimniFS-specific commands (`manifest`, `slabs`, `verify`, `history`, `identity`, `tree`). All read-only — the image is immutable. Useful for debugging, conformance inspection, and education.
- **No foreign-format commands.** `limni` only reads/writes `.lim`. DwarFS Frozen2 import lives in the separate `limnifs/limnifs-frozen2` repo as its own tool — `limni` never links frozen2 code. This keeps LimniFS's identity clean: one format, one CLI.
- Machine-readable output: `--json` on every command; stable exit codes.
- Key/locator UX: recipients, keyring integration, mirror config files.

**Does NOT own:** any algorithm or format logic — every command is a composition of 03–09 crate calls. If logic lands here, it gets pushed down.

## Invariants

- Vocabulary is semantically driven: commands and output use design §3 terms (drop, slab, tier names, turnover), so the CLI teaches the model.
- Every mutation command prints the resulting `ManifestRoot` — identity is always visible.

## Tasks

- [10-cli-skeleton.md](10-cli-skeleton.md)
- [10-cli-limn-merge.md](10-cli-limn-merge.md)
- [10-cli-inspect-verify.md](10-cli-inspect-verify.md)

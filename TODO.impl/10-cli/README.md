# 10 — cli

`limni`: the single binary users touch. Thin UX layer over the crates — no
format, crypto, or merge logic lives here.

- **Phase:** 1
- **Crate:** `limni`
- **Design refs:** §3 (vocabulary), §6–§10 (commands map to components)

## Responsibilities (MECE)

**Owns:**

- Subcommand UX: `limni limn` (build), `ls`/`cat`/`stat` (inspect), `verify` (hash+AEAD+signature), `deepen` (tier policy), `merge --flatten` / `turnover`, `import-dwarfs`, `export-car`/`import-car`, `dms status|solve|collect`, `mount` (delegates to 11).
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

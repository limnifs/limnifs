# 12 — tebako-integration

The first real consumer (design §12, §15 Phase 1 exit). Proves LimniFS does
what dwarfs-t does for tebako, better, and defines the migration path.

- **Phase:** 1
- **Artifacts:** tebako-side glue (press/mount), parity test suite
- **Repos:** cross-org — adapter + parity suite live in `limnifs/limnifs` (or `tamatebako/libtfs` for the VFS adapter half); tebako itself stays in `tamatebako/tebako`. The adapter consumes `limnifs-core` as a versioned crates.io dep, never a git path — the org boundary is exercised as a real external-consumer contract.
- **Design refs:** §15 (Phase 1), §5.1 (migration), §12 (audience)

## Responsibilities (MECE)

**Owns:**

- `tebako press --format limni`: produce tebako packages carrying `.limni` images (alongside existing dwarfs-t path during transition).
- tebako runtime mount path via libtfs adapter or direct `limnifs-core` static link.
- **Parity suite:** same Ruby/Node app packaged both ways (dwarfs-t vs LimniFS); diff file trees, compare cold start, image size, memory.
- Migration tooling story: `limni import-dwarfs` on existing `.tfs`/DwarFS images (uses 09), validated on tebako's real published images.

**Does NOT own:** LimniFS internals (all components), libtfs's multi-format VFS role (stays; LimniFS becomes one more adapter, then the primary one).

## Invariants

- No tebako behavioral regressions: packaged apps must not observe any difference except performance/size.
- Transition is reversible: dwarfs-t packaging path remains until parity suite is green for two consecutive tebako releases.

## Tasks

- [12-press-consume-limni.md](12-press-consume-limni.md)
- [12-parity-tests.md](12-parity-tests.md)

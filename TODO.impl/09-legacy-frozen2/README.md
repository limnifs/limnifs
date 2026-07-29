# 09 — legacy-frozen2

`limnifs-frozen2`: read-only adapter for DwarFS v2 (Thrift/Frozen2) images and
one-way migration into `.limni`. Exists for tebako's installed base.

- **Phase:** 1
- **Repo:** `limnifs/limnifs-frozen2` (separate repo = license-scan boundary; the only repo where vendored MIT/Apache DwarFS code may appear)
- **Crate:** `limnifs-frozen2` (separate so the GPL-free core never links it)
- **Design refs:** §5.1 (legacy reading), §1 (GPL constraints)

## Responsibilities (MECE)

**Owns:**

- Frozen2 metadata decoding and section walking, clean-room; MIT/Apache `oxalica/dwarfs` reader may be vendored where useful (license-compatible).
- Exposing a legacy image through the *same* `Image`/`DropSource` traits as 03, so mounters can't tell the difference.
- `import-dwarfs`: re-encode a Frozen2 image into a `.limni` image (via 04's pipeline), preserving tree semantics and recording provenance in the new manifest.

**Does NOT own:** writing Frozen2 (explicitly never built — design §2 non-goals), Thrift decoding of v1 metadata beyond what migration requires, LimniFS format decisions.

## Invariants

- Read-only with respect to the legacy format; the only output format is `.limni`.
- License boundary: this crate is the *only* place third-party DwarFS code may appear; CI license-scans it separately.

## Tasks

- [09-frozen2-reader.md](09-frozen2-reader.md)
- [09-import-dwarfs.md](09-import-dwarfs.md)

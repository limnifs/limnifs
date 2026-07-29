# 08 — Locator trait, registry, file locator

- **Status:** pending
- **Phase:** 2
- **Depends on:** 03-drop-store-reader
- **Design refs:** §10.1 (registry), §4 (lying-locator detection)

## Goal

`Locator` trait + scheme registry + the `file:` locator (mmap); mirror
configuration parsing and priority/racing policy hooks.

## Notes

- OCP: each scheme is its own crate; the registry maps scheme ID → factory from manifest data (01).
- Demotion of lying locators (BLAKE3 mismatch) is policy here, verification is 03's job.

## Acceptance

- File locator passes all read vectors via mmap; mirror fallback vector (primary file missing) reads through secondary without caller change.

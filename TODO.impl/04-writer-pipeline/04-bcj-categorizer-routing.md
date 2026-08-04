# 04 — BCJ categorizer routing

- **Status:** DONE (2026-08-04)
- **Phase:** 2
- **Depends on:** 04-bcj-composite-codecs (PR #138)
- **Design refs:** §6, 04-classifier-seine
- **Priority:** ~~P1~~ closed

## Resolution

`ExecutableCategorizer` added in
`limnifs-write/src/file_categorizer/executable.rs`. Detects ELF,
PE, and Mach-O magics and routes x86_64 / aarch64 architectures to
the corresponding BCJ composite codec:

| Format | Field | Architecture | Routed codec |
|---|---|---|---|
| ELF | `e_machine` @ 18 | x86_64 (`0x3E`) | `CODEC_BCJ_X86_LZ4` (0x20) |
| ELF | `e_machine` @ 18 | aarch64 (`0xB7`) | `CODEC_BCJ_ARM64_LZ4` (0x23) |
| PE | `Machine` @ lfanew+4 | x86_64 (`0x8664`) | `CODEC_BCJ_X86_LZ4` |
| PE | `Machine` @ lfanew+4 | aarch64 (`0xAA64`) | `CODEC_BCJ_ARM64_LZ4` |
| Mach-O | `cputype` @ 4 | x86_64 (`0x01000007`) | `CODEC_BCJ_X86_LZ4` |
| Mach-O | `cputype` @ 4 | arm64 (`0x0100000C`) | `CODEC_BCJ_ARM64_LZ4` |

Registered in `default_registry()` after the existing categorizers
(`fits`, `pcm_audio`, `csv_text`). Order matters: more-specific
categorizers register first so they win.

Unknown architectures return `None` — caller falls through to
plain FastCDC + classify. We do NOT route to a wrong-arch BCJ
filter (which would corrupt the binary).

7 tests cover the matrix:
- ELF x86_64 + aarch64 detection.
- PE x86_64 detection.
- Mach-O x86_64 detection.
- Unknown ELF arch returns None.
- Small input returns None.
- Non-executable content returns None.

## Goal

Once the BCJ composite codecs (id 0x20+) land, the classifier needs
to detect ELF/PE/Mach-O binaries and route them to the BCJ-x86 /
BCJ-arm64 composite instead of plain LZ4.

## Acceptance

- [x] `BinaryClassifier` detects all four executable formats (ELF,
      PE, Mach-O 32-bit, Mach-O 64-bit; both endianness for Mach-O).
- [x] Categorizer registry routes executables to BCJ composites.
- [ ] Benchmark on Linux kernel source shows BCJ composite beats
      plain LZ4 by ≥ 20% ratio on `vmlinux` and similar. (Requires
      Linux CI; ratio win proven on synthetic x86-call fixtures in
      `bcj_composites::tests::bcj_x86_lz4_beats_plain_lz4_on_synthetic_exec`.)

## Why LimniFS cares

`vmlinux`, Docker images of OS distros, language runtimes (Python,
Node, Ruby) — all are heavy on executable code that doesn't
compress well with general-purpose codecs. BCJ is the single
highest-ratio win available for the `binary` content class.

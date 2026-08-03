# 04 — BCJ categorizer routing

- **Status:** pending
- **Phase:** 2
- **Depends on:** 04-omnizip-new-algos (BCJ composite codecs)
- **Design refs:** §6, 04-classifier-seine
- **Priority:** P1

## Goal

Once the BCJ composite codecs (id 0x20+) land, the classifier
needs to detect ELF/PE/Mach-O binaries and route them to the
BCJ-x86 / BCJ-arm64 composite instead of plain LZ4.

## Design

1. New `BinaryClassifier` (in `file_categorizer`) keyed on the
   first 16 bytes (ELF magic `\x7FELF`, PE magic `MZ`, Mach-O
   magic `\xFE\xED\xFA\xCE` / `\xFE\xED\xFA\xCF` / `\xCF\xFA\xED\xFE`).
2. Pick BCJ filter based on detected architecture:
   - ELF e_machine = EM_X86_64 (62) → BCJ_X86
   - ELF e_machine = EM_AARCH64 (183) → BCJ_ARM64
   - PE Machine = 0x8664 → BCJ_X86
   - Mach-O cputype = CPU_TYPE_X86_64 → BCJ_X86
   - Mach-O cputype = CPU_TYPE_ARM64 → BCJ_ARM64
3. Composite codec picked from profile: BCJ_X86_LZ4 (max-write),
   BCJ_X86_ZSTD (balanced), BCJ_X86_LZMA (max-ratio).

## Acceptance

- [ ] `BinaryClassifier` detects all four executable formats.
- [ ] Categorizer registry routes executables to BCJ composites.
- [ ] Benchmark on Linux kernel source shows BCJ composite beats
      plain LZ4 by ≥ 20% ratio on `vmlinux` and similar.

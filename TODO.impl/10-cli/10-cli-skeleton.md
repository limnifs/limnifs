# 10 — CLI skeleton

- **Status:** done — limni/src/main.rs (clap, 25+ subcommands)
- **Phase:** 1
- **Depends on:** 03-core-reader
- **Design refs:** §3 (vocabulary), §15

## Goal

`limni` binary: clap-based subcommand framework, `--json` everywhere, stable
exit codes, version/feature reporting from the registry data (01).

## Notes

- UX teaches the model: help text uses drop/slab/tier vocabulary.
- No logic: every subcommand is a stub calling into crates (logic pushed down rule from component README).

## Acceptance

- `--help` tree complete for all planned subcommands; `--json` schema documented; shell completions generated.

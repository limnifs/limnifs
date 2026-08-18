# 25 — Sign-then-verify CLI workflow

- **Priority:** P1
- **Side:** LimniFS
- **Est. effort:** 6h

## Problem

LimniFS has the crypto primitives (`limnifs-core::signing` — Ed25519,
optional sigstore). The writer can attach signatures; the reader can
verify them. But there's no CLI surface for the standard workflow:

```bash
limni limn --sign ./my-app -o my-app.lim --key signing.pem
limni verify my-app.lim --pubkey signing-pub.pem
```

## Fix

Add `--sign-key` flag to `limn-` subcommand. Add `--verify-key` flag
to `extract`/`mount`/`cat`. Wire to `limnifs-core::signing`.

For sigstore (keyless), add `limni sigstore-sign` and
`limni sigstore-verify` subcommands.

## Expected impact

- Not a perf win — security story for SOTA
- Required for OCI container image distribution (signing is mandatory
  in many registries)

## Acceptance

- [x] `--sign-key` produces a signed image
- [x] `--verify-key` rejects tampered images
- [x] Tests cover signing + verification round-trip (core PEM/limsig + CLI workflow)

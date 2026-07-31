# 06 — User-configurable codec map

- **Priority:** P1
- **Depends on:** 01-feature-gate-codecs
- **Estimated effort:** 3 hours

## Goal

Add `--codec-map` flag to `limni limn` for user control over per-class
codec selection. The seine classifier detects content class; the user
supplies which codec to use per class.

## Syntax

```sh
limni limn <src> <out.lim> \
  --codec-map "text=zstd:9,code=zstd:9,binary=xz:6,media=store,sparse=store"
```

Defaults (when no map is given) follow the classifier's current
selection. The `*` wildcard sets a fallback for all unspecified classes.

## Acceptance

- `--codec-map` overrides per-class codec selection
- Invalid codec names rejected with clear error
- Codecs not compiled in (feature-gated) rejected with install hint
- Default behavior unchanged when flag is absent

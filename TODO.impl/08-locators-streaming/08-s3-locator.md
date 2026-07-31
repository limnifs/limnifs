# 08 — S3 locator

- **Status:** done — limnifs-core/src/s3_locator.rs (path-style, anon buckets for v1)
- **Phase:** 2
- **Depends on:** 08-http-range-streaming
- **Design refs:** §10.1, §12 (cloud sharding/assembly)

## Goal

`s3:` locator: range GETs, multipart upload for the writer path, credential
chain per AWS SDK conventions, cross-region mirror entries.

## Notes

- Cloud assembly = manifest referencing slabs across buckets/regions; this locator plus the registry makes that configuration, not code.
- Writer push path reuses 04's `Sink` trait.

## Acceptance

- MinIO-based integration vectors: round-trip write/read, cross-"region" mirror failover, range-streaming budget met.

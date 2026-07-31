# 08 — HTTP range streaming

- **Status:** done — limnifs-core/src/http_locator.rs (hand-rolled HTTP/1.1, behind http feature)
- **Phase:** 2
- **Depends on:** 08-locator-trait-registry
- **Design refs:** §10.1 (streaming-native), §6 (read path), §1.1 (DwarFS read amplification)

## Goal

`http(s):` locator: range requests, slab-index-guided read-ahead, hedged
requests across mirrors, ETag/caching respect.

## Notes

- ≤ 2 RTTs to first byte after manifest open (budget); read-ahead window adaptive to observed bandwidth.
- Plain static HTTP hosting must suffice — no server-side cooperation (CI artifact use case, design §12).

## Acceptance

- Streaming vectors over a local test server: byte-exact reads, amplification within budget, mirror-failover mid-stream vector passes.

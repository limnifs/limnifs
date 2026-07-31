# 10 — Policy-attached operations

- **Priority:** P1
- **Depends on:** 02-epoch-format
- **Estimated effort:** 1 day

## Goal

Each operation in an epoch can carry a policy constraint that future
epochs MUST respect. Policies are part of the Merkle chain — violating
them produces a detectable inconsistency.

## Policy types

| Policy | Effect |
|---|---|
| Immutable | No future epoch can modify this path |
| AppendOnly | Files can be added under this path but not removed or modified |
| Quorum(N) | Modifications require N independent signatures |
| Retention(until_date) | Files cannot be removed until the specified date |
| LegalHold(authority) | Frozen until a corresponding Release from the same authority |

## Syntax in epoch

```
Add("/etc/shadow", drop_id, policy=Immutable)
Mkdir("/var/log/audit", policy=AppendOnly)
```

## Acceptance

- Policy checker rejects epochs that violate existing policies
- Policies survive flatten/turnover
- LegalHold cannot be overridden without the corresponding Release

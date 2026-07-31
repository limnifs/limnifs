# tiered storage

- **Priority:** P3
- **Depends on:** 02
- **Estimated effort:** see detail

## Goal

Tiered epoch storage.

## Detail

Hot drops → NVMe, warm → SSD, cold → HDD, archive → cloud. Epoch system tracks access frequency per DropId. Transparent promotion/demotion.

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green

# gdpr forget

- **Priority:** P3
- **Depends on:** 02,10,11
- **Estimated effort:** see detail

## Goal

GDPR right-to-be-forgotten.

## Detail

Forget(subject_id) removes files from future epochs. Past sealed epochs retain data (SEC compliance). Chain documents the deletion request.

## Acceptance

- Spec written and implemented
- Feature-gated if external dependencies required
- Air-gapped baseline unaffected
- CI green

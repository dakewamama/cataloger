# Continuity

## Completed

- Existing GitHub cataloger repository is the renamed catalyst-indexer checkout.
- Static resolver preserves cluster genesis identity, native program key,
  deployment/version, schema, adapter reference and provenance.
- Boundary correction removes the ARM dependency, capability declarations and ARM
  context conversion. Runtime joins Cataloger with Catalyst.
- supported_from_slot and supported_until_slot_exclusive bound verified interpretation,
  not deployment lifetime. No live deployment records or workspace split were added.
- Legacy webhook binary and database remain unchanged.

## Test status

- Correction: exact workflow commands pass locally: formatting, strict Clippy,
  7 resolver tests and 23 retained runtime tests. Cargo.lock contains no ARM dependency.
- Historical GitHub main runs for 0040ee5, 0d33570 and d60d492 failed. Local success did
  not establish CI success. Latest failed job has no steps: GitHub annotation says
  the account is locked due to a billing issue. Hosted CI is EXTERNALLY BLOCKED.
  This external lock does not invalidate exact local workflow evidence.

## Runtime backlog

- Slot is observed but not persisted; event identity lacks instruction position.
- Idempotency uses signature/discriminator/raw bytes and merges identical distinct events.
- Native u64 amounts narrow to i64; middleware tests do not exercise middleware.
- No replay, coverage or finality; broadcast is lossy.
- Subscriptions event decoding is hard-coded and unversioned.
- Fix these at the runtime milestone, not in the resolver correction.

## Real blocker

- The owner must resolve the GitHub billing lock before hosted Actions can start.
  No workflow or lint weakening can repair that account condition.

## Next critical path

- Boundary correction pushed at a88dbd8; final local workflow recheck passes all 30 tests.
- Owner explicitly approved progression with hosted CI EXTERNALLY BLOCKED. Railway is
  not a CI substitute. Subscriptions maintainer study is unlocked after final SDK
  Freeze/Thaw Direct correction and local SUB-0 review. No adapter implementation yet.
- Architecture book remains unlocated; constitution supplies current requirements.

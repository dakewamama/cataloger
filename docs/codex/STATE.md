# Continuity

## Completed

- Actual GitHub mapping: catalyst-crank -> arm; catalyst-indexer -> cataloger.
  The old catalyst-indexer GitHub URL redirects to cataloger as well; there is no
  distinct indexer among these checkouts.
- ARM root library pushed: 5b05e06892dcc6d20d1db7f0324030916b152d6a.
- Catalyst semantic ABI pushed: d803c44 (main).
- Static Cataloger library resolves bounded deployment history with schema, adapter,
  supported ARM capabilities and provenance. Unknown slots/versions fail closed.
- Existing webhook binary retained as catalyst-indexer; no on-chain source changed.

## Test status

- Cataloger: 7 resolver tests plus all 23 legacy tests pass; strict Clippy passes.
- ARM: 12 tests plus fmt/Clippy pass.
- SDK: 6 Rust ABI tests, 3 Bun tests and TypeScript check pass; fmt/Clippy pass.
- Known legacy authentication tests do not actually exercise middleware; amounts
  still use unchecked u64 -> i64 conversion. These are runtime issues, not resolver claims.

## Verified upstream

- GitHub repository names and remote redirects verified, identities unchanged.
- No SPL or Token-2022 deployment has been entered into the catalog yet.
- No Subscriptions source study; SUB-0 locked.
- Cataloger now uses solana-pubkey 4.2.0, the native address type compiled with
  official token interfaces and Mollusk in the SDK compatibility proof.
- This dependency change adds no deployment records or new resolution claims.

## Real blockers

- None for this audit. Architecture book not located; constitution supplies current requirements.

## Next critical path

- Resolver milestone pushed at 0040ee5; next official token interface compatibility,
  golden state fixtures and revoke round trip.
- Record verified deployment coverage before advertising a live supported version.
- Do not create a separate repository to compensate for historical names.

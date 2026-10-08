# Observation sources

## Agave RPC client

- Repository: https://github.com/anza-xyz/agave
- Revision: `c9c6f3287e26f24e3476e13e751aebe710191e89`, published client `4.2.2`.
- License: Apache-2.0. Inspection: TARGETED SOURCE, followed by local conformance
  tests and public finalized RPC capture.
- Relevant source: `rpc-client/src/nonblocking/rpc_client.rs`,
  `rpc/src/rpc.rs::get_multiple_accounts`, account decoder client types and manifests.
- One `getMultipleAccounts` selects one bank and returns its context slot. The
  unsliced Base64 response retains missing accounts explicitly. Separate requests
  cannot be assembled into evidence for one bank; `minContextSlot` is only a lower bound.
- Reuse: DEPEND / PUBLIC API. Transport, typed responses and account decoding remain
  upstream. No implementation source copied.
- Locked `five8_core` at 1.0.0: its allowed broad range otherwise reused 0.1.2,
  whose decode error lacks the Error implementation required by `solana-keypair`.
  This reproduces the client's published compatible lockfile without a new dependency.

## Carbon

- Repository: https://github.com/sevenlabs-hq/carbon
- Revision: `a64db3cfb0bb2e500ba6d2a355cd4d7da96890cd`, workspace `2.0.0`.
- License: MIT. Inspection: TARGETED SOURCE of RPC GPA, validator snapshot and
  Yellowstone datasource implementations and manifests.
- GPA preserves the returned context slot, but separate owner scans cannot include
  token state, Clock and loader state in one atomic observation. Archive snapshots
  provide a bank while adding substantial runtime dependencies.
- Reuse: PATTERN ONLY for this bounded snapshot milestone. Streaming remains a
  later integration question. No code copied.

## Yellowstone Vixen / Shipstern

- Repository: https://github.com/solana-rpc/shipstern (Vixen's current redirect).
- Revision: `3cd623a461817e13ed8ab4a56404247b002d87c3`, workspace `0.11.0`.
- License: MIT. Inspection: TARGETED SOURCE of Solana RPC, snapshot and Yellowstone
  source implementations, manifests and reconnect handling.
- RPC source reads `getSlot` separately from GPA and stamps that earlier slot onto
  updates. It cannot provide this milestone's atomic account/Clock observation.
  Stream replay depends on server support; reconnect alone cannot establish coverage.
- Reuse: REJECT for bounded bank collection; streaming evaluation remains open.
  No code copied.

## Loader and Clock interfaces

- Repository: https://github.com/anza-xyz/solana-sdk
- Loader: `solana-loader-v3-interface` 7.0.0,
  revision `182207e7bc89c0f961696b37651c8bf55957779f`, `loader-v3-interface/`.
  Clock: 4.0.0, revision `5dedf83cf2789b9451289a60e85ba923a51ea71d`, `clock/`.
- License: Apache-2.0. Inspection: TARGETED SOURCE of state layouts, size tests,
  PDA helper and Clock serialization.
- Reuse: DEPEND. Native types own metadata layouts and derivation. Full program
  payload hashing deliberately rejects unverified padding/layout variants.
  Loader-v3 activates deployed code in the following slot; see
  [official deployment documentation](https://solana.com/docs/core/programs/program-deployment).
- Finality and account authenticity are RPC trust assumptions. A response supplies
  neither a bank hash nor a cryptographic state proof. No source copied.

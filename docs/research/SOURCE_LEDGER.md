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
- Simulation: TARGETED SOURCE of the pinned client's
  `simulate_transaction_with_config`, published transaction 4.1.6 serialization,
  and [Agave v3.1.8 native RPC implementation](https://github.com/anza-xyz/agave/blob/v3.1.8/rpc/src/rpc.rs#L3692).
  `minContextSlot` selects a lower bound, not an exact bank. Returned account count
  cannot exceed message account count; addresses may include bank accounts outside
  the message. Failed execution returns null entries. Reuse: DEPEND / PUBLIC API.
  The bounded SPL path requests source, owner and Clock, verifies the same finalized
  bank and retains native output separately from finalized observations. No source copied.
- Agave `v3.1.10` (`7bc9c805218ca06769956e2cb61601329f5a0f6c`), Apache-2.0:
  TARGETED SOURCE of
  `program-runtime/src/loaded_programs.rs::{extract,finish_cooperative_loading_task}`,
  `svm/src/transaction_processor.rs::replenish_program_cache`, and test-validator
  JSON account import, followed by EXECUTED LOCALLY through its actual RPC server.
  `spl-revoke-rpc.json` retains the successful finalized local response and input
  envelopes. Code identity is unchanged from the captured SPL payload.
  A deployment skipped by a local warp can fail cache ancestry/insertion checks;
  `ProgramCacheHitMaxLimit` alone does not prove capacity exhaustion. Warping to the
  deployment and observing a finalized descendant resolved the fixture failure.
  Reuse: PUBLIC API / PATTERN ONLY. No implementation source copied.

## SPL simulation fixture

- Mollusk `0.15.1`, source `f432ef136ee9779d2a814ebf2b80f44c10607255`, Apache-2.0.
  TARGETED SOURCE and EXECUTED LOCALLY through `process_transaction_instructions`
  with the maintained token program fixture. Reuse: DEPEND / PUBLIC API.
- `spl-delegate.bin` becomes `spl-revoke-after.bin` by executing canonical native
  Revoke, not by editing account bytes. Complete ELF hash `8190d3f7...` matches the
  captured classic SPL payload. The runtime transport tests combine this native
  transition with retained code/Clock evidence explicitly as a fixture; it is not
  an observed live delegate. Malformed cases are explicit test mutations.
  No upstream implementation code or executable copied into the runtime.

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

## SQLite checkpoint index

- Existing dependency: SQLx 0.7.4 and its locked SQLite library. Inspection:
  TARGETED SOURCE of SQLx migrations, plus native journal integration tests.
- [SQLite comparison rules](https://www.sqlite.org/datatype3.html#sort_order)
  order fixed-width BLOBs bytewise. Big-endian slot bytes preserve unsigned
  positions; an index over immutable rows avoids a separate mutable head table.
- Reuse: DEPEND / PUBLIC API. No source copied.

## Subscriptions deployment provenance

- Source: https://github.com/solana-foundation/subscriptions,
  revision `56de552a26a0f0af437c0ce5191b3309741cc596`, MIT.
  [Official devnet run](https://github.com/solana-foundation/subscriptions/actions/runs/37008589067)
  built that source with solana-verify 0.5.2, Agave 3.1.10 and platform-tools v1.52.
  TARGETED SOURCE of the pinned release workflow; official build log inspected.
- SDK `0bfb3f57c2b4b9c22cf41d582573e8e60352be05` records the locked native
  rebuild and executable conformance proof. Reuse: PUBLIC API for semantic
  dispatch; no upstream implementation source copied into the runtime.
- Public devnet finalized capture at slot `509022453` retains both programs,
  their ProgramData and Clock in one unsliced response. Raw response SHA256:
  `794c6c9fbb35697eabf1931fb42440cca8b8a8dc2a42de95fb95b1a945651f53`.
  Retained under `services/catalyst-indexer/tests/fixtures/` and EXECUTED LOCALLY
  through journal dispatch/replay and rejection tests.
- Subscriptions deploy `506642674` full payload hash `2675ad1d...` differs
  from the older local fixture. Its verification hash `e705f5a3...` matches
  the official build and local rebuild. Devnet SPL deploy `451008000` retains
  the tested complete `8190d3f7...` payload. Runtime checks both complete hashes
  and exact deployment selectors; verified-build normalization is not substituted.
- Experience: a shared source/client version need not produce identical executables
  across toolchains. A reproducible source link establishes code identity, while
  current grant interpretation still needs native account and wallet evidence.
  Missing grant state remains incomplete. No open-ended coverage is inferred.

## SBPF execution identity

- Research: [OpenZeppelin's relocation-oriented programming analysis](https://www.openzeppelin.com/news/relocation-oriented-programming-on-solana), published May 27, 2026. It demonstrates that v0-v2 relocation processing can overwrite executable bytes and read-only data, including bytes outside normal instruction operands. Its proof of concept targets sbpf crate 0.14.2. Inspection: full article and linked loader paths.
- Loader: [Agave 3.1.10](https://github.com/anza-xyz/agave/tree/7bc9c805218ca06769956e2cb61601329f5a0f6c), commit `7bc9c805218ca06769956e2cb61601329f5a0f6c`, Apache-2.0. Relevant paths: `programs/bpf_loader/src/lib.rs`, `program-runtime/src/loaded_programs.rs`, `syscalls/src/lib.rs`, `platform-tools-sdk/cargo-build-sbf/src/toolchain.rs`, `svm-feature-set/src/lib.rs`. Inspection: TARGETED SOURCE. The loader builds its runtime environment, loads and relocates ELF, runs `RequisiteVerifier`, then makes the program available for execution.
- SBPF parser/VM: [anza-xyz/sbpf](https://github.com/anza-xyz/sbpf), crate `solana-sbpf` 0.13.1, source revision `acd2c551a0f8df2a8f077a6e4b55f546d4deb98d`, selected by Agave 3.1.10. License: MIT OR Apache-2.0. Relevant paths: `src/elf.rs`, `src/elf_parser/mod.rs`, `src/program.rs`, `src/verifier.rs`. Inspection: TARGETED SOURCE plus executed loader against the captured ELF. Reuse: DEPEND on this exact version. No source copied.
- Runtime environment: Agave crates `agave-syscalls`, `solana-program-runtime` and `solana-svm-feature-set` 3.1.10. The local inspection command uses Agave's environment factory and `SVMFeatureSet::all_enabled`; this is a reproducible inspection profile, not evidence of the validator's feature set at the captured bank. Runtime identity remains absent until separately attested.
- Static syscalls: [SIMD-0178](https://github.com/solana-foundation/solana-improvement-documents/blob/64dff76/proposals/0178-static-syscalls.md) requires static syscall encoding for SBPF v3 and removes dynamic call relocations. SIMD-0189 specifies stricter v3 ELF parsing. The captured Subscriptions ELF flags are zero, so these v3 rules do not apply. Reuse: SPEC for version distinction, official Agave loader for implementation. No source copied.
- Reproducible build: the official Subscriptions Actions run `37008589067` used `solana-verify` 0.5.2, Agave 3.1.10 and platform-tools v1.52. The full padded ProgramData payload is 133280 bytes with SHA256 `2675ad1d2b5068d47fc5d169156cf4859a9c21c0406ce63e3828e3b7320fddbf`. `solana-verify` normalization (trim trailing zeroes) hashes to `e705f5a309f84f849b402f20de4bea5f2cc1d1d4f691ba7caabcb07c8b46af51`, matching the official run and locked local rebuild.
- Trust boundary: the local loader image is deterministic for its pinned crate and explicit profile. The finalized RPC capture does not attest Agave's active feature set or exact deployment environment. The adapter therefore treats missing runtime evidence as insufficient for live semantic support. The reported image fingerprint is not a JIT machine-code hash and makes no claim about validator execution without a matching runtime witness. No code copied.

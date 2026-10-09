# Native snapshot evidence

Extracted via `git show` from `tests/fixtures/` in the `catalyst-sdk` repository
at commit `b3682df29b3b6c64e9fcf96b35a6dd23fa552759`.
Source: https://github.com/dakewamama/catalyst-sdk/tree/b3682df29b3b6c64e9fcf96b35a6dd23fa552759/tests/fixtures

| Source fixture | SHA-256 (whole original file) |
| --- | --- |
| `spl-delegate.bin` | `f5cfe08fa28ea703b5895b36d519811a0ff7f55b5728e4e550741a882bbb66f5` |
| `spl-delegate.json` | `545da5bed8dc6c41a581fe9c0ef91436fa9bad6551829dfebcd6e5af159a10fb` |
| `subscriptions-lifecycle.json` | `cda5667e1273599ebbee906b293dba2ae1f8a6f99626be961a40172a759f4b23` |
| `subscriptions-plan-arm.json` | `f039cd349d944b9a05851cd9210337ea3893f0db2c95cab77e3272c4b62d80cf` |

Catalog adapter-source URLs identify the executed SDK implementation at
`54ed06aa3a7c8a31268dface42d0bf27ca4adaee`. Fixture provenance remains at
`b3682df29b3b6c64e9fcf96b35a6dd23fa552759`; the raw accounts are unchanged.

`spl-delegate.bin`, `spl-delegate.json`, and `subscriptions-plan-arm.json` are
byte-for-byte copies. `subscriptions-snapshots.json` retains source and client
revisions, ELF hashes, full clocks, transition names/positions/outcomes, and filenames. The importable
`owner-pull-60.json` contains ten whole raw accounts from `owner_pull_60:after`;
`revoke-cancelled-before.json` and `revoke-cancelled-after.json` contain eight
whole raw accounts each from the corresponding `revoke_cancelled` phases.
Selection: subscriber, merchant, puller, subscription authority, plan 0,
subscription delegation for plan 0, mint, and subscriber token account. The
slot-108 capture also includes the plan-1 and delegation-1 accounts to exercise
two targets sharing one technical authority.
No bytes within selected account records are changed; omitted accounts concern
destinations, execution-only accounts, and the unselected plan-1 state at slot
122. `rent_epoch` is not recorded upstream and decodes to the native Account
default, zero.

The SPL account envelope and owner/delegate witnesses follow the pinned SDK's
`tests/spl.rs::state`: token lamports 10,000,000, owner lamports 1,000,000,
default delegate, token program owner, source `[1;32]`; native Pack supplies the
owner and delegate addresses. SPL slot 1 follows its golden; timestamp 0 is a
local test label because that fixture records no clock. Subscription clocks are
unaltered (slot 108/time 1800000000 and slot 122/time 1800003600).

Observations use `local:mollusk-fixtures` / `mollusk:fixture-bank` and Fixture
origin. This evidence establishes local native behavior, not live deployment
coverage. `catalog.json` records are manually bounded to [1,123) for SPL and
[100,123) for subscriptions, use the pinned SDK's exact version/deployment constants,
SPL adapter 0.1 and subscriptions ADAPTER_VERSION, and cite the source fixture
hashes. Native schemas are `spl-token-interface` 3.0.0 and `subscriptions` 0.5.0;
the latter client source is `subscriptions:0.5.0:5a347ffaa969036061d274d3c91e0277962e2b51` and native source
revision `56de552a26a0f0af437c0ce5191b3309741cc596`. No upstream Rust
implementation/test code or ELF is copied. The max-u64 fields used by tests are
explicit test mutations, not native evidence.

Snapshot files use the runtime's native serde representation: pubkeys and data
are byte arrays, accounts are address/account pairs, and `rentEpoch` is zero
where the upstream fixture omitted it. Tests consume these exact files and
`catalog.json`. `spl-snapshot.json` wraps the unaltered SPL binary with the SDK
account envelope above. Both subscription targets at slot 108 can be selected
using the two plan/delegation pairs present in `owner-pull-60.json`.

## Finalized mainnet capture

`mainnet-spl-response.json` is the complete, unmodified JSON-RPC response from
`getMultipleAccounts` against `https://api.mainnet-beta.solana.com` on October 8,
2026, with finalized commitment and unsliced Base64 data. SHA256:
`68ffbb2b01685f3bdadf174780681fb4dac9355b58282a09d145ff8f5acd1b0e`.
`mainnet-spl-capture.json` records the genesis hash and exact request address order.
Tests remap values into the collector's sorted request order without changing native bytes.

Context slot is `454547887`. The Clock account comes from this same response.
SPL ProgramData `3gvYRKWyXRR9xKWe1ZjPhLY5ZJRN7KDB4rFZFGoJfFk2` records deployment
slot `419472000` and no upgrade authority. The complete 108600-byte payload has
SHA256 `8190d3f7ceb6cb7a7a8d8924bff89f9f611e15ce1f806f2b6237f3311a98f697`, exactly
the tested Mollusk ELF. The test catalog supports only the observed slot; it does
not claim historical deployment coverage or future support.

Source `HwD4QpS4bsutLbWZWhbmFUZfkXC5Au1DbkzYEzjDgps8` returns native RPC `null`.
This is an actual absence observation, not a fabricated live token/delegate grant.
Mutation cases explicitly alter this capture to test rejection of changed code,
loader metadata, malformed responses and missing evidence. Native delegate/revoke
behavior remains proven by the SDK's separate executable fixtures.

## Finalized devnet deployment capture

`devnet-subscriptions-response.json` is the complete unmodified public devnet
`getMultipleAccounts` response captured October 9, 2026. It retains Subscriptions,
SPL, both canonical ProgramData accounts and Clock from finalized slot `509022453`.
The metadata file records the exact address order, genesis, request configuration
and response SHA256 `794c6c9fbb35697eabf1931fb42440cca8b8a8dc2a42de95fb95b1a945651f53`.
These are program observations; no live grant or token account is fabricated.

Subscriptions deployment slot `506642674` has full payload SHA256
`2675ad1d2b5068d47fc5d169156cf4859a9c21c0406ce63e3828e3b7320fddbf`.
Its trailing-zero-normalized executable hash matches the official deployment
job and the local pinned source/toolchain rebuild. The runtime checks the full
payload, including allocation padding. Devnet SPL deployment slot `451008000`
has the same full payload as the tested classic token ELF.

Tests select explicitly unobserved target accounts to prove that verified code
does not establish a grant: dispatch reaches missing-state handling and stores
Incomplete. Changed payloads fail before native grant decoding; missing ProgramData
or Clock remains incomplete. Catalog intervals cover only the captured slot.
Native authority/control behavior is separately executed in SDK `0bfb3f5`.

## Native revoke output

`spl-revoke-after.bin` is the full 165-byte account data returned by canonical
SPL Revoke executed with Mollusk 0.15.1's atomic transaction API. Input data is
the unchanged `spl-delegate.bin` above; source is `[1;32]`, owner `[4;32]`,
with account envelopes matching `spl-snapshot.json`. No output bytes were edited.
SHA256 is `e1e671510fa14513936cbe2a11505c6c53504fd91422ae7a1249da5aebb0d129`.
The executable's full SHA256 is `8190d3f7ceb6cb7a7a8d8924bff89f9f611e15ce1f806f2b6237f3311a98f697`.
Reproduction: the SDK's `tests/spl.rs::native_revoke_round_trip` at `54ed06a`
uses this same input and instruction through `process_transaction_instructions`.

Simulation transport tests join native fixture accounts to the captured mainnet
program/ProgramData and Clock to exercise validation. This combination is an
explicit test fixture, not a coherent live capture or a live delegate claim.
RPC envelopes, clocks and failure cases are deliberate test constructions.
The loopback CLI test inspects the actual encoded unsigned transaction/configuration
and verifies that simulation leaves finalized journal state unchanged.

`spl-revoke-rpc.json` retains decoded input envelopes and the native response from
Agave 3.1.10 at finalized local slot `419472002`, genesis
`goeVyofiTSYtn1D32hdxfDbeBQvW5Poa4pNhGncUR9E`. Fixture SHA256:
`f594da805e10eabd726d9cb4d48e1a2f8e87514fcb4b005cbd0392926b02c39f`.
Only ProgramData's data is omitted to avoid duplicating the existing full capture;
the test restores it unchanged from `mainnet-spl-response.json`. All envelopes,
Clock and simulated output remain as decoded by the maintained RPC client.
The owner envelope includes the local validator's genesis funding and simulated fee.
These are local fixture accounts, not a public cluster grant or a signed transaction.

Reproduction: import the captured SPL program and ProgramData with the native
source/owner envelopes using `solana-test-validator --account-dir`, warp to the
ProgramData deployment slot `419472000`, then wait for a finalized descendant.
Observe the singleton SPL scope with a catalog interval covering that bank, and run
`authorization-api simulate-revoke ID SOURCE` before the finalized bank advances.
The returned token data matches `spl-revoke-after.bin`; reading finalized state again
still returns `spl-delegate.bin`. The journal checkpoint stays unchanged.
Warping past the imported deployment skipped its local ancestry and produced a
program-cache insertion failure before execution. Starting at the deployment slot
resolved it without editing the imported program bytes or changing validation.

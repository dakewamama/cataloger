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
`f6f47781e53a9995a15882e626adea1fcdd69d44`. Fixture provenance remains at
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

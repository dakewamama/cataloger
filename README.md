# cataloger

Cataloger identifies which deployment, schema and adapter interpretation is verified
at a slot. Catalyst owns authorization semantics. The runtime joins the resolver's
record with Catalyst and constructs the ARM context; Cataloger does not depend on ARM
or declare which capabilities an adapter emits.

The workspace keeps the resolver in `crates/cataloger` and the retained webhook
service in `services/catalyst-indexer`. The resolver uses Solana address types and
serialization; HTTP, database and runtime dependencies belong to the service.
The GitHub repository was renamed from catalyst-indexer, and the old URL redirects here.

## Static deployment resolver

Catalog::new accepts manually verified ProgramVersion records. Each binds a cluster
genesis hash and program key to deployment/version, schema source/revision, adapter
protocol/version/source and provenance evidence. Cataloger does not discover, crawl
or decode program state.

Support is bounded by the half-open interval
[supported_from_slot, supported_until_slot_exclusive). The exclusive upper bound is a
verification boundary, not evidence that the native deployment ended or upgraded.
Unknown versions, unsupported slots, gaps and mismatched selectors return
UnsupportedVersion. There is no open-ended latest-version assumption.

Construction rejects malformed records and overlapping support intervals. Record order
does not affect resolution. resolve checks cluster, program, slot and an optional exact
version selector, returning the original record with its provenance. Callers authenticate
records and their evidence; nonempty fields establish structural validity alone.

Seven synthetic resolver tests cover support boundaries, gaps, version rejection,
ambiguous intervals, independent cluster/program ranges, determinism and provenance.
These test the resolver algorithm. The runtime also checks a real finalized mainnet
SPL observation against its complete program bytes; support remains bounded to
verified records and every live projection revalidates its deployment evidence.

```sh
cargo fmt --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
git diff --check
```

## Snapshot authorization API

`authorization-api` imports one bank's observed accounts, resolves verified program
versions through Cataloger and compiles them through Catalyst into ARM. Supported
targets are classic SPL delegates and fixed, recurring or plan-backed Subscriptions
delegations. An explicitly observed closed SPL account compiles to no delegate;
an omitted account remains insufficient evidence. The service uses the official
token account decoder and native Solana account serialization.

Each immutable journal record retains raw accounts, cluster/bank identity, slot,
clock, target scope, resolved schema/adapter provenance, SDK revision and projection.
SQLite stores the record as JSON text, preserving native `u64` values without a
signed integer cast. Account and target ordering does not affect its content hash.
Duplicate ingestion is idempotent; a conflicting interpretation cannot overwrite
the record. Replay uses the retained resolver records and must reproduce the
original projection. Record checksums are verified before reads. Unknown versions,
missing evidence and malformed native state remain distinct stored outcomes.

```sh
export DATABASE_URL=sqlite:///absolute/path/authorizations.db
cargo run -p catalyst-indexer --bin authorization-api -- import CATALOG.json SNAPSHOT.json
cargo run -p catalyst-indexer --bin authorization-api -- observe CATALOG.json SCOPE.json
cargo run -p catalyst-indexer --bin authorization-api -- replay SNAPSHOT_ID
cargo run -p catalyst-indexer --bin authorization-api -- serve 127.0.0.1:3001
```

The catalog file is an array of manually verified `ProgramVersion` records. Its
publisher must authenticate deployment evidence; structural validation alone does
not establish provenance. The snapshot supplies all accounts from one coherent
bank, its clock and the targets to interpret. A removed account must be observed
explicitly; a missing account is insufficient evidence.

`observe` uses the official Agave RPC client and `RPC_URL`. Its scope supplies the
expected cluster genesis hash, target selectors, account witnesses and optional
`min_context_slot`. It adds each target's primary accounts, Clock, program and
ProgramData addresses, then reads the entire set with one finalized, unsliced
`getMultipleAccounts` request. A scope exceeding 100 accounts is rejected rather
than split across banks. Dependency discovery can precede this request; discovery
reads must not be mixed into its result. Missing witnesses remain incomplete.

An explicit RPC `null` becomes the native empty account; malformed encoded accounts,
short responses and inconsistent clocks fail collection. Live interpretation checks
loader ownership, executable state, canonical ProgramData linkage, active deployment
slot and the full payload hash against the catalog record. Fixture labels, unknown
code or unsupported slots cannot become supported live projections. Clock supplies
the bank's timestamp, not local wall time or a separate block-time request.

`GET /snapshots/{id}` returns retained evidence and its projection.
`GET /snapshots/{id}/authorizations/{address}` selects records that mention the
address as principal, subject or resource. Compound principals remain intact;
membership does not imply independent authority. Invalid addresses return 400,
unknown snapshots 404 and storage failures 500.

Responses expose the state hash, observation position, exact versions and incomplete
coverage. An empty result does not establish that an address has no authorization.
These are historical snapshots, not a merged current-state view. Native fixture
deployments cannot interpret finalized live observations. Classic SPL's observed
deployment matches the tested ELF; Subscriptions and Token-2022 live versions remain
unsupported. The retained mainnet capture proves a null source account and verified
SPL identity at one bank, not a live delegate grant or complete address coverage.
Continuous ingestion, checkpoints, repair and complete address coverage remain
outstanding. ARM amounts remain integer base units; clients must preserve 64-bit values.

Finalized commitment and account authenticity rely on the chosen RPC provider. The
`rpc:finalized:SLOT` bank label identifies its context, not a cryptographic bank hash
or state proof. Support intervals remain verification boundaries. Raw unsupported
observations are retained; a newly verified catalog requires a fresh observation
rather than overwriting an immutable historical interpretation. Replay requires
the recorded SDK revision; upgrading the runtime cannot silently reinterpret history.

## Existing observation service

The retained binary is the previous ingestion implementation described below. It does
not dispatch through Cataloger yet and must not be presented as a version-verified
semantic runtime. Its existing migrations and event storage remain available for the
runtime integration milestone. The legacy table is not a canonical observation journal. Slot is logged but not persisted;
instruction positions are not retained. Runtime migration must preserve both.

Event indexer for the [Solana Subscriptions Program](https://github.com/solana-foundation/subscriptions) (`De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44`).

Ingests Helius raw webhooks, decodes self-CPI event data from inner instructions, persists raw bytes alongside decoded fields, and broadcasts typed events to downstream consumers.

Rust, Axum, SQLx, SQLite, Tokio.

## Why

The Subscriptions Program emits six lifecycle events via Anchor-compatible self-CPI. It does not schedule pulls, retry failures, or react to anything. Merchants submit `transfer_subscription` themselves and handle everything after.

This is the ingestion layer for that "everything after". It answers what happened, to which plan, for which subscriber, when, and for how much.

## Events

| Disc | Event | Decoded fields persisted |
|---|---|---|
| 0 | SubscriptionCreated | plan, subscriber, mint |
| 1 | SubscriptionCancelled | plan, subscriber, expires_at_ts |
| 2 | SubscriptionTransfer | plan, delegator, mint, amount, period bounds |
| 3 | FixedTransfer | delegator, mint, amount |
| 4 | RecurringTransfer | delegator, mint, amount, period bounds |
| 5 | SubscriptionResumed | plan, subscriber |

All six store `raw_data` alongside the decoded columns, so re-decoding is possible without replaying from chain.

## Pipeline

```
Helius raw webhook
  -> auth middleware (Authorization header)
  -> deserialize Vec<RawTransaction>
  -> extractor: resolve programIdIndex against accountKeys
  -> filter by program ID, base58 decode, check EVENT_IX_TAG
  -> decoder: discriminator -> typed CatalystEvent
  -> persist raw + decoded (ON CONFLICT DO NOTHING)
  -> broadcast::Sender<CatalystEvent>
```

## Wire format

```
[0..8]   EVENT_IX_TAG   0x1d9acb512ea545e4  (Sha256("anchor:event")[..8], little-endian)
[8]      discriminator  0-5
[9..]    payload        fixed layout per event type
```

Offsets come from each event's `write_inner` in the Foundation's source, not from inference. `SubscriptionTransfer` and `RecurringTransfer` share an identical 192-byte payload layout.

## Bugs found and fixed

Each of these failed silently. The service returned 200 and logged success while dropping data.

### 1. The payload shape was wrong

The original `WebhookPayload` modeled `{ type, transactions[] }` with instructions carrying `programId` as a string.

Real Helius raw webhooks POST a top-level JSON array, and inner instructions reference `programIdIndex`, an index into `transaction.message.accountKeys`, not a resolved pubkey.

Every real webhook failed serde deserialization and returned 422 before reaching the handler. Helius counted them as delivered. The dashboard showed 36 events. The indexer logged nothing and stored nothing.

The test that should have caught this built its JSON to match the struct, validating the struct against itself.

### 2. Panic on truncated events

`extract_events` checked `len >= 9` and the tag. `decode_event` then read `SubscriptionTransfer`'s receiver at `p[160..192]`. Any tag-matching payload shorter than its event's layout sliced out of bounds and panicked the handler.

Fixed with a per-discriminator length table checked before decoding.

### 3. Insert failures returned 200

Insert errors were logged, then the handler returned `"ok"`. Helius uses at-least-once delivery and retries on non-2xx, so a transient DB error meant permanent silent loss with Helius told it succeeded.

The handler now returns `StatusCode::INTERNAL_SERVER_ERROR` on insert failure.

### 4. SQLite defaults under concurrent writes

Plain `SqlitePool::connect` with no `journal_mode` and no `busy_timeout`. SQLite allows one writer, so concurrent inserts during a delivery burst throw `SQLITE_BUSY`, which bug 3 then discarded silently.

Now WAL mode, 5 second busy timeout, `synchronous=NORMAL`.

### 5. No idempotency

Once bug 3 was fixed, retries became possible and nothing prevented duplicate rows. `signature` alone is not a key, since multiple events share one transaction signature.

Unique index on `(signature, discriminator, raw_data)`. `insert` returns `Option<TriggerEvent>`, where `None` means the row already existed and is logged as a normal skip.

The pattern across all five: every failure was silent. The first fix was not any single bug, it was making failure loud.

## Schema

```sql
trigger_events
  id                INTEGER PRIMARY KEY AUTOINCREMENT
  signature         TEXT NOT NULL
  program_id        TEXT NOT NULL
  discriminator     INTEGER NOT NULL
  raw_data          BLOB NOT NULL
  created_at        TEXT NOT NULL DEFAULT (datetime('now'))
  plan              TEXT
  subscriber        TEXT
  mint              TEXT
  amount            INTEGER
  period_start_ts   INTEGER
  period_end_ts     INTEGER

UNIQUE (signature, discriminator, raw_data)
```

Decoded columns are nullable since not every event type populates every field.

## Running

```bash
cargo test
RUST_LOG=info cargo run -p catalyst-indexer
```

`.env`:

```
HELIUS_WEBHOOK_SECRET=<matches the webhook's Authorization header>
DATABASE_URL=sqlite:///absolute/path/catalyst.db
```

Migrations run on startup.

`RUST_LOG=info` matters in practice. Without it the default level swallows every `tracing::info!` and the service looks dead while working correctly.

Helius webhook config:

- Type must be raw. Enhanced webhooks do not carry inner instruction data.
- Account: `De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44`
- URL must include the full path, `/webhook/helius`

## Known gaps

- Broadcast is lossy. `tokio::sync::broadcast` drops for subscribers that lag past capacity. Fine while nothing subscribes, wrong for a billing reactor, which needs a persistent per-subscriber queue.
- No replay. Only captures what streams past while running. Tunnel drops, crashes, and Helius gaps leave holes with no backfill and no record that a hole exists.
- No finality gate. A dropped or rolled back transaction leaves a stale row.
- Helius is a single point of failure. No RPC polling fallback.
- `amount as i64` silently narrows native u64 values. Canonical storage must preserve
  them as decimal text or fixed bytes.
- Identity uses signature/discriminator/raw bytes, so identical events at distinct
  instruction positions collide. Runtime identity must include instruction position.
- Authentication tests inspect headers/environment but do not execute middleware.
- Subscriptions event decoding is hard-coded and unversioned; retained raw event bytes
  do not establish a replayable, versioned evidence journal.
- Coverage gaps are not tracked.

## Status

Devnet. Verified against live traffic across three event types, SubscriptionCancelled, SubscriptionTransfer and SubscriptionResumed, with real signatures, plan PDAs and decoded amounts.

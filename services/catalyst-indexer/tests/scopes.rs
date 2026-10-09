use arm::{
    AuthorityKind, Authorization, Availability, EffectiveAuthorization, Principal, UsageSemantics,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use cataloger::{Catalog, ProgramVersion};
use catalyst_indexer::{Error, Journal, Origin, Projection, Record, Snapshot, Target, router};
use serde_json::Value;
use sha2::{Digest, Sha256};
use solana_pubkey::Pubkey;
use sqlx::{SqlitePool, migrate::Migrator};
use std::{
    borrow::Cow,
    fs::File,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{sync::Barrier, task::JoinSet};
use tower::ServiceExt;

fn spl() -> Snapshot {
    serde_json::from_str(include_str!("fixtures/spl-snapshot.json")).unwrap()
}
fn revoke(after: bool) -> Snapshot {
    serde_json::from_str(if after {
        include_str!("fixtures/revoke-cancelled-after.json")
    } else {
        include_str!("fixtures/revoke-cancelled-before.json")
    })
    .unwrap()
}
fn catalog() -> Catalog {
    Catalog::new(
        serde_json::from_str::<Vec<ProgramVersion>>(include_str!("fixtures/catalog.json")).unwrap(),
    )
    .unwrap()
}
fn scope(snapshot: &Snapshot) -> String {
    snapshot.scope_id().unwrap()
}
fn json(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap()
}
async fn ingest(journal: &Journal, snapshot: Snapshot) -> Record {
    journal.ingest(snapshot, &catalog()).await.unwrap()
}
async fn current(journal: &Journal, snapshot: &Snapshot) -> Record {
    journal.latest(&scope(snapshot)).await.unwrap().unwrap()
}
async fn metadata(pool: &SqlitePool, record: &Record) -> (String, Vec<u8>) {
    let query =
        sqlx::query_as("SELECT scope_id, observed_slot FROM authority_snapshots WHERE id = ?");
    query.bind(&record.id).fetch_one(pool).await.unwrap()
}
fn digest(raw: &str) -> String {
    format!("{:x}", Sha256::digest(raw.as_bytes()))
}
fn key(byte: u8) -> Pubkey {
    Pubkey::new_from_array([byte; 32])
}
fn ordering(mut snapshot: Snapshot, slot: u64) -> Snapshot {
    // Synthetic ordering observations reuse fixture bytes; no native bank is executed.
    snapshot.observation.slot = slot;
    snapshot.observation.bank = "test:synthetic-slot-ordering".into();
    snapshot
}
fn compiled(record: &Record) -> &[Authorization] {
    match &record.projection {
        Projection::Compiled { authorizations } => authorizations,
        other => panic!("expected compiled fixture, got {other:?}"),
    }
}
async fn retained(journal: &Journal, record: &Record) {
    assert_eq!(journal.get(&record.id).await.unwrap(), Some(record.clone()));
    assert_eq!(journal.replay(&record.id).await.unwrap(), *record);
}
async fn latest(journal: &Journal, record: &Record) {
    assert_eq!(current(journal, &record.snapshot).await, *record);
}
async fn get(app: &Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}
struct Database(PathBuf);
impl Database {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = PathBuf::from(format!(
            "/tmp/catalyst-scopes-{}-{nonce}.sqlite",
            std::process::id()
        ));
        File::create_new(&path).unwrap();
        Self(path)
    }
    fn url(&self) -> String {
        format!("sqlite://{}", self.0.display())
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            if let Err(error) = std::fs::remove_file(format!("{}{suffix}", self.0.display())) {
                assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
            }
        }
    }
}
async fn legacy(database: &Database, records: &[Record]) -> SqlitePool {
    let pool = SqlitePool::connect(&database.url()).await.unwrap();
    let migrations = sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version < 20261008000001)
        .cloned()
        .collect();
    let previous = Migrator {
        migrations: Cow::Owned(migrations),
        ..Migrator::DEFAULT
    };
    previous.run(&pool).await.unwrap();
    for record in records {
        let raw = serde_json::to_string(record).unwrap();
        let query = sqlx::query(
            "INSERT INTO authority_snapshots (id, record, record_sha256) VALUES (?, ?, ?)",
        );
        query
            .bind(&record.id)
            .bind(&raw)
            .bind(digest(&raw))
            .execute(&pool)
            .await
            .unwrap();
    }
    pool
}
async fn originals(pool: &SqlitePool) -> Vec<(String, String, String)> {
    let query =
        sqlx::query_as("SELECT id, record, record_sha256 FROM authority_snapshots ORDER BY id");
    query.fetch_all(pool).await.unwrap()
}

#[tokio::test]
async fn scope_is_stable_under_ordering_witness_loss_and_each_secondary_selector() {
    assert_eq!(
        scope(&spl()),
        "94eb6dbb2e175e597c4605e90ed01d2a960fca891381d6d559406615b1ad3c74"
    );
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let mut snapshot = revoke(false);
    snapshot.targets.push(spl().targets.remove(0));
    let scope_id = scope(&snapshot);
    let first = ingest(&journal, snapshot.clone()).await;
    snapshot.accounts.reverse();
    snapshot.targets.reverse();
    assert_eq!(scope(&snapshot), scope_id);
    assert_eq!(ingest(&journal, snapshot.clone()).await, first);
    latest(&journal, &first).await;
    snapshot.accounts.retain(|(address, _)| *address != key(6));
    assert_eq!(scope(&snapshot), scope_id);
    let serialized = json(&snapshot);
    for field in ["authority", "source", "mint", "plan"] {
        let mut changed = serialized.clone();
        changed["targets"][1][field] = json(key(90));
        let changed: Snapshot = serde_json::from_value(changed).unwrap();
        assert_eq!(scope(&changed), scope_id, "secondary selector {field}");
    }
    snapshot.targets.iter_mut().for_each(|target| {
        if let Target::Subscription { plan, .. } = target {
            *plan = None;
        }
    });
    assert_eq!(scope(&snapshot), scope_id);
}

#[tokio::test]
async fn cluster_origin_and_primary_target_sets_have_independent_current_records() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let base = spl();
    let mut cluster = base.clone();
    cluster.observation.cluster = "test:isolated-cluster".into();
    let mut origin = base.clone();
    origin.observation.origin = Origin::Finalized;
    let mut primary = base.clone();
    primary.targets = vec![Target::SplDelegate { source: key(99) }];
    let mut added = base.clone();
    added.targets.extend(revoke(false).targets);
    let subscription = revoke(false);
    let mut delegation = subscription.clone();
    if let Target::Subscription { delegation, .. } = &mut delegation.targets[0] {
        *delegation = key(98);
    }
    let mut records = Vec::new();
    for snapshot in [
        base,
        cluster,
        origin,
        primary,
        added,
        subscription,
        delegation,
    ] {
        let record = ingest(&journal, snapshot).await;
        assert!(
            records
                .iter()
                .all(|old: &Record| scope(&old.snapshot) != scope(&record.snapshot))
        );
        records.push(record);
    }
    for record in &records {
        latest(&journal, record).await;
    }
}

#[tokio::test]
async fn newer_failed_projections_suppress_compiled_and_stale_or_duplicate_arrivals_cannot_regress()
{
    for status in ["unsupported", "incomplete", "invalid"] {
        let journal = Journal::open("sqlite::memory:").await.unwrap();
        let old = ingest(&journal, spl()).await;
        assert!(!compiled(&old).is_empty());
        let scope_id = scope(&old.snapshot);
        let mut snapshot = ordering(spl(), if status == "unsupported" { 123 } else { 3 });
        // Missing and malformed witnesses are test mutations, not native transactions.
        if status == "incomplete" {
            snapshot.accounts.retain(|(address, _)| *address != key(2));
        }
        if status == "invalid" {
            snapshot.accounts[0].1.data.truncate(4);
        }
        let failed = ingest(&journal, snapshot).await;
        assert_eq!(json(&failed.projection)["status"], status);
        let stale = ingest(&journal, ordering(spl(), 2)).await;
        assert_eq!(ingest(&journal, spl()).await, old);
        latest(&journal, &failed).await;
        for record in [&old, &stale, &failed] {
            retained(&journal, record).await;
        }
        let app = router(journal);
        let (code, view) = get(
            &app,
            &format!("/scopes/{scope_id}/authorizations/{}", key(2)),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(view["state_version"], failed.id);
        assert_eq!(view["projection"], json(&failed.projection));
        assert_eq!(view["coverage"]["complete"], false);
        let (_, historical) = get(
            &app,
            &format!("/snapshots/{}/authorizations/{}", old.id, key(2)),
        )
        .await;
        assert_eq!(historical["projection"]["status"], "compiled");
    }
}

#[tokio::test]
async fn native_108_to_122_revoke_removes_grant_and_preserves_technical_and_admin_current() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let before: Snapshot =
        serde_json::from_str(include_str!("fixtures/owner-pull-60.json")).unwrap();
    assert_eq!(before.observation.slot, 108);
    let before = ingest(&journal, before).await;
    let after = ingest(&journal, revoke(true)).await;
    assert_eq!(after.snapshot.observation.slot, 122);
    assert_eq!(scope(&before.snapshot), scope(&after.snapshot));
    let surviving: Vec<_> = compiled(&after).iter().map(|a| a.id.as_str()).collect();
    assert_eq!(compiled(&before).len(), 3);
    assert_eq!(surviving.len(), 2);
    assert!(surviving.iter().any(|id| id.contains(":technical:")));
    assert!(surviving.iter().any(|id| id.contains(":modify-authority")));
    assert!(
        surviving
            .iter()
            .all(|id| compiled(&before).iter().any(|a| &a.id == id))
    );
    latest(&journal, &after).await;
    retained(&journal, &before).await;
    let app = router(journal);
    let Target::Subscription {
        source,
        plan: Some(plan),
        ..
    } = &before.snapshot.targets[0]
    else {
        panic!("fixture target");
    };
    for (address, remaining, original) in [(source, 1, 2), (plan, 1, 1)] {
        let (_, current) = get(
            &app,
            &format!(
                "/scopes/{}/authorizations/{address}",
                scope(&after.snapshot)
            ),
        )
        .await;
        let (_, historical) = get(
            &app,
            &format!("/snapshots/{}/authorizations/{address}", before.id),
        )
        .await;
        assert_eq!(
            current["projection"]["authorizations"]
                .as_array()
                .unwrap()
                .len(),
            remaining
        );
        assert_eq!(
            historical["projection"]["authorizations"]
                .as_array()
                .unwrap()
                .len(),
            original
        );
    }
}

#[tokio::test]
async fn actual_same_slot_revoke_observations_are_ambiguous_in_either_arrival_order_until_newer() {
    assert_eq!(revoke(false).observation, revoke(true).observation);
    assert_eq!(revoke(false).observation.slot, 122);
    for order in [[false, true], [true, false]] {
        let journal = Journal::open("sqlite::memory:").await.unwrap();
        let first = ingest(&journal, revoke(order[0])).await;
        latest(&journal, &first).await;
        let second = ingest(&journal, revoke(order[1])).await;
        assert_ne!(first.id, second.id);
        assert_eq!(compiled(&first).len(), if order[0] { 2 } else { 3 });
        assert_eq!(compiled(&second).len(), if order[1] { 2 } else { 3 });
        let scope_id = scope(&first.snapshot);
        let older: Snapshot =
            serde_json::from_str(include_str!("fixtures/owner-pull-60.json")).unwrap();
        ingest(&journal, older).await;
        for record in [&first, &second] {
            retained(&journal, record).await;
            assert_eq!(ingest(&journal, record.snapshot.clone()).await, *record);
            assert!(matches!(
                journal.latest(&scope_id).await,
                Err(Error::AmbiguousScope)
            ));
        }
        let app = router(journal.clone());
        for path in [
            format!("/scopes/{scope_id}"),
            format!("/scopes/{scope_id}/authorizations/{}", key(1)),
        ] {
            assert_eq!(get(&app, &path).await.0, StatusCode::CONFLICT);
        }
        for record in [&first, &second] {
            assert_eq!(
                get(&app, &format!("/snapshots/{}", record.id)).await,
                (StatusCode::OK, json(record))
            );
        }
        let newer = ingest(&journal, ordering(revoke(true), 123)).await;
        assert!(matches!(newer.projection, Projection::Unsupported { .. }));
        latest(&journal, &newer).await;
        assert_eq!(
            get(&app, &format!("/scopes/{scope_id}")).await.0,
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn slots_order_as_unsigned_u64_through_maximum_and_retain_every_arrival() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let mut maximum = 0;
    let signed_max = i64::MAX as u64;
    for slot in [
        9,
        10,
        255,
        256,
        signed_max,
        1 << 63,
        u64::MAX - 1,
        u64::MAX,
        256,
        0,
    ] {
        let record = ingest(&journal, ordering(spl(), slot)).await;
        maximum = maximum.max(slot);
        assert_eq!(
            current(&journal, &record.snapshot)
                .await
                .snapshot
                .observation
                .slot,
            maximum
        );
        retained(&journal, &record).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_high_and_low_slot_writers_keep_unsigned_maximum() {
    let database = Database::new();
    let journals = [
        Journal::open(&database.url()).await.unwrap(),
        Journal::open(&database.url()).await.unwrap(),
    ];
    let slots = [3, u64::MAX, 256, 1 << 63, 2, 255];
    let barrier = Arc::new(Barrier::new(slots.len()));
    let mut tasks = JoinSet::new();
    for (index, slot) in slots.into_iter().enumerate() {
        let journal = journals[index % 2].clone();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            barrier.wait().await;
            ingest(&journal, ordering(spl(), slot)).await
        });
    }
    while let Some(result) = tasks.join_next().await {
        let record = result.unwrap();
        retained(&journals[0], &record).await;
    }
    assert_eq!(
        current(&journals[1], &spl())
            .await
            .snapshot
            .observation
            .slot,
        u64::MAX
    );
}

#[tokio::test]
async fn rejected_insert_rolls_back_journal_and_current_index() {
    let database = Database::new();
    let journal = Journal::open(&database.url()).await.unwrap();
    let pool = SqlitePool::connect(&database.url()).await.unwrap();
    for seeded in [false, true] {
        let previous = journal.latest(&scope(&spl())).await.unwrap();
        assert_eq!(previous.is_some(), seeded);
        sqlx::query(
            "CREATE TRIGGER reject_snapshot BEFORE INSERT ON authority_snapshots BEGIN SELECT RAISE(ABORT, 'test insert failure'); END",
        )
        .execute(&pool)
        .await
        .unwrap();
        let before = originals(&pool).await;
        assert!(matches!(
            journal.ingest(ordering(spl(), 3), &catalog()).await,
            Err(Error::Database(_))
        ));
        assert_eq!(originals(&pool).await, before);
        assert_eq!(journal.latest(&scope(&spl())).await.unwrap(), previous);
        sqlx::query("DROP TRIGGER reject_snapshot")
            .execute(&pool)
            .await
            .unwrap();
        let restored = ingest(&journal, spl()).await;
        latest(&journal, &restored).await;
    }
}

#[tokio::test]
async fn legacy_writer_missing_metadata_is_rejected_even_for_existing_id() {
    let database = Database::new();
    let journal = Journal::open(&database.url()).await.unwrap();
    let pool = SqlitePool::connect(&database.url()).await.unwrap();
    let source = Journal::open("sqlite::memory:").await.unwrap();
    let record = ingest(&source, spl()).await;
    let raw = serde_json::to_string(&record).unwrap();
    for existing in [false, true] {
        let before = originals(&pool).await;
        assert_eq!(before.is_empty(), !existing);
        assert!(
            sqlx::query("INSERT INTO authority_snapshots (id, record, record_sha256) VALUES (?, ?, ?) ON CONFLICT(id) DO NOTHING")
                .bind(&record.id)
                .bind(&raw)
                .bind(digest(&raw))
                .execute(&pool)
                .await
                .is_err()
        );
        assert_eq!(originals(&pool).await, before);
        assert_eq!(ingest(&journal, spl()).await, record);
    }
}

#[tokio::test]
async fn duplicate_provenance_conflict_preserves_original_record_and_index_metadata() {
    let database = Database::new();
    let journal = Journal::open(&database.url()).await.unwrap();
    let pool = SqlitePool::connect(&database.url()).await.unwrap();
    let record = ingest(&journal, spl()).await;
    let before = originals(&pool).await;
    let mut versions: Vec<ProgramVersion> =
        serde_json::from_str(include_str!("fixtures/catalog.json")).unwrap();
    versions[0]
        .provenance
        .evidence_reference
        .push_str(":test-conflicting-provenance");
    assert!(matches!(
        journal
            .ingest(spl(), &Catalog::new(versions).unwrap())
            .await,
        Err(Error::Conflict)
    ));
    assert_eq!(originals(&pool).await, before);
    let position = metadata(&pool, &record).await;
    assert_eq!(
        position,
        (scope(&record.snapshot), 1_u64.to_be_bytes().to_vec())
    );
    latest(&journal, &record).await;
    retained(&journal, &record).await;
}

#[tokio::test]
async fn checkpoint_reopen_and_legacy_migration_preserve_originals_current_and_ambiguity() {
    let source = Journal::open("sqlite::memory:").await.unwrap();
    let mut records = Vec::new();
    for snapshot in [
        ordering(spl(), u64::MAX),
        spl(),
        revoke(true),
        revoke(false),
    ] {
        records.push(ingest(&source, snapshot).await);
    }
    for old_schema in [false, true] {
        let database = Database::new();
        let pool = if old_schema {
            legacy(&database, &records).await
        } else {
            SqlitePool::connect(&database.url()).await.unwrap()
        };
        let original = if old_schema {
            originals(&pool).await
        } else {
            Vec::new()
        };
        if old_schema {
            let columns: Vec<String> =
                sqlx::query_scalar("SELECT name FROM pragma_table_info('authority_snapshots')")
                    .fetch_all(&pool)
                    .await
                    .unwrap();
            assert_eq!(columns, ["id", "record", "record_sha256"]);
        }
        let journal = Journal::open(&database.url()).await.unwrap();
        if old_schema {
            assert_eq!(originals(&pool).await, original);
        }
        for record in &records {
            assert_eq!(ingest(&journal, record.snapshot.clone()).await, *record);
            assert_eq!(
                metadata(&pool, record).await,
                (
                    scope(&record.snapshot),
                    record.snapshot.observation.slot.to_be_bytes().to_vec()
                )
            );
        }
        let before = originals(&pool).await;
        let checkpoint: (i64, i64, i64) = sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(checkpoint.0, 0);
        pool.close().await;
        drop(journal);
        let reopened = Journal::open(&database.url()).await.unwrap();
        for record in &records {
            retained(&reopened, record).await;
        }
        latest(&reopened, &records[0]).await;
        assert!(matches!(
            reopened.latest(&scope(&revoke(false))).await,
            Err(Error::AmbiguousScope)
        ));
        let pool = SqlitePool::connect(&database.url()).await.unwrap();
        assert_eq!(originals(&pool).await, before);
        pool.close().await;
    }
    let corrupt = Database::new();
    let pool = legacy(&corrupt, &records[..1]).await;
    sqlx::query("UPDATE authority_snapshots SET record_sha256 = 'test:corrupt'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        Journal::open(&corrupt.url()).await,
        Err(Error::ReplayMismatch)
    ));
    let position: (Option<String>, Option<Vec<u8>>) =
        sqlx::query_as("SELECT scope_id, observed_slot FROM authority_snapshots")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(position, (None, None));
}

#[tokio::test]
async fn corrupt_index_scope_slot_or_checksum_fails_closed_with_http_500() {
    let database = Database::new();
    let journal = Journal::open(&database.url()).await.unwrap();
    let record = ingest(&journal, spl()).await;
    let pool = SqlitePool::connect(&database.url()).await.unwrap();
    let scope_id = scope(&record.snapshot);
    let raw = serde_json::to_string(&record).unwrap();
    let checksum = digest(&raw);
    let app = router(journal.clone());
    for (lookup, slot, digest) in [
        ("test-corrupt-scope", 1_u64, checksum.as_str()),
        (scope_id.as_str(), 2, checksum.as_str()),
        (scope_id.as_str(), 1, "test-corrupt-checksum"),
    ] {
        let query = sqlx::query(
            "UPDATE authority_snapshots SET scope_id = ?, observed_slot = ?, record_sha256 = ? WHERE id = ?",
        );
        let query = query
            .bind(lookup)
            .bind(slot.to_be_bytes().to_vec())
            .bind(digest);
        query.bind(&record.id).execute(&pool).await.unwrap();
        assert!(matches!(
            journal.latest(lookup).await,
            Err(Error::ReplayMismatch)
        ));
        for path in [
            format!("/scopes/{lookup}"),
            format!("/scopes/{lookup}/authorizations/{}", key(2)),
        ] {
            assert_eq!(get(&app, &path).await.0, StatusCode::INTERNAL_SERVER_ERROR);
        }
        if digest == checksum {
            retained(&journal, &record).await;
        }
    }
    let restore = sqlx::query(
        "UPDATE authority_snapshots SET scope_id = ?, observed_slot = ?, record_sha256 = ? WHERE id = ?",
    );
    restore
        .bind(&scope_id)
        .bind(1_u64.to_be_bytes().to_vec())
        .bind(&checksum)
        .bind(&record.id)
        .execute(&pool)
        .await
        .unwrap();
    let newer = ingest(&journal, ordering(spl(), 2)).await;
    assert!(newer.id < record.id, "healthy candidate must sort first");
    let tie = sqlx::query("UPDATE authority_snapshots SET observed_slot = ? WHERE id = ?");
    tie.bind(2_u64.to_be_bytes().to_vec())
        .bind(&record.id)
        .execute(&pool)
        .await
        .unwrap();
    let before = ingest(&journal, revoke(false)).await;
    let after = ingest(&journal, revoke(true)).await;
    let corrupt_second = if before.id > after.id {
        &before
    } else {
        &after
    };
    let corrupt =
        sqlx::query("UPDATE authority_snapshots SET record_sha256 = 'test:corrupt' WHERE id = ?");
    corrupt
        .bind(&corrupt_second.id)
        .execute(&pool)
        .await
        .unwrap();
    for scope_id in [scope_id, scope(&before.snapshot)] {
        assert!(matches!(
            journal.latest(&scope_id).await,
            Err(Error::ReplayMismatch)
        ));
        for path in [
            format!("/scopes/{scope_id}"),
            format!("/scopes/{scope_id}/authorizations/{}", key(2)),
        ] {
            assert_eq!(get(&app, &path).await.0, StatusCode::INTERNAL_SERVER_ERROR);
        }
    }
}

#[tokio::test]
async fn current_http_returns_latest_partial_coverage_provenance_and_400_404() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let old = ingest(&journal, spl()).await;
    let record = ingest(&journal, ordering(spl(), 2)).await;
    let scope_id = scope(&record.snapshot);
    let app = router(journal);
    assert_eq!(
        get(&app, &format!("/scopes/{scope_id}")).await,
        (StatusCode::OK, json(&record))
    );
    let (status, view) = get(
        &app,
        &format!("/scopes/{scope_id}/authorizations/{}", key(2)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let accounts: Vec<_> = record
        .snapshot
        .accounts
        .iter()
        .map(|(key, _)| key)
        .collect();
    assert_eq!(
        view,
        serde_json::json!({
            "state_version": record.id, "observation": record.snapshot.observation,
            "coverage": {"complete": false, "accounts": accounts, "targets": record.snapshot.targets},
            "versions": record.versions, "projection": record.projection,
            "sdk_revision": catalyst_indexer::SDK_REVISION,
            "runtime_version": catalyst_indexer::RUNTIME_VERSION,
            "arm_schema_version": arm::SCHEMA_VERSION
        })
    );
    let authorizations: Vec<Authorization> =
        serde_json::from_value(view["projection"]["authorizations"].clone()).unwrap();
    assert_eq!(authorizations, compiled(&record));
    assert!(!authorizations.is_empty());
    assert_eq!(
        authorizations[0].evidence.references,
        vec![
            format!("journal:sha256:{}", record.id),
            record.versions[0].provenance.evidence_reference.clone(),
            format!("catalyst-sdk:git:{}", catalyst_indexer::SDK_REVISION)
        ]
    );
    assert_eq!(
        get(&app, &format!("/snapshots/{}", old.id)).await,
        (StatusCode::OK, json(&old))
    );
    let (_, empty) = get(
        &app,
        &format!("/scopes/{scope_id}/authorizations/{}", key(99)),
    )
    .await;
    assert_eq!(empty["projection"]["authorizations"], serde_json::json!([]));
    for (path, expected) in [
        (
            format!("/scopes/{scope_id}/authorizations/not-a-pubkey"),
            StatusCode::BAD_REQUEST,
        ),
        ("/scopes/missing".into(), StatusCode::NOT_FOUND),
        (
            format!("/scopes/missing/authorizations/{}", key(2)),
            StatusCode::NOT_FOUND,
        ),
    ] {
        assert_eq!(get(&app, &path).await.0, expected);
    }
}

#[tokio::test]
async fn effective_native_authority_uses_observed_clock_and_preserves_uncertainty() {
    let plan: Snapshot = serde_json::from_str(include_str!("fixtures/owner-pull-60.json")).unwrap();
    for (snapshot, address, count) in [
        (spl(), key(2), 1),
        (plan.clone(), key(1), 2),
        (plan, key(2), 2),
        (revoke(false), key(1), 2),
    ] {
        let journal = Journal::open("sqlite::memory:").await.unwrap();
        let record = ingest(&journal, snapshot).await;
        let app = router(journal);
        let prefix = format!("/scopes/{}", scope(&record.snapshot));
        let (status, raw) = get(&app, &format!("{prefix}/authorizations/{address}")).await;
        assert_eq!(status, StatusCode::OK);
        let (status, mut evaluated) = get(&app, &format!("{prefix}/effective/{address}")).await;
        assert_eq!(status, StatusCode::OK);
        let effective: Vec<EffectiveAuthorization> =
            serde_json::from_value(evaluated["projection"]["authorizations"].clone()).unwrap();
        assert_eq!(effective.len(), count);
        let expired = record.snapshot.observation.slot == 122;
        for entry in &effective {
            assert_eq!(
                entry.evaluated_at_unix_seconds,
                record.snapshot.observation.unix_timestamp
            );
            assert_eq!(
                entry.availability,
                match entry.authorization.authority_kind {
                    AuthorityKind::Direct => Availability::Conditional,
                    AuthorityKind::Derived { .. } if expired => Availability::Inactive,
                    AuthorityKind::Derived { .. } | AuthorityKind::Administrative => {
                        Availability::Unknown
                    }
                }
            );
        }
        evaluated["projection"]["authorizations"] = json(
            effective
                .iter()
                .map(|entry| &entry.authorization)
                .collect::<Vec<_>>(),
        );
        assert_eq!(evaluated, raw);
        assert_eq!(evaluated["coverage"]["complete"], false);
    }
}

#[tokio::test]
async fn scoped_views_cannot_restore_a_revoked_or_failed_grant() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let initial: Snapshot =
        serde_json::from_str(include_str!("fixtures/owner-pull-60.json")).unwrap();
    let record = ingest(&journal, initial.clone()).await;
    let app = router(journal.clone());
    let merchant = format!("/scopes/{}/effective/{}", scope(&initial), key(6));
    let (_, before) = get(&app, &merchant).await;
    assert_eq!(
        before["projection"]["authorizations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        before["projection"]["authorizations"][0]["availability"],
        "unknown"
    );
    let revoked = ingest(&journal, revoke(true)).await;
    let (status, after) = get(&app, &merchant).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after["state_version"], revoked.id);
    assert_eq!(
        after["projection"],
        serde_json::json!({"status": "compiled", "authorizations": []})
    );
    assert_eq!(ingest(&journal, initial).await, record);
    assert_eq!(get(&app, &merchant).await.1, after);
    let (status, graph) = get(
        &app,
        &format!("/scopes/{}/graph/{}", scope(&revoked.snapshot), key(6)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(graph["state_version"], revoked.id);
    assert_eq!(graph["projection"], after["projection"]);
    assert_eq!(graph["unresolved_parents"], serde_json::json!([]));
    for expected in ["unsupported", "incomplete", "invalid"] {
        let journal = Journal::open("sqlite::memory:").await.unwrap();
        ingest(&journal, spl()).await;
        let mut failed = ordering(spl(), if expected == "unsupported" { 123 } else { 2 });
        if expected == "incomplete" {
            failed.accounts.retain(|(address, _)| *address != key(2));
        }
        if expected == "invalid" {
            failed.accounts[0].1.data.truncate(4);
        }
        let record = ingest(&journal, failed).await;
        let app = router(journal);
        let prefix = format!("/scopes/{}", scope(&record.snapshot));
        for suffix in [
            format!("effective/{}", key(2)),
            format!("graph/{}", key(2)),
            "coverage".into(),
        ] {
            let (status, view) = get(&app, &format!("{prefix}/{suffix}")).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(view["state_version"], record.id);
            assert_eq!(view["projection"]["status"], expected);
            assert_eq!(view["projection"], json(&record.projection));
            if suffix.starts_with("graph/") {
                assert_eq!(view["unresolved_parents"], Value::Null);
            }
        }
    }
}

#[tokio::test]
async fn scoped_routes_reject_bad_addresses_conflicting_banks_and_invalid_arm() {
    let database = Database::new();
    let journal = Journal::open(&database.url()).await.unwrap();
    let record = ingest(&journal, spl()).await;
    ingest(&journal, revoke(false)).await;
    ingest(&journal, revoke(true)).await;
    let app = router(journal.clone());
    for (path, expected) in [
        (
            format!("/scopes/{}/effective/invalid", scope(&record.snapshot)),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!("/scopes/missing/effective/{}", key(2)),
            StatusCode::NOT_FOUND,
        ),
        (
            format!("/scopes/{}/effective/{}", scope(&revoke(false)), key(1)),
            StatusCode::CONFLICT,
        ),
        (
            format!("/scopes/{}/graph/invalid", scope(&record.snapshot)),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!("/scopes/missing/graph/{}", key(2)),
            StatusCode::NOT_FOUND,
        ),
        ("/scopes/missing/coverage".into(), StatusCode::NOT_FOUND),
        (
            format!("/scopes/{}/graph/{}", scope(&revoke(false)), key(1)),
            StatusCode::CONFLICT,
        ),
        (
            format!("/scopes/{}/coverage", scope(&revoke(false))),
            StatusCode::CONFLICT,
        ),
    ] {
        assert_eq!(get(&app, &path).await.0, expected);
    }
    let path = format!("/scopes/{}/effective/{}", scope(&record.snapshot), key(2));
    let pool = SqlitePool::connect(&database.url()).await.unwrap();
    let mut malformed = record.clone();
    if let Projection::Compiled { authorizations } = &mut malformed.projection {
        authorizations[0].schema_version = "unsupported".into();
    }
    let raw = serde_json::to_string(&malformed).unwrap();
    sqlx::query("UPDATE authority_snapshots SET record = ?, record_sha256 = ? WHERE id = ?")
        .bind(&raw)
        .bind(digest(&raw))
        .bind(&record.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(get(&app, &path).await.0, StatusCode::INTERNAL_SERVER_ERROR);
    let graph = format!("/scopes/{}/graph/{}", scope(&record.snapshot), key(2));
    assert_eq!(get(&app, &graph).await.0, StatusCode::INTERNAL_SERVER_ERROR);
    sqlx::query("UPDATE authority_snapshots SET record_sha256 = 'corrupt' WHERE id = ?")
        .bind(&record.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(get(&app, &path).await.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(get(&app, &graph).await.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        get(
            &app,
            &format!("/scopes/{}/coverage", scope(&record.snapshot))
        )
        .await
        .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn graph_preserves_native_lineage_shared_budget_and_administrative_authority() {
    let snapshot: Snapshot =
        serde_json::from_str(include_str!("fixtures/owner-pull-60.json")).unwrap();
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let record = ingest(&journal, snapshot).await;
    let app = router(journal);
    let prefix = format!("/scopes/{}", scope(&record.snapshot));
    let (status, coverage) = get(&app, &format!("{prefix}/coverage")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        coverage["projection"]["authorizations"],
        json(compiled(&record))
    );
    assert_eq!(coverage["state_version"], record.id);
    assert_eq!(coverage["observation"], json(&record.snapshot.observation));
    assert_eq!(coverage["versions"], json(&record.versions));
    assert_eq!(coverage["coverage"]["complete"], false);
    let technical = compiled(&record)
        .iter()
        .find(|authorization| authorization.authority_kind == AuthorityKind::Direct)
        .unwrap();
    let Principal::Identity(signer) = &technical.principal else {
        panic!("native technical principal must be an identity")
    };
    for (address, count) in [
        (key(6).to_string(), 2),
        (signer.clone(), 2),
        (key(2).to_string(), 3),
        (key(99).to_string(), 0),
    ] {
        let path = format!("{prefix}/graph/{address}");
        let (status, mut graph) = get(&app, &path).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(get(&app, &path).await.1, graph);
        let nodes: Vec<EffectiveAuthorization> =
            serde_json::from_value(graph["projection"]["authorizations"].clone()).unwrap();
        assert_eq!(nodes.len(), count);
        assert_eq!(graph["unresolved_parents"], serde_json::json!([]));
        for node in &nodes {
            assert!(compiled(&record).contains(&node.authorization));
            assert_eq!(
                node.evaluated_at_unix_seconds,
                record.snapshot.observation.unix_timestamp
            );
            match &node.authorization.authority_kind {
                AuthorityKind::Direct => {
                    assert_eq!(node.availability, Availability::Conditional);
                    assert_eq!(
                        node.authorization.usage,
                        UsageSemantics::Cumulative {
                            remaining: Some(u64::MAX - 60)
                        }
                    );
                }
                AuthorityKind::Derived { parents } => {
                    assert_eq!(parents, std::slice::from_ref(&technical.id));
                    assert_eq!(node.availability, Availability::Unknown);
                    assert_eq!(
                        node.authorization.usage,
                        UsageSemantics::Recurring {
                            period_seconds: 3600,
                            anchor_unix_seconds: 1_800_000_000,
                            observed_period_start: 1_800_000_000,
                            remaining: Some(40),
                        }
                    );
                    assert!(
                        matches!(&node.authorization.principal, Principal::AnyOf(principals) if principals.len() == 2)
                    );
                }
                AuthorityKind::Administrative => {
                    assert_eq!(node.availability, Availability::Unknown)
                }
            }
        }
        graph.as_object_mut().unwrap().remove("unresolved_parents");
        graph["projection"] = coverage["projection"].clone();
        assert_eq!(graph, coverage);
    }
}

#[tokio::test]
async fn graph_uses_only_current_parent_evidence_and_bounds_cyclic_or_ambiguous_lineage() {
    let snapshot: Snapshot =
        serde_json::from_str(include_str!("fixtures/owner-pull-60.json")).unwrap();
    let database = Database::new();
    let journal = Journal::open(&database.url()).await.unwrap();
    let old = ingest(&journal, snapshot.clone()).await;
    let current = ingest(&journal, ordering(snapshot, 109)).await;
    let app = router(journal);
    let pool = SqlitePool::connect(&database.url()).await.unwrap();
    let path = format!("/scopes/{}/graph/{}", scope(&current.snapshot), key(6));
    let parent = compiled(&old)
        .iter()
        .find(|a| a.authority_kind == AuthorityKind::Direct)
        .unwrap()
        .id
        .clone();
    for case in ["missing", "cycle", "duplicate"] {
        // Synthetic retained projections test graph behavior, not native program execution.
        let mut record = current.clone();
        let Projection::Compiled { authorizations } = &mut record.projection else {
            panic!("expected native compiled projection")
        };
        if case == "missing" {
            authorizations.retain(|a| a.id != parent);
        } else if case == "cycle" {
            let child = authorizations
                .iter()
                .find(|a| matches!(a.authority_kind, AuthorityKind::Derived { .. }))
                .unwrap()
                .id
                .clone();
            authorizations
                .iter_mut()
                .find(|a| a.id == parent)
                .unwrap()
                .authority_kind = AuthorityKind::Derived {
                parents: vec![child],
            };
        } else {
            authorizations.push(authorizations[0].clone());
        }
        let raw = serde_json::to_string(&record).unwrap();
        sqlx::query("UPDATE authority_snapshots SET record = ?, record_sha256 = ? WHERE id = ?")
            .bind(&raw)
            .bind(digest(&raw))
            .bind(&current.id)
            .execute(&pool)
            .await
            .unwrap();
        let (status, graph) = get(&app, &path).await;
        if case == "duplicate" {
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            continue;
        }
        assert_eq!(status, StatusCode::OK);
        assert_eq!(graph["state_version"], current.id);
        assert_eq!(graph["coverage"]["complete"], false);
        let nodes: Vec<EffectiveAuthorization> =
            serde_json::from_value(graph["projection"]["authorizations"].clone()).unwrap();
        assert!(
            nodes
                .iter()
                .all(|node| node.availability == Availability::Unknown)
        );
        assert_eq!(nodes.len(), if case == "missing" { 1 } else { 2 });
        assert_eq!(
            graph["unresolved_parents"],
            if case == "missing" {
                json(vec![parent.clone()])
            } else {
                serde_json::json!([])
            }
        );
    }
}

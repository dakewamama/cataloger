use arm::{AuthorityKind, Authorization, UsageSemantics};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use cataloger::{Catalog, ProgramVersion};
use catalyst_indexer::{
    Error, Journal, Origin, Projection, RUNTIME_VERSION, Record, SDK_REVISION, Snapshot, Target,
    router,
};
use catalyst_sdk::{Adapter, spl, subscriptions};
use serde_json::Value;
use sha2::{Digest, Sha256};
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use spl_token_interface::state::Account as TokenAccount;
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use tower::ServiceExt;

const FIXTURE_SDK_REVISION: &str = "b3682df29b3b6c64e9fcf96b35a6dd23fa552759";
const CLUSTER: &str = "local:mollusk-fixtures";
const BANK: &str = "mollusk:fixture-bank";
const SPL_HASH: &str = "f5cfe08fa28ea703b5895b36d519811a0ff7f55b5728e4e550741a882bbb66f5";
const TRACE_HASH: &str = "cda5667e1273599ebbee906b293dba2ae1f8a6f99626be961a40172a759f4b23";
const DELEGATION: &str = "Cpy5ZUAnt9UFD6g4jzUAQfk3AhzDVhRbo2Ye9nJYjVfv";
const AUTHORITY: &str = "C6SqwcnpGmWGkM3UuWJ2w58UZvYrgUxQaAFem6wPSBW6";
const SOURCE: &str = "HwD4QpS4bsutLbWZWhbmFUZfkXC5Au1DbkzYEzjDgps8";
const PLAN: &str = "3DztDM68WwtEuWfnPeP4CsEijJaX1tp5x9YszSsSguvJ";

fn key(value: &str) -> Pubkey {
    value.parse().unwrap()
}

fn versions() -> Vec<ProgramVersion> {
    serde_json::from_str(include_str!("fixtures/catalog.json")).unwrap()
}

fn catalog() -> Catalog {
    Catalog::new(versions()).unwrap()
}

fn spl_snapshot() -> Snapshot {
    serde_json::from_str(include_str!("fixtures/spl-snapshot.json")).unwrap()
}

fn subscription_snapshot(name: &str, phase: &str) -> Snapshot {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/subscriptions-snapshots.json")).unwrap();
    assert_eq!(fixture["sdk_revision"], FIXTURE_SDK_REVISION);
    assert_eq!(fixture["source_fixture_sha256"], TRACE_HASH);
    assert_eq!(
        format!("sha256:{}", fixture["program_sha256"].as_str().unwrap()),
        subscriptions::PROGRAM_VERSION
    );
    assert_eq!(
        format!("sha256:{}", fixture["token_sha256"].as_str().unwrap()),
        spl::PROGRAM_VERSION
    );
    let step = fixture["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["name"] == name && step["phase"] == phase)
        .unwrap();
    assert_eq!(step["outcome"], "Ok(())");
    let json = match (name, phase) {
        ("owner_pull_60", "after") => include_str!("fixtures/owner-pull-60.json"),
        ("revoke_cancelled", "before") => include_str!("fixtures/revoke-cancelled-before.json"),
        ("revoke_cancelled", "after") => include_str!("fixtures/revoke-cancelled-after.json"),
        _ => panic!("unknown fixture {name}:{phase}"),
    };
    let snapshot: Snapshot = serde_json::from_str(json).unwrap();
    assert_eq!(
        snapshot.observation.slot,
        step["clock"]["slot"].as_u64().unwrap()
    );
    assert_eq!(
        snapshot.observation.unix_timestamp,
        step["clock"]["unix_timestamp"].as_i64().unwrap()
    );
    snapshot
}

fn compiled(record: &Record) -> &[Authorization] {
    match &record.projection {
        Projection::Compiled { authorizations } => authorizations,
        other => panic!("expected native projection, got {other:?}"),
    }
}

fn assert_raw_retained(record: &Record, input: &Snapshot) {
    assert!(!record.coverage_complete);
    assert_eq!(record.snapshot.observation, input.observation);
    assert_eq!(
        record
            .snapshot
            .accounts
            .iter()
            .cloned()
            .collect::<BTreeMap<_, _>>(),
        input.accounts.iter().cloned().collect()
    );
    assert_eq!(record.snapshot.targets.len(), input.targets.len());
    assert!(
        input
            .targets
            .iter()
            .all(|target| record.snapshot.targets.contains(target))
    );
}

fn assert_golden(record: &Record, golden: &str) {
    let mut actual = compiled(record).to_vec();
    let mut expected: Vec<Authorization> = serde_json::from_str(golden).unwrap();
    actual.sort_by(|a, b| a.id.cmp(&b.id));
    expected.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(actual.len(), expected.len());
    for (authorization, golden) in actual.iter_mut().zip(&expected) {
        let version = record
            .versions
            .iter()
            .find(|v| v.adapter.protocol == authorization.native_context.protocol)
            .unwrap();
        assert_eq!(
            authorization.evidence.references,
            vec![
                format!("journal:sha256:{}", record.id),
                version.provenance.evidence_reference.clone(),
                format!("catalyst-sdk:git:{SDK_REVISION}"),
            ]
        );
        assert_eq!(
            authorization.evidence.observed_at,
            format!("{}:{}:{}", CLUSTER, BANK, record.snapshot.observation.slot)
        );
        authorization.evidence = golden.evidence.clone();
    }
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn native_spl_and_subscription_snapshots_match_independent_goldens() {
    let known = versions();
    assert_eq!(
        known[0].program_id,
        spl::DelegateAdapter.protocol().program_id
    );
    assert_eq!(known[0].version, spl::PROGRAM_VERSION);
    assert_eq!(known[0].deployment, spl::DEPLOYMENT);
    assert_eq!(known[0].adapter.version, "0.1");
    assert_eq!(known[0].schema.revision, "3.0.0");
    assert_eq!(
        known[1].program_id,
        subscriptions::DelegationAdapter.protocol().program_id
    );
    assert_eq!(known[1].version, subscriptions::PROGRAM_VERSION);
    assert_eq!(known[1].deployment, subscriptions::DEPLOYMENT);
    assert_eq!(known[1].adapter.version, subscriptions::ADAPTER_VERSION);
    assert_eq!(known[1].schema.revision, "0.5.0");
    assert_eq!(
        spl_snapshot().accounts[0].1.data,
        include_bytes!("fixtures/spl-delegate.bin")
    );
    for (version, module) in known.iter().zip(["spl", "subscriptions"]) {
        assert_eq!(
            version.adapter.source,
            format!(
                "https://github.com/dakewamama/catalyst-sdk/blob/{SDK_REVISION}/src/{module}.rs"
            )
        );
        assert_eq!(version.cluster, CLUSTER);
        assert_eq!(version.provenance.revision, FIXTURE_SDK_REVISION);
        assert_eq!(version.supported_until_slot_exclusive, 123);
    }
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("fixtures/spl-delegate.bin"))
        ),
        SPL_HASH
    );
    for (snapshot, golden, version_count) in [
        (
            spl_snapshot(),
            include_str!("fixtures/spl-delegate.json"),
            1,
        ),
        (
            subscription_snapshot("owner_pull_60", "after"),
            include_str!("fixtures/subscriptions-plan-arm.json"),
            2,
        ),
    ] {
        let journal = Journal::open("sqlite::memory:").await.unwrap();
        let record = journal.ingest(snapshot.clone(), &catalog()).await.unwrap();
        assert_raw_retained(&record, &snapshot);
        assert_golden(&record, golden);
        assert_eq!(record.versions.len(), version_count);
        assert_eq!(record.sdk_revision, SDK_REVISION);
        assert_eq!(record.runtime_version, RUNTIME_VERSION);
        assert_eq!(journal.get(&record.id).await.unwrap(), Some(record.clone()));
        assert_eq!(journal.replay(&record.id).await.unwrap(), record);
    }
}

#[tokio::test]
async fn reordering_is_idempotent_duplicates_are_rejected_and_provenance_is_immutable() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let mut snapshot = subscription_snapshot("owner_pull_60", "after");
    let second_delegation = key("72WBUVFCYnM49PzaHQgDjb8gG8DRdWarmnHTNGhSSNyf");
    snapshot.targets.push(Target::Subscription {
        delegation: second_delegation,
        authority: key(AUTHORITY),
        source: key(SOURCE),
        mint: Pubkey::new_from_array([3; 32]),
        plan: Some(key("GaXQmCTVvqbE5B987e4cVUCXTRpRF4L2cVPYo8QynyQC")),
    });
    let first = journal.ingest(snapshot.clone(), &catalog()).await.unwrap();
    assert_eq!(compiled(&first).len(), 5);
    let technical = compiled(&first)
        .iter()
        .filter(|a| a.id.contains(":technical:"))
        .collect::<Vec<_>>();
    assert_eq!(technical.len(), 1);
    assert_eq!(
        technical[0].usage,
        UsageSemantics::Cumulative {
            remaining: Some(u64::MAX - 60)
        }
    );
    for (delegation, remaining) in [(key(DELEGATION), 40), (second_delegation, 100)] {
        let grant = compiled(&first)
            .iter()
            .find(|a| a.id.contains(&delegation.to_string()))
            .unwrap();
        assert_eq!(
            grant.authority_kind,
            AuthorityKind::Derived {
                parents: vec![technical[0].id.clone()]
            }
        );
        assert_eq!(
            grant.usage,
            UsageSemantics::Recurring {
                period_seconds: 3600,
                anchor_unix_seconds: 1_800_000_000,
                observed_period_start: 1_800_000_000,
                remaining: Some(remaining)
            }
        );
    }
    snapshot.accounts.reverse();
    snapshot.targets.reverse();
    assert_eq!(
        journal.ingest(snapshot.clone(), &catalog()).await.unwrap(),
        first
    );
    let mut changed_versions = versions();
    changed_versions[0]
        .provenance
        .evidence_reference
        .push_str(":conflicting-source");
    let conflict = Catalog::new(changed_versions).unwrap();
    assert!(matches!(
        journal.ingest(snapshot.clone(), &conflict).await,
        Err(Error::Conflict)
    ));
    let mut duplicate_account = snapshot.clone();
    duplicate_account
        .accounts
        .push(duplicate_account.accounts[0].clone());
    assert!(matches!(
        journal.ingest(duplicate_account, &catalog()).await,
        Err(Error::InvalidSnapshot)
    ));
    let mut duplicate_target = snapshot;
    duplicate_target
        .targets
        .push(duplicate_target.targets[0].clone());
    assert!(matches!(
        journal.ingest(duplicate_target, &catalog()).await,
        Err(Error::InvalidSnapshot)
    ));
    assert_eq!(journal.get(&first.id).await.unwrap(), Some(first.clone()));
    assert_eq!(journal.replay(&first.id).await.unwrap(), first);
}

#[tokio::test]
async fn failed_projections_remain_distinct_and_retain_their_raw_evidence() {
    for case in [
        "unknown-version",
        "coverage-gap",
        "missing-evidence",
        "malformed-state",
    ] {
        let journal = Journal::open("sqlite::memory:").await.unwrap();
        let mut snapshot = subscription_snapshot("owner_pull_60", "after");
        let mut known = versions();
        match case {
            "unknown-version" => known[1].version = "sha256:unverified-program".into(),
            "coverage-gap" => snapshot.observation.slot = 123,
            "missing-evidence" => snapshot
                .accounts
                .retain(|(address, _)| *address != Pubkey::new_from_array([6; 32])),
            "malformed-state" => snapshot
                .accounts
                .iter_mut()
                .find(|(address, _)| *address == key(SOURCE))
                .unwrap()
                .1
                .data
                .truncate(4),
            _ => unreachable!(),
        }
        let record = journal
            .ingest(snapshot.clone(), &Catalog::new(known).unwrap())
            .await
            .unwrap();
        match (&record.projection, case) {
            (Projection::Unsupported { reason }, "unknown-version" | "coverage-gap")
            | (Projection::Incomplete { reason }, "missing-evidence")
            | (Projection::Invalid { reason }, "malformed-state") => assert!(!reason.is_empty()),
            _ => panic!(
                "wrong failure classification for {case}: {:?}",
                record.projection
            ),
        }
        if case == "unknown-version" {
            assert_eq!(record.versions[0].version, "sha256:unverified-program");
        }
        if case == "coverage-gap" {
            assert!(record.versions.is_empty());
        }
        assert_raw_retained(&record, &snapshot);
        assert_eq!(journal.get(&record.id).await.unwrap(), Some(record.clone()));
        assert_eq!(journal.replay(&record.id).await.unwrap(), record);
    }
}

#[tokio::test]
async fn explicit_spl_closure_requires_evidence_and_a_verified_version() {
    for case in ["omitted", "unknown-version", "closed"] {
        let journal = Journal::open("sqlite::memory:").await.unwrap();
        let mut snapshot = spl_snapshot();
        let source = Pubkey::new_from_array([1; 32]);
        let mut known = versions();
        if case == "omitted" {
            snapshot.accounts.retain(|(address, _)| *address != source);
        } else {
            // Test mutation of observed absence, not an executed native close.
            snapshot
                .accounts
                .iter_mut()
                .find(|(address, _)| *address == source)
                .unwrap()
                .1 = solana_account::Account::default();
        }
        if case == "unknown-version" {
            known[0].version = "sha256:unverified-program".into();
        }
        let record = journal
            .ingest(snapshot.clone(), &Catalog::new(known).unwrap())
            .await
            .unwrap();
        match case {
            "closed" => assert!(compiled(&record).is_empty()),
            "omitted" => assert!(matches!(record.projection, Projection::Incomplete { .. })),
            "unknown-version" => {
                assert!(matches!(record.projection, Projection::Unsupported { .. }))
            }
            _ => unreachable!(),
        }
        assert_raw_retained(&record, &snapshot);
        assert_eq!(journal.replay(&record.id).await.unwrap(), record);
    }
}

struct TemporaryDatabase(PathBuf);

impl TemporaryDatabase {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = PathBuf::from(format!(
            "/tmp/catalyst-snapshots-{}-{nonce}.sqlite",
            std::process::id()
        ));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        Self(path)
    }
    fn url(&self) -> String {
        format!("sqlite://{}", self.0.display())
    }
}

impl Drop for TemporaryDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let path = PathBuf::from(format!("{}{suffix}", self.0.display()));
            if let Err(error) = std::fs::remove_file(path) {
                assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
            }
        }
    }
}

#[tokio::test]
async fn reopening_sqlite_replays_historical_native_revoke_without_erasing_technical_authority() {
    let database = TemporaryDatabase::new();
    let records = {
        let journal = Journal::open(&database.url()).await.unwrap();
        let mut records = Vec::new();
        for (name, phase) in [
            ("owner_pull_60", "after"),
            ("revoke_cancelled", "before"),
            ("revoke_cancelled", "after"),
        ] {
            let snapshot = subscription_snapshot(name, phase);
            let record = journal.ingest(snapshot.clone(), &catalog()).await.unwrap();
            assert_raw_retained(&record, &snapshot);
            records.push(record);
        }
        records
    };
    let reopened = Journal::open(&database.url()).await.unwrap();
    for record in &records {
        assert_eq!(
            reopened.get(&record.id).await.unwrap().as_ref(),
            Some(record)
        );
        assert_eq!(&reopened.replay(&record.id).await.unwrap(), record);
    }
    assert_golden(
        &records[0],
        include_str!("fixtures/subscriptions-plan-arm.json"),
    );
    let before = &records[1];
    let after = &records[2];
    assert_ne!(before.id, after.id);
    assert_eq!(before.snapshot.observation, after.snapshot.observation);
    assert_eq!(before.snapshot.observation.slot, 122);
    assert_eq!(before.snapshot.observation.unix_timestamp, 1_800_003_600);
    assert_eq!(compiled(before).len(), 3);
    assert_eq!(compiled(after).len(), 2);
    let grant = format!(
        "{}:{DELEGATION}:100:spend",
        subscriptions::DelegationAdapter.protocol().program_id
    );
    assert!(
        compiled(before)
            .iter()
            .any(|authorization| authorization.id == grant)
    );
    assert!(
        compiled(after)
            .iter()
            .all(|authorization| authorization.id != grant)
    );
    let mut survivors = compiled(before)
        .iter()
        .filter(|authorization| authorization.id != grant)
        .cloned()
        .collect::<Vec<_>>();
    for authorization in &mut survivors {
        authorization.evidence = compiled(after)
            .iter()
            .find(|a| a.id == authorization.id)
            .unwrap()
            .evidence
            .clone();
    }
    assert_eq!(survivors, compiled(after));
    let removed = after
        .snapshot
        .accounts
        .iter()
        .find(|(address, _)| *address == key(DELEGATION))
        .unwrap();
    assert!(removed.1.data.is_empty());
    assert_eq!(removed.1.lamports, 0);
    assert_eq!(removed.1.owner, Pubkey::default());
}

#[tokio::test]
async fn maximum_u64_fields_survive_json_sqlite_and_replay_without_fabricated_coverage() {
    let database = TemporaryDatabase::new();
    let mut snapshot = spl_snapshot();
    snapshot.observation.slot = u64::MAX;
    let raw = &mut snapshot.accounts[0].1;
    raw.lamports = u64::MAX;
    raw.rent_epoch = u64::MAX;
    let mut token = TokenAccount::unpack(&raw.data).unwrap();
    token.amount = u64::MAX;
    token.delegated_amount = u64::MAX;
    TokenAccount::pack(token, &mut raw.data).unwrap();
    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(json.contains("18446744073709551615"));
    assert_eq!(serde_json::from_str::<Snapshot>(&json).unwrap(), snapshot);
    let record = {
        let journal = Journal::open(&database.url()).await.unwrap();
        journal.ingest(snapshot.clone(), &catalog()).await.unwrap()
    };
    assert!(matches!(record.projection, Projection::Unsupported { .. }));
    assert!(record.versions.is_empty());
    let reopened = Journal::open(&database.url()).await.unwrap();
    let loaded = reopened.get(&record.id).await.unwrap().unwrap();
    assert_raw_retained(&loaded, &snapshot);
    let raw = &loaded
        .snapshot
        .accounts
        .iter()
        .find(|(address, _)| *address == Pubkey::new_from_array([1; 32]))
        .unwrap()
        .1;
    assert_eq!((raw.lamports, raw.rent_epoch), (u64::MAX, u64::MAX));
    let native = TokenAccount::unpack(&raw.data).unwrap();
    assert_eq!(
        (native.amount, native.delegated_amount),
        (u64::MAX, u64::MAX)
    );
    assert_eq!(loaded.snapshot.observation.slot, u64::MAX);
    assert_eq!(reopened.replay(&record.id).await.unwrap(), record);
}

async fn get(app: &Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        },
    )
}

#[tokio::test]
async fn router_selects_principals_subjects_and_resources_with_explicit_partial_provenance() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let record = journal
        .ingest(subscription_snapshot("owner_pull_60", "after"), &catalog())
        .await
        .unwrap();
    let app = router(journal);
    let all = compiled(&record);
    let grant = all
        .iter()
        .find(|a| a.id.contains(DELEGATION))
        .unwrap()
        .clone();
    let technical = all
        .iter()
        .find(|a| a.id.contains(":technical:"))
        .unwrap()
        .clone();
    let plan = all
        .iter()
        .find(|a| a.id.contains(":modify-authority"))
        .unwrap()
        .clone();
    for (address, mut expected) in [
        (
            Pubkey::new_from_array([2; 32]),
            vec![grant.clone(), plan.clone()],
        ),
        (Pubkey::new_from_array([6; 32]), vec![grant.clone()]),
        (key(AUTHORITY), vec![technical.clone()]),
        (
            Pubkey::new_from_array([1; 32]),
            vec![grant.clone(), technical.clone()],
        ),
        (key(SOURCE), vec![grant, technical]),
        (key(PLAN), vec![plan]),
        (Pubkey::new_from_array([99; 32]), vec![]),
    ] {
        let (status, view) = get(
            &app,
            &format!("/snapshots/{}/authorizations/{address}", record.id),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(view["state_version"], record.id);
        assert_eq!(view["coverage"]["complete"], false);
        assert_eq!(
            view["coverage"]["accounts"],
            serde_json::to_value(
                record
                    .snapshot
                    .accounts
                    .iter()
                    .map(|(address, _)| address)
                    .collect::<Vec<_>>()
            )
            .unwrap()
        );
        assert_eq!(
            view["coverage"]["targets"],
            serde_json::to_value(&record.snapshot.targets).unwrap()
        );
        assert_eq!(
            view["versions"],
            serde_json::to_value(&record.versions).unwrap()
        );
        assert_eq!(
            view["observation"],
            serde_json::to_value(&record.snapshot.observation).unwrap()
        );
        assert_eq!(view["sdk_revision"], SDK_REVISION);
        assert_eq!(view["runtime_version"], RUNTIME_VERSION);
        assert_eq!(view["arm_schema_version"], arm::SCHEMA_VERSION);
        let mut actual: Vec<Authorization> =
            serde_json::from_value(view["projection"]["authorizations"].clone()).unwrap();
        assert_eq!(view["projection"]["status"], "compiled");
        actual.sort_by(|a, b| a.id.cmp(&b.id));
        expected.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(actual, expected, "address {address}");
    }
    let (status, raw) = get(&app, &format!("/snapshots/{}", record.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(serde_json::from_value::<Record>(raw).unwrap(), record);
    assert_eq!(
        get(
            &app,
            &format!("/snapshots/{}/authorizations/not-a-pubkey", record.id)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(&app, &format!("/snapshots/missing/authorizations/{SOURCE}"))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, "/snapshots/missing").await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn finalized_origin_cannot_turn_fixture_deployments_into_live_support() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let mut snapshot = spl_snapshot();
    snapshot.observation.origin = Origin::Finalized;
    let record = journal.ingest(snapshot.clone(), &catalog()).await.unwrap();
    assert!(matches!(record.projection, Projection::Unsupported { .. }));
    assert_raw_retained(&record, &snapshot);
    assert_eq!(record.versions.len(), 1);
    assert_eq!(record.versions[0].deployment, spl::DEPLOYMENT);
    assert_eq!(journal.replay(&record.id).await.unwrap(), record);
    let app = router(journal);
    let (status, view) = get(
        &app,
        &format!(
            "/snapshots/{}/authorizations/{}",
            record.id,
            Pubkey::new_from_array([2; 32])
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["projection"]["status"], "unsupported");
    assert_eq!(view["coverage"]["complete"], false);
    assert_eq!(view["observation"]["origin"], "finalized");
    assert_eq!(
        view["versions"],
        serde_json::to_value(record.versions).unwrap()
    );
}

#[tokio::test]
async fn tampered_sqlite_rows_cannot_relabel_raw_evidence_or_claim_complete_coverage() {
    let database = TemporaryDatabase::new();
    let journal = Journal::open(&database.url()).await.unwrap();
    let record = journal.ingest(spl_snapshot(), &catalog()).await.unwrap();
    let independent = sqlx::SqlitePool::connect(&database.url()).await.unwrap();
    for case in ["raw-account", "record-id", "coverage"] {
        let mut tampered = record.clone();
        match case {
            "raw-account" => tampered.snapshot.accounts[0].1.lamports += 1,
            "record-id" => tampered.id = "different-snapshot-id".into(),
            "coverage" => tampered.coverage_complete = true,
            _ => unreachable!(),
        }
        sqlx::query("UPDATE authority_snapshots SET record = ? WHERE id = ?")
            .bind(serde_json::to_string(&tampered).unwrap())
            .bind(&record.id)
            .execute(&independent)
            .await
            .unwrap();
        assert!(
            matches!(journal.get(&record.id).await, Err(Error::ReplayMismatch)),
            "{case}"
        );
        assert!(
            matches!(journal.replay(&record.id).await, Err(Error::ReplayMismatch)),
            "{case}"
        );
    }
    let mut tampered = record.clone();
    tampered.projection = Projection::Compiled {
        authorizations: vec![],
    };
    sqlx::query("UPDATE authority_snapshots SET record = ? WHERE id = ?")
        .bind(serde_json::to_string(&tampered).unwrap())
        .bind(&record.id)
        .execute(&independent)
        .await
        .unwrap();
    assert!(matches!(
        journal.get(&record.id).await,
        Err(Error::ReplayMismatch)
    ));
    assert!(matches!(
        journal.replay(&record.id).await,
        Err(Error::ReplayMismatch)
    ));
    sqlx::query("UPDATE authority_snapshots SET record = ? WHERE id = ?")
        .bind(serde_json::to_string(&record).unwrap())
        .bind(&record.id)
        .execute(&independent)
        .await
        .unwrap();
    assert_eq!(journal.replay(&record.id).await.unwrap(), record);
    independent.close().await;
}

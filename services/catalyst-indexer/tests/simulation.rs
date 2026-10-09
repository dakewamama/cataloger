use arm::AuthorizationChange;
use axum::{Json, Router, routing::post};
use base64::{Engine, engine::general_purpose::STANDARD};
use cataloger::{AdapterRef, Catalog, ProgramVersion, Provenance, SchemaSource};
use catalyst_indexer::{Error, Journal, Origin, Record, SDK_REVISION, Snapshot, Target, rpc};
use catalyst_sdk::{Adapter, Error as AdapterError, spl};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use solana_account::Account;
use solana_account_decoder_client_types::UiAccount;
use solana_clock::Clock;
use solana_commitment_config::CommitmentConfig;
use solana_loader_v3_interface::get_program_data_address;
use solana_pubkey::Pubkey;
use solana_rpc_client::{
    mock_sender::MockSender, nonblocking::rpc_client::RpcClient, rpc_client::RpcClientConfig,
};
use solana_rpc_client_api::request::RpcRequest;
use solana_sdk_ids::sysvar;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

const SLOT: u64 = 454_547_887;
const TIMESTAMP: i64 = 1_791_463_325;
const AFTER: &[u8] = include_bytes!("fixtures/spl-revoke-after.bin");

fn source() -> Pubkey {
    Pubkey::new_from_array([1; 32])
}

fn owner() -> Pubkey {
    Pubkey::new_from_array([4; 32])
}

fn metadata() -> Value {
    serde_json::from_str(include_str!("fixtures/mainnet-spl-capture.json")).unwrap()
}

fn snapshot() -> Snapshot {
    let mut snapshot: Snapshot =
        serde_json::from_str(include_str!("fixtures/spl-snapshot.json")).unwrap();
    let capture = metadata();
    let response: Value =
        serde_json::from_str(include_str!("fixtures/mainnet-spl-response.json")).unwrap();
    // Native fixture accounts plus captured code/Clock exercise the boundary, not a live grant.
    for (key, value) in capture["keys"]
        .as_array()
        .unwrap()
        .iter()
        .zip(response["result"]["value"].as_array().unwrap())
    {
        if value.is_null() {
            continue;
        }
        let key = key.as_str().unwrap().parse().unwrap();
        let account: UiAccount = serde_json::from_value(value.clone()).unwrap();
        snapshot.accounts.push((key, account.to_account().unwrap()));
    }
    snapshot.observation.cluster = capture["cluster"].as_str().unwrap().into();
    snapshot.observation.bank = format!("rpc:finalized:{SLOT}");
    snapshot.observation.slot = SLOT;
    snapshot.observation.unix_timestamp = TIMESTAMP;
    snapshot.observation.origin = Origin::Finalized;
    snapshot
}

fn version() -> ProgramVersion {
    let program = spl::DelegateAdapter.protocol().program_id;
    ProgramVersion {
        cluster: metadata()["cluster"].as_str().unwrap().into(),
        program_id: program,
        deployment: spl::LIVE_DEPLOYMENT.into(),
        version: spl::PROGRAM_VERSION.into(),
        supported_from_slot: SLOT,
        supported_until_slot_exclusive: SLOT + 2,
        schema: SchemaSource {
            uri: "https://docs.rs/spl-token-interface/3.0.0".into(),
            revision: "3.0.0".into(),
        },
        adapter: AdapterRef {
            protocol: "spl-token".into(),
            version: "0.1".into(),
            source: format!(
                "https://github.com/dakewamama/catalyst-sdk/blob/{SDK_REVISION}/src/spl.rs"
            ),
        },
        provenance: Provenance {
            source: "fixture:native-revoke-and-captured-program".into(),
            revision: "mollusk-svm:0.15.1".into(),
            evidence_reference: "fixture:spl-revoke-after.bin".into(),
        },
    }
}

fn catalog() -> Catalog {
    Catalog::new(vec![version()]).unwrap()
}

fn account(snapshot: &mut Snapshot, key: Pubkey) -> &mut Account {
    &mut snapshot
        .accounts
        .iter_mut()
        .find(|(address, _)| *address == key)
        .unwrap()
        .1
}

fn ui(account: &Account) -> Value {
    json!({
        "lamports": account.lamports,
        "owner": account.owner.to_string(),
        "executable": account.executable,
        "rentEpoch": account.rent_epoch,
        "space": account.data.len(),
        "data": [STANDARD.encode(&account.data), "base64"]
    })
}

fn response(snapshot: &Snapshot) -> Value {
    let account = |key| {
        &snapshot
            .accounts
            .iter()
            .find(|(address, _)| *address == key)
            .unwrap()
            .1
    };
    let mut source = account(source()).clone();
    source.data = AFTER.to_vec();
    json!({
        "context": {"slot": snapshot.observation.slot},
        "value": {
            "err": null,
            "logs": ["Program log: Instruction: Revoke"],
            "accounts": [ui(&source), ui(account(owner())), ui(account(sysvar::clock::id()))],
            "unitsConsumed": null,
            "replacementBlockhash": {"blockhash": "11111111111111111111111111111111", "lastValidBlockHeight": 100}
        }
    })
}

fn client(reply: Value) -> RpcClient {
    RpcClient::new_mock_with_mocks(
        "succeeds".into(),
        [
            (RpcRequest::GetGenesisHash, metadata()["cluster"].clone()),
            (RpcRequest::SimulateTransaction, reply),
        ]
        .into_iter()
        .collect(),
    )
}

async fn record(journal: &Journal) -> Record {
    journal.ingest(snapshot(), &catalog()).await.unwrap()
}

#[tokio::test]
async fn native_revoke_golden_agrees_without_advancing_the_finalized_journal() {
    assert_eq!(
        format!("{:x}", Sha256::digest(AFTER)),
        "e1e671510fa14513936cbe2a11505c6c53504fd91422ae7a1249da5aebb0d129"
    );
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let record = record(&journal).await;
    let expected_response = response(&record.snapshot);
    let result = rpc::simulate_revoke(
        &client(expected_response.clone()),
        &journal,
        &record.id,
        source(),
    )
    .await
    .unwrap();
    assert_eq!(result.state_version, record.id);
    assert_eq!(result.observation, record.snapshot.observation);
    assert_eq!(result.versions, record.versions);
    assert_eq!(result.sdk_revision, SDK_REVISION);
    assert!(!result.coverage.complete);
    assert_eq!(result.coverage.targets, record.snapshot.targets);
    assert!(
        matches!(&result.changes[..], [AuthorizationChange::Removed { authorization }]
        if authorization.id.ends_with(":delegate:spend"))
    );
    assert_eq!(result.transaction.message.account_keys[0], owner());
    assert!(
        result
            .transaction
            .signatures
            .iter()
            .all(|signature| *signature == Default::default())
    );
    assert_eq!(result.transaction.message.instructions.len(), 1);
    assert_eq!(result.transaction.message.instructions[0].data, [5]);
    assert_eq!(result.response.context.slot, SLOT);
    assert_eq!(
        serde_json::to_value(&result.response).unwrap()["value"]["accounts"],
        expected_response["value"]["accounts"]
    );
    assert_eq!(
        result.id,
        format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(
                    &result.state_version,
                    &result.transaction,
                    &result.config,
                    &result.response
                ))
                .unwrap()
            )
        )
    );
    let repeated = rpc::simulate_revoke(&client(expected_response), &journal, &record.id, source())
        .await
        .unwrap();
    assert_eq!(result.id, repeated.id);
    assert_eq!(journal.replay(&record.id).await.unwrap(), record);
    assert_eq!(
        journal
            .latest(&record.snapshot.scope_id().unwrap())
            .await
            .unwrap(),
        Some(record)
    );
}

#[tokio::test]
async fn agave_rpc_capture_agrees_with_the_native_transition() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/spl-revoke-rpc.json")).unwrap();
    let mut before: Snapshot = serde_json::from_value(fixture["before"].clone()).unwrap();
    let programdata = get_program_data_address(&spl::DelegateAdapter.protocol().program_id);
    account(&mut before, programdata).data = account(&mut snapshot(), programdata).data.clone();
    let mut version = version();
    version.cluster = before.observation.cluster.clone();
    version.supported_from_slot = before.observation.slot;
    version.supported_until_slot_exclusive = before.observation.slot + 1;
    version.provenance.source = "fixture:agave-rpc:3.1.10".into();
    version.provenance.revision = "agave:3.1.10".into();
    version.provenance.evidence_reference = "fixture:spl-revoke-rpc.json".into();
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let record = journal
        .ingest(before, &Catalog::new(vec![version]).unwrap())
        .await
        .unwrap();
    let client = RpcClient::new_mock_with_mocks(
        "succeeds".into(),
        [
            (
                RpcRequest::GetGenesisHash,
                json!(record.snapshot.observation.cluster),
            ),
            (RpcRequest::SimulateTransaction, fixture["response"].clone()),
        ]
        .into_iter()
        .collect(),
    );
    let result = rpc::simulate_revoke(&client, &journal, &record.id, source())
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&result.response).unwrap(),
        fixture["response"]
    );
    assert!(matches!(
        &result.changes[..],
        [AuthorizationChange::Removed { .. }]
    ));
    assert_eq!(
        STANDARD
            .decode(
                fixture["response"]["value"]["accounts"][0]["data"][0]
                    .as_str()
                    .unwrap()
            )
            .unwrap(),
        AFTER
    );
    assert!(
        !result
            .transaction
            .message
            .account_keys
            .contains(&sysvar::clock::id())
    );
    assert_eq!(journal.replay(&record.id).await.unwrap(), record);
}

#[tokio::test]
async fn simulation_failure_retains_native_error_and_null_accounts() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let record = record(&journal).await;
    let mut reply = response(&record.snapshot);
    reply["value"]["err"] = json!({"InstructionError": [0, {"Custom": 1}]});
    reply["value"]["accounts"] = json!([null, null, null]);
    let result = rpc::simulate_revoke(&client(reply.clone()), &journal, &record.id, source()).await;
    match result {
        Err(Error::SimulationFailed(response)) => {
            assert_eq!(
                serde_json::to_value(response.value.err).unwrap(),
                reply["value"]["err"]
            );
            assert_eq!(response.value.accounts.unwrap(), vec![None, None, None]);
        }
        _ => panic!("expected native failure"),
    }
    assert_eq!(journal.replay(&record.id).await.unwrap(), record);
}

#[tokio::test]
async fn newer_and_older_simulation_banks_fail_despite_the_slot_floor() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let record = record(&journal).await;
    for slot in [SLOT - 1, SLOT + 1] {
        let mut reply = response(&record.snapshot);
        reply["context"]["slot"] = json!(slot);
        assert!(matches!(
            rpc::simulate_revoke(&client(reply), &journal, &record.id, source()).await,
            Err(Error::InvalidObservation(
                "simulation/context slot mismatch"
            ))
        ));
    }
}

#[tokio::test]
async fn missing_truncated_and_undecodable_simulated_accounts_fail_closed() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let record = record(&journal).await;
    for case in [
        "omitted", "short", "long", "null", "base64", "space", "owner",
    ] {
        let mut reply = response(&record.snapshot);
        let accounts = &mut reply["value"]["accounts"];
        let reason = match case {
            "omitted" => {
                *accounts = Value::Null;
                "missing simulated accounts"
            }
            "short" => {
                accounts.as_array_mut().unwrap().pop();
                "missing simulated accounts"
            }
            "long" => {
                accounts.as_array_mut().unwrap().push(Value::Null);
                "missing simulated accounts"
            }
            "null" => {
                accounts[0] = Value::Null;
                "missing simulated source"
            }
            "base64" => {
                accounts[0]["data"] = json!(["!", "base64"]);
                "undecodable RPC account"
            }
            "space" => {
                accounts[0]["space"] = json!(1);
                "truncated RPC account"
            }
            "owner" => {
                accounts[0]["owner"] = json!("invalid");
                "undecodable RPC account"
            }
            _ => unreachable!(),
        };
        assert!(
            matches!(
                rpc::simulate_revoke(&client(reply), &journal, &record.id, source()).await,
                Err(Error::InvalidObservation(actual)) if actual == reason
            ),
            "{case}"
        );
    }
}

#[tokio::test]
async fn the_simulated_clock_must_match_the_retained_bank() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let record = record(&journal).await;
    for case in ["timestamp", "slot", "owner", "null"] {
        let mut changed = record.snapshot.clone();
        let clock_account = account(&mut changed, sysvar::clock::id());
        let mut clock: Clock = bincode::deserialize(&clock_account.data).unwrap();
        match case {
            "timestamp" => clock.unix_timestamp += 1,
            "slot" => clock.slot += 1,
            "owner" => clock_account.owner = owner(),
            "null" => (),
            _ => unreachable!(),
        }
        clock_account.data = bincode::serialize(&clock).unwrap();
        let mut reply = response(&changed);
        if case == "null" {
            reply["value"]["accounts"][2] = Value::Null;
        }
        assert!(
            matches!(
                rpc::simulate_revoke(&client(reply), &journal, &record.id, source()).await,
                Err(Error::InvalidObservation("simulation clock mismatch"))
                    | Err(Error::Adapter(AdapterError::InvalidState(_)))
            ),
            "{case}"
        );
    }
}

#[tokio::test]
async fn successful_rpc_status_cannot_hide_an_unchanged_delegate_or_owner_change() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let record = record(&journal).await;
    for case in ["unchanged", "owner", "amount", "mint"] {
        let mut data = AFTER.to_vec();
        match case {
            "unchanged" => data = include_bytes!("fixtures/spl-delegate.bin").to_vec(),
            "owner" => data[32] ^= 1,
            "amount" => data[64] ^= 1,
            "mint" => data[0] ^= 1,
            _ => unreachable!(),
        }
        let mut reply = response(&record.snapshot);
        reply["value"]["accounts"][0]["data"] = json!([STANDARD.encode(data), "base64"]);
        assert!(
            matches!(
                rpc::simulate_revoke(&client(reply), &journal, &record.id, source()).await,
                Err(Error::Adapter(AdapterError::DiffMismatch))
            ),
            "{case}"
        );
    }
}

#[tokio::test]
async fn fixtures_other_scopes_and_unknown_code_reject_before_rpc() {
    for case in [
        "fixture",
        "scope",
        "code",
        "missing",
        "frozen",
        "controlled-owner",
    ] {
        let journal = Journal::open("sqlite::memory:").await.unwrap();
        let mut snapshot = snapshot();
        match case {
            "fixture" => snapshot.observation.origin = Origin::Fixture,
            "scope" => snapshot
                .targets
                .push(Target::SplDelegate { source: owner() }),
            "code" => {
                let program = spl::DelegateAdapter.protocol().program_id;
                *account(&mut snapshot, get_program_data_address(&program))
                    .data
                    .last_mut()
                    .unwrap() ^= 1;
            }
            "missing" => snapshot
                .accounts
                .retain(|(key, _)| *key != sysvar::clock::id()),
            "frozen" => account(&mut snapshot, source()).data[108] = 2,
            "controlled-owner" => {
                account(&mut snapshot, owner()).owner = spl::DelegateAdapter.protocol().program_id
            }
            _ => unreachable!(),
        }
        let record = journal.ingest(snapshot, &catalog()).await.unwrap();
        let no_rpc = RpcClient::new_mock("fails".into());
        let result = rpc::simulate_revoke(&no_rpc, &journal, &record.id, source()).await;
        assert!(
            matches!(
                result,
                Err(Error::InvalidObservation("finalized evidence required"))
                    | Err(Error::Adapter(
                        AdapterError::UnsupportedOperation
                            | AdapterError::UnsupportedVersion
                            | AdapterError::InsufficientEvidence
                    ))
            ),
            "{case}"
        );
    }
}

#[tokio::test]
async fn superseded_or_ambiguous_observations_cannot_authorize_simulation() {
    for ambiguous in [true, false] {
        let journal = Journal::open("sqlite::memory:").await.unwrap();
        let record = record(&journal).await;
        let mut next = record.snapshot.clone();
        if ambiguous {
            account(&mut next, owner()).lamports += 1;
        } else {
            next.observation.slot += 1;
            next.observation.bank = format!("rpc:finalized:{}", next.observation.slot);
            let clock_account = account(&mut next, sysvar::clock::id());
            let mut clock: Clock = bincode::deserialize(&clock_account.data).unwrap();
            clock.slot += 1;
            clock_account.data = bincode::serialize(&clock).unwrap();
        }
        journal.ingest(next, &catalog()).await.unwrap();
        let result = rpc::simulate_revoke(
            &RpcClient::new_mock("fails".into()),
            &journal,
            &record.id,
            source(),
        )
        .await;
        assert!(matches!(
            result,
            Err(Error::AmbiguousScope) | Err(Error::Conflict)
        ));
    }
}

#[tokio::test]
async fn cluster_mismatch_unfinalized_commitment_and_provider_error_reject() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let record = record(&journal).await;
    for commitment in [CommitmentConfig::confirmed(), CommitmentConfig::processed()] {
        let client = RpcClient::new_sender(
            MockSender::new("fails"),
            RpcClientConfig::with_commitment(commitment),
        );
        assert!(matches!(
            rpc::simulate_revoke(&client, &journal, &record.id, source()).await,
            Err(Error::InvalidObservation("finalized evidence required"))
        ));
    }
    let wrong = RpcClient::new_mock_with_mocks(
        "succeeds".into(),
        [(
            RpcRequest::GetGenesisHash,
            json!("11111111111111111111111111111111"),
        )]
        .into_iter()
        .collect(),
    );
    assert!(matches!(
        rpc::simulate_revoke(&wrong, &journal, &record.id, source()).await,
        Err(Error::InvalidObservation("cluster genesis mismatch"))
    ));
    assert!(matches!(
        rpc::simulate_revoke(
            &RpcClient::new_mock("fails".into()),
            &journal,
            &record.id,
            source()
        )
        .await,
        Err(Error::Rpc(_))
    ));
}

#[tokio::test]
async fn max_u64_source_lamports_survive_simulation_evidence_serialization() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let mut snapshot = snapshot();
    account(&mut snapshot, source()).lamports = u64::MAX;
    let record = journal.ingest(snapshot, &catalog()).await.unwrap();
    let result = rpc::simulate_revoke(
        &client(response(&record.snapshot)),
        &journal,
        &record.id,
        source(),
    )
    .await
    .unwrap();
    let value = serde_json::to_value(&result).unwrap();
    assert_eq!(
        value["response"]["value"]["accounts"][0]["lamports"].as_u64(),
        Some(u64::MAX)
    );
    assert!(!value["coverage"]["complete"].as_bool().unwrap());
}

#[tokio::test]
async fn a_checkpoint_advancing_during_rpc_invalidates_the_simulation() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let record = record(&journal).await;
    let reply = response(&record.snapshot);
    let mut next = record.snapshot.clone();
    next.observation.slot += 1;
    next.observation.bank = format!("rpc:finalized:{}", next.observation.slot);
    let clock_account = account(&mut next, sysvar::clock::id());
    let mut clock: Clock = bincode::deserialize(&clock_account.data).unwrap();
    clock.slot += 1;
    clock_account.data = bincode::serialize(&clock).unwrap();
    let writer = journal.clone();
    let app = Router::new().route(
        "/",
        post(move |Json(request): Json<Value>| {
            let writer = writer.clone();
            let next = next.clone();
            let reply = reply.clone();
            async move {
                let result = match request["method"].as_str().unwrap() {
                    "getGenesisHash" => metadata()["cluster"].clone(),
                    "simulateTransaction" => {
                        writer.ingest(next, &catalog()).await.unwrap();
                        reply
                    }
                    other => panic!("unexpected RPC: {other}"),
                };
                Json(json!({"jsonrpc": "2.0", "id": request["id"], "result": result}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = RpcClient::new_with_timeout_and_commitment(
        format!("http://{}/", listener.local_addr().unwrap()),
        Duration::from_secs(5),
        CommitmentConfig::finalized(),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let result = rpc::simulate_revoke(&client, &journal, &record.id, source()).await;
    server.abort();
    assert!(matches!(result, Err(Error::Conflict)));
    let current = journal
        .latest(&record.snapshot.scope_id().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.snapshot.observation.slot, SLOT + 1);
    assert_ne!(current.id, record.id);
}

#[tokio::test]
async fn cli_uses_the_native_unsigned_rpc_contract_without_submitting_a_transaction() {
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let recorded = requests.clone();
    let snapshot = snapshot();
    let reply = response(&snapshot);
    let app = Router::new().route(
        "/",
        post(move |Json(request): Json<Value>| {
            let requests = recorded.clone();
            let reply = reply.clone();
            async move {
                requests.lock().unwrap().push(request.clone());
                let result = match request["method"].as_str().unwrap() {
                    "getGenesisHash" => metadata()["cluster"].clone(),
                    "simulateTransaction" => reply,
                    other => panic!("unexpected RPC: {other}"),
                };
                Json(json!({"jsonrpc": "2.0", "id": request["id"], "result": result}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let database = format!(
        "sqlite:///tmp/catalyst-simulation-{}-{address}.db",
        std::process::id()
    );
    let journal = Journal::open(&database).await.unwrap();
    let record = journal.ingest(snapshot, &catalog()).await.unwrap();
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_authorization-api"))
            .args(["simulate-revoke", &record.id, &source().to_string()])
            .env("DATABASE_URL", &database)
            .env("RPC_URL", format!("http://{address}/"))
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    server.abort();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["state_version"], record.id);
    assert_eq!(result["changes"][0]["kind"], "removed");
    assert_eq!(
        journal
            .latest(&record.snapshot.scope_id().unwrap())
            .await
            .unwrap(),
        Some(record)
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["method"], "getGenesisHash");
    assert_eq!(requests[1]["method"], "simulateTransaction");
    let request = &requests[1]["params"];
    let transaction: solana_transaction::Transaction =
        bincode::deserialize(&STANDARD.decode(request[0].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(transaction).unwrap(),
        result["transaction"]
    );
    assert_eq!(request[1]["commitment"], "finalized");
    assert_eq!(request[1]["minContextSlot"], SLOT);
    assert_eq!(request[1]["sigVerify"], false);
    assert_eq!(request[1]["replaceRecentBlockhash"], true);
    assert_eq!(request[1]["encoding"], "base64");
    assert_eq!(request[1]["accounts"]["encoding"], "base64");
    assert_eq!(
        request[1]["accounts"]["addresses"],
        json!([
            source().to_string(),
            owner().to_string(),
            sysvar::clock::id().to_string()
        ])
    );
}

use axum::{Json, Router, routing::post};
use cataloger::{AdapterRef, Catalog, ProgramVersion, Provenance, SchemaSource};
use catalyst_indexer::{Error, Journal, Origin, Projection, SDK_REVISION, Snapshot, Target, rpc};
use catalyst_sdk::{Adapter, spl, subscriptions};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use solana_account::Account;
use solana_account_decoder_client_types::UiAccount;
use solana_clock::Clock;
use solana_commitment_config::CommitmentConfig;
use solana_loader_v3_interface::{get_program_data_address, state::UpgradeableLoaderState};
use solana_pubkey::Pubkey;
use solana_rpc_client::{
    mock_sender::MockSender, nonblocking::rpc_client::RpcClient, rpc_client::RpcClientConfig,
};
use solana_rpc_client_api::request::RpcRequest;
use solana_sdk_ids::{bpf_loader_upgradeable, sysvar};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

mod support;

const SLOT: u64 = 454_547_887;
const DEPLOYED: u64 = 419_472_000;
const TIMESTAMP: i64 = 1_791_463_325;
const RESPONSE_HASH: &str = "68ffbb2b01685f3bdadf174780681fb4dac9355b58282a09d145ff8f5acd1b0e";
const PROGRAM_HASH: &str = "8190d3f7ceb6cb7a7a8d8924bff89f9f611e15ce1f806f2b6237f3311a98f697";
const DEPLOYMENT: &str = "solana:loader-v3:3gvYRKWyXRR9xKWe1ZjPhLY5ZJRN7KDB4rFZFGoJfFk2:419472000";
const RAW: &[u8] = include_bytes!("fixtures/mainnet-spl-response.json");

fn metadata() -> Value {
    serde_json::from_str(include_str!("fixtures/mainnet-spl-capture.json")).unwrap()
}

fn scope() -> rpc::Scope {
    let metadata = metadata();
    rpc::Scope {
        cluster: metadata["cluster"].as_str().unwrap().into(),
        accounts: vec![],
        targets: vec![Target::SplDelegate {
            source: metadata["source"].as_str().unwrap().parse().unwrap(),
        }],
        min_context_slot: Some(SLOT),
    }
}

fn source() -> Pubkey {
    metadata()["source"].as_str().unwrap().parse().unwrap()
}

fn program() -> Pubkey {
    spl::DelegateAdapter.protocol().program_id
}

fn programdata() -> Pubkey {
    get_program_data_address(&program())
}

fn captured() -> BTreeMap<Pubkey, Value> {
    let metadata = metadata();
    let response: Value = serde_json::from_slice(RAW).unwrap();
    let keys = metadata["keys"].as_array().unwrap();
    let values = response["result"]["value"].as_array().unwrap();
    assert_eq!(keys.len(), values.len());
    keys.iter()
        .zip(values)
        .map(|(key, value)| (key.as_str().unwrap().parse().unwrap(), value.clone()))
        .collect()
}

fn response(scope: &rpc::Scope) -> Value {
    let mut response: Value = serde_json::from_slice(RAW).unwrap();
    let captured = captured();
    // RPC values are positional in the recorded request, not in Scope.accounts.
    response["result"]["value"] = scope
        .accounts()
        .unwrap()
        .iter()
        .map(|key| captured.get(key).unwrap().clone())
        .collect();
    response["result"].take()
}

fn client(response: Value) -> RpcClient {
    RpcClient::new_mock_with_mocks(
        "succeeds".into(),
        [
            (RpcRequest::GetGenesisHash, metadata()["cluster"].clone()),
            (RpcRequest::GetMultipleAccounts, response),
        ]
        .into_iter()
        .collect(),
    )
}

async fn snapshot() -> Snapshot {
    rpc::observe(&client(response(&scope())), scope())
        .await
        .unwrap()
}

fn version() -> ProgramVersion {
    let metadata = metadata();
    ProgramVersion {
        cluster: metadata["cluster"].as_str().unwrap().into(),
        program_id: program(),
        deployment: DEPLOYMENT.into(),
        version: format!("sha256:{PROGRAM_HASH}"),
        executable: None,
        supported_from_slot: SLOT,
        supported_until_slot_exclusive: SLOT + 1,
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
            source: metadata["rpc"].as_str().unwrap().into(),
            revision: format!("sha256:{RESPONSE_HASH}"),
            evidence_reference: format!("rpc:mainnet-spl-response.json:sha256:{RESPONSE_HASH}"),
        },
    }
}

fn catalog() -> Catalog {
    let mut version = version();
    let response: Value = serde_json::from_slice(RAW).unwrap();
    let index = metadata()["keys"]
        .as_array()
        .unwrap()
        .iter()
        .position(|key| key.as_str() == Some(&programdata().to_string()))
        .unwrap();
    let account: UiAccount =
        serde_json::from_value(response["result"]["value"][index].clone()).unwrap();
    let account: Account = account.to_account().unwrap();
    support::attest_fixture(
        &mut version,
        &account.data[UpgradeableLoaderState::size_of_programdata_metadata()..],
        SLOT,
    );
    Catalog::new(vec![version]).unwrap()
}

fn account(snapshot: &mut Snapshot, key: Pubkey) -> &mut Account {
    &mut snapshot
        .accounts
        .iter_mut()
        .find(|(address, _)| *address == key)
        .unwrap()
        .1
}

fn set_slot(snapshot: &mut Snapshot, slot: u64) {
    snapshot.observation.slot = slot;
    snapshot.observation.bank = format!("rpc:finalized:{slot}");
    let account = account(snapshot, sysvar::clock::id());
    let mut clock: Clock = bincode::deserialize(&account.data).unwrap();
    clock.slot = slot;
    account.data = bincode::serialize(&clock).unwrap();
}

async fn serve(
    reply: Value,
) -> (
    RpcClient,
    Arc<Mutex<Vec<Value>>>,
    tokio::task::JoinHandle<()>,
) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let app = Router::new().route(
        "/",
        post(move |Json(request): Json<Value>| {
            let recorded = recorded.clone();
            let reply = reply.clone();
            async move {
                recorded.lock().unwrap().push(request.clone());
                let mut response = match request["method"].as_str().unwrap() {
                    "getGenesisHash" => json!({"result": metadata()["cluster"]}),
                    "getMultipleAccounts" => reply,
                    other => panic!("unexpected RPC method: {other}"),
                };
                response["jsonrpc"] = json!("2.0");
                response["id"] = request["id"].clone();
                Json(response)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = RpcClient::new_with_timeout(
        format!("http://{}/", listener.local_addr().unwrap()),
        Duration::from_secs(5),
    );
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (client, requests, task)
}

#[tokio::test]
async fn captured_native_bytes_compile_and_replay_as_empty_partial_live_evidence() {
    assert_eq!(format!("{:x}", Sha256::digest(RAW)), RESPONSE_HASH);
    assert_eq!(metadata()["response_sha256"], RESPONSE_HASH);
    assert_eq!(
        source().to_string(),
        "HwD4QpS4bsutLbWZWhbmFUZfkXC5Au1DbkzYEzjDgps8"
    );
    assert!(captured()[&source()].is_null());
    let snapshot = snapshot().await;
    assert_eq!(snapshot.observation.origin, Origin::Finalized);
    assert_eq!(
        snapshot.observation.cluster,
        metadata()["cluster"].as_str().unwrap()
    );
    assert_eq!(snapshot.observation.bank, format!("rpc:finalized:{SLOT}"));
    assert_eq!(
        (
            snapshot.observation.slot,
            snapshot.observation.unix_timestamp
        ),
        (SLOT, TIMESTAMP)
    );
    assert_eq!(snapshot.targets, scope().targets);
    let accounts: BTreeMap<_, _> = snapshot.accounts.iter().cloned().collect();
    assert_eq!(accounts.len(), captured().len());
    for (key, raw) in captured() {
        let observed = &accounts[&key];
        if raw.is_null() {
            assert_eq!(observed, &Account::default());
        } else {
            let ui: UiAccount = serde_json::from_value(raw).unwrap();
            assert_eq!(observed.data, ui.data.decode().unwrap());
            assert_eq!(observed.owner.to_string(), ui.owner);
            assert_eq!(observed.executable, ui.executable);
            assert_eq!(observed.lamports, ui.lamports);
            assert_eq!(observed.rent_epoch, u64::MAX);
        }
    }
    let program_account = &accounts[&program()];
    assert_eq!(program_account.owner, bpf_loader_upgradeable::id());
    assert!(program_account.executable);
    assert_eq!(
        program_account.data.len(),
        UpgradeableLoaderState::size_of_program()
    );
    assert_eq!(
        bincode::deserialize::<UpgradeableLoaderState>(&program_account.data).unwrap(),
        UpgradeableLoaderState::Program {
            programdata_address: programdata()
        }
    );
    let data = &accounts[&programdata()];
    assert_eq!(data.owner, bpf_loader_upgradeable::id());
    assert!(!data.executable);
    let offset = UpgradeableLoaderState::size_of_programdata_metadata();
    assert_eq!(
        bincode::deserialize::<UpgradeableLoaderState>(&data.data[..offset]).unwrap(),
        UpgradeableLoaderState::ProgramData {
            slot: DEPLOYED,
            upgrade_authority_address: None
        }
    );
    assert_eq!(data.data[offset..].len(), 108_600);
    assert_eq!(
        format!("{:x}", Sha256::digest(&data.data[offset..])),
        PROGRAM_HASH
    );
    assert_eq!(spl::PROGRAM_VERSION, format!("sha256:{PROGRAM_HASH}"));
    assert_eq!(spl::LIVE_DEPLOYMENT, DEPLOYMENT);
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let record = journal
        .ingest(snapshot.clone(), &Catalog::new(vec![version()]).unwrap())
        .await
        .unwrap();
    assert_eq!(record.snapshot, snapshot);
    assert!(!record.coverage_complete);
    assert_eq!(record.versions, vec![version()]);
    assert_eq!(
        record.id,
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&snapshot).unwrap())
        )
    );
    assert!(matches!(record.projection, Projection::Incomplete { .. }));
    assert_eq!(journal.get(&record.id).await.unwrap(), Some(record.clone()));
    assert_eq!(journal.replay(&record.id).await.unwrap(), record);
}

#[tokio::test]
async fn official_client_requests_one_complete_finalized_batch_with_minimum_slot() {
    let mut scope = scope();
    scope.accounts = vec![source(), sysvar::clock::id(), program(), source()];
    let keys = scope.accounts().unwrap();
    assert_eq!(keys.len(), 4);
    let (client, requests, task) = serve(json!({"result": response(&scope)})).await;
    let observed = rpc::observe(&client, scope.clone()).await;
    task.abort();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["method"], "getGenesisHash");
    assert_eq!(requests[1]["method"], "getMultipleAccounts");
    assert_eq!(
        requests[1]["params"][0],
        json!(keys.iter().map(ToString::to_string).collect::<Vec<_>>())
    );
    assert_eq!(requests[1]["params"][1]["encoding"], "base64");
    assert_eq!(requests[1]["params"][1]["commitment"], "finalized");
    assert_eq!(requests[1]["params"][1]["minContextSlot"], SLOT);
    assert!(
        requests[1]["params"][1]
            .get("dataSlice")
            .is_none_or(Value::is_null)
    );
    assert_eq!(
        observed
            .unwrap()
            .accounts
            .iter()
            .map(|(key, _)| *key)
            .collect::<Vec<_>>(),
        keys
    );
}

#[tokio::test]
async fn scope_enforces_the_single_batch_limit_and_unique_targets_before_rpc() {
    let mut scope = scope();
    scope
        .accounts
        .extend((1..=96).map(|n| Pubkey::new_from_array([n; 32])));
    assert_eq!(scope.accounts().unwrap().len(), 100);
    scope.accounts.push(Pubkey::new_from_array([97; 32]));
    let failing_client = RpcClient::new_mock("fails".into());
    assert!(matches!(
        rpc::observe(&failing_client, scope).await,
        Err(Error::InvalidObservation("scope exceeds one RPC batch"))
    ));
    let mut duplicate = self::scope();
    duplicate.targets.push(duplicate.targets[0].clone());
    assert!(matches!(
        rpc::observe(&failing_client, duplicate).await,
        Err(Error::InvalidObservation("duplicate target"))
    ));
    for case in ["cluster", "targets"] {
        let mut empty = self::scope();
        match case {
            "cluster" => empty.cluster.clear(),
            "targets" => empty.targets.clear(),
            _ => unreachable!(),
        }
        assert!(matches!(
            rpc::observe(&failing_client, empty).await,
            Err(Error::InvalidObservation("empty scope"))
        ));
    }
    let mut subscription = self::scope();
    let [delegation, authority, source, mint, plan] =
        [1, 2, 3, 4, 5].map(|n| Pubkey::new_from_array([n; 32]));
    subscription.targets.push(Target::Subscription {
        delegation,
        authority,
        source,
        mint,
        plan: Some(plan),
    });
    let keys = subscription.accounts().unwrap();
    let subscription_program = subscriptions::DelegationAdapter.protocol().program_id;
    for key in [
        delegation,
        authority,
        source,
        mint,
        plan,
        sysvar::clock::id(),
        program(),
        programdata(),
        subscription_program,
        get_program_data_address(&subscription_program),
    ] {
        assert!(keys.contains(&key), "missing batch witness {key}");
    }
    subscription.targets.push(Target::Subscription {
        delegation,
        authority: mint,
        source,
        mint,
        plan: None,
    });
    assert!(matches!(
        subscription.accounts(),
        Err(Error::InvalidObservation("duplicate target"))
    ));
}

#[tokio::test]
async fn provider_errors_genesis_commitment_short_response_and_slot_floor_reject_observation() {
    let base = response(&scope());
    let mut short = base.clone();
    short["value"].as_array_mut().unwrap().pop();
    assert!(matches!(
        rpc::observe(&client(short), scope()).await,
        Err(Error::InvalidObservation("invalid finalized response"))
    ));
    let mut minimum = scope();
    minimum.min_context_slot = Some(SLOT + 1);
    assert!(matches!(
        rpc::observe(&client(base.clone()), minimum).await,
        Err(Error::InvalidObservation("invalid finalized response"))
    ));
    let mut other_cluster = scope();
    other_cluster.cluster = "11111111111111111111111111111111".into();
    assert!(matches!(
        rpc::observe(&client(Value::Null), other_cluster).await,
        Err(Error::InvalidObservation("cluster genesis mismatch"))
    ));
    for commitment in [CommitmentConfig::confirmed(), CommitmentConfig::processed()] {
        let config = RpcClientConfig::with_commitment(commitment);
        let client = RpcClient::new_sender(MockSender::new("fails"), config);
        assert!(
            matches!(
                rpc::observe(&client, scope()).await,
                Err(Error::InvalidObservation("finalized commitment required"))
            ),
            "{commitment:?}"
        );
    }
    let (client, requests, task) = serve(
        json!({"error": {"code": -32016, "message": "Minimum context slot has not been reached"}}),
    )
    .await;
    let result = rpc::observe(&client, scope()).await;
    task.abort();
    assert!(
        matches!(&result, Err(Error::Rpc(error)) if error.to_string().contains("Minimum context slot")),
        "{result:?}"
    );
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn malformed_some_accounts_and_response_shapes_reject_before_snapshot_creation() {
    let keys = scope().accounts().unwrap();
    let index = keys.iter().position(|key| *key == source()).unwrap();
    let some = json!({"lamports": 1, "owner": program().to_string(), "executable": false, "rentEpoch": u64::MAX, "space": 1, "data": ["AA==", "base64"]});
    for case in ["base64", "owner", "space"] {
        let mut response = response(&scope());
        response["value"][index] = some.clone();
        let value = &mut response["value"][index];
        match case {
            "base64" => value["data"] = json!(["!", "base64"]),
            "owner" => value["owner"] = json!("invalid"),
            "space" => value["space"] = json!(2),
            _ => unreachable!(),
        }
        let expected = if case == "space" {
            "truncated RPC account"
        } else {
            "undecodable RPC account"
        };
        assert!(
            matches!(rpc::observe(&client(response), scope()).await, Err(Error::InvalidObservation(reason)) if reason == expected),
            "{case}"
        );
    }
    for value in [
        json!({}),
        json!({"context": {"slot": SLOT}, "value": "invalid"}),
        json!({"context": {"slot": "454547887"}, "value": []}),
    ] {
        assert!(matches!(
            rpc::observe(&client(value), scope()).await,
            Err(Error::Rpc(_))
        ));
    }
    let mut missing_fields = response(&scope());
    missing_fields["value"][index] = json!({"data": ["AA==", "base64"]});
    assert!(matches!(
        rpc::observe(&client(missing_fields), scope()).await,
        Err(Error::Rpc(_))
    ));
}

#[tokio::test]
async fn explicit_null_missing_witness_and_malformed_native_state_remain_distinct() {
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let snapshot = snapshot().await;
    let record = journal.ingest(snapshot.clone(), &catalog()).await.unwrap();
    assert_eq!(
        record.projection,
        Projection::Compiled {
            authorizations: vec![]
        }
    );
    for key in [source(), program(), programdata(), sysvar::clock::id()] {
        let mut missing = snapshot.clone();
        missing.accounts.retain(|(address, _)| *address != key);
        let record = journal.ingest(missing.clone(), &catalog()).await.unwrap();
        assert!(
            matches!(record.projection, Projection::Incomplete { .. }),
            "{key}: {:?}",
            record.projection
        );
        assert_eq!(record.snapshot, missing);
        assert!(!record.coverage_complete);
        assert_eq!(journal.replay(&record.id).await.unwrap(), record);
    }
    let mut missing_authority = snapshot.clone();
    *account(&mut missing_authority, source()) = Account {
        lamports: 1,
        owner: program(),
        data: include_bytes!("fixtures/spl-delegate.bin").to_vec(),
        ..Account::default()
    };
    let record = journal
        .ingest(missing_authority.clone(), &catalog())
        .await
        .unwrap();
    assert!(matches!(record.projection, Projection::Incomplete { .. }));
    assert_eq!(record.snapshot, missing_authority);
    assert_eq!(journal.replay(&record.id).await.unwrap(), record);
    let mut malformed = response(&scope());
    let index = scope()
        .accounts()
        .unwrap()
        .iter()
        .position(|key| *key == source())
        .unwrap();
    malformed["value"][index] = json!({"lamports": 1, "owner": program().to_string(), "executable": false, "rentEpoch": u64::MAX, "space": 1, "data": ["AA==", "base64"]});
    let observed = rpc::observe(&client(malformed), scope()).await.unwrap();
    let record = journal.ingest(observed.clone(), &catalog()).await.unwrap();
    assert!(matches!(record.projection, Projection::Invalid { .. }));
    assert_eq!(record.snapshot, observed);
    assert_eq!(journal.replay(&record.id).await.unwrap(), record);
}

#[tokio::test]
async fn malformed_clock_rejects_rpc_and_changed_bank_time_invalidates_compilation() {
    let index = scope()
        .accounts()
        .unwrap()
        .iter()
        .position(|key| *key == sysvar::clock::id())
        .unwrap();
    for case in ["slot", "owner", "lamports", "executable", "size", "null"] {
        let mut response = response(&scope());
        let clock = &mut response["value"][index];
        match case {
            "slot" => response["context"]["slot"] = json!(SLOT + 1),
            "owner" => clock["owner"] = json!(program().to_string()),
            "lamports" => clock["lamports"] = json!(0),
            "executable" => clock["executable"] = json!(true),
            "size" => {
                clock["data"] = json!(["AA==", "base64"]);
                clock["space"] = json!(1);
            }
            "null" => *clock = Value::Null,
            _ => unreachable!(),
        }
        assert!(
            matches!(
                rpc::observe(&client(response), scope()).await,
                Err(Error::InvalidObservation("invalid bank clock"))
            ),
            "{case}"
        );
    }
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let snapshot = snapshot().await;
    for case in ["time", "slot", "owner"] {
        let mut input = snapshot.clone();
        match case {
            "time" => input.observation.unix_timestamp += 1,
            "slot" => input.observation.slot += 1,
            "owner" => account(&mut input, sysvar::clock::id()).owner = program(),
            _ => unreachable!(),
        }
        let mut version = version();
        version.supported_from_slot = input.observation.slot;
        version.supported_until_slot_exclusive = input.observation.slot + 1;
        let record = journal
            .ingest(input.clone(), &Catalog::new(vec![version]).unwrap())
            .await
            .unwrap();
        assert!(
            matches!(record.projection, Projection::Invalid { .. }),
            "{case}"
        );
        assert_eq!(record.snapshot, input);
        assert!(!record.coverage_complete);
        assert_eq!(journal.replay(&record.id).await.unwrap(), record);
    }
}

#[tokio::test]
async fn executable_hash_deployment_and_catalog_identity_fail_closed() {
    let snapshot = snapshot().await;
    for case in [
        "executable-bytes",
        "program-owner",
        "metadata-slot",
        "catalog-version",
        "catalog-deployment",
        "adapter-version",
        "fixture-deployment",
        "unknown-catalog",
    ] {
        let mut input = snapshot.clone();
        let mut version = version();
        let elf = account(&mut input, programdata()).data
            [UpgradeableLoaderState::size_of_programdata_metadata()..]
            .to_vec();
        support::attest_fixture(&mut version, &elf, input.observation.slot);
        match case {
            "executable-bytes" => *account(&mut input, programdata()).data.last_mut().unwrap() ^= 1,
            "program-owner" => account(&mut input, program()).owner = Pubkey::default(),
            "metadata-slot" => account(&mut input, programdata()).data[4..12]
                .copy_from_slice(&(DEPLOYED - 1).to_le_bytes()),
            "catalog-version" => version.version = format!("sha256:{}", "0".repeat(64)),
            "catalog-deployment" => {
                version.deployment = format!("solana:loader-v3:{}:{}", programdata(), DEPLOYED - 1)
            }
            "adapter-version" => version.adapter.version = "unknown".into(),
            "fixture-deployment" => version.deployment = spl::DEPLOYMENT.into(),
            "unknown-catalog" => version.program_id = Pubkey::new_from_array([99; 32]),
            _ => unreachable!(),
        }
        let journal = Journal::open("sqlite::memory:").await.unwrap();
        let record = journal
            .ingest(input.clone(), &Catalog::new(vec![version]).unwrap())
            .await
            .unwrap();
        assert!(
            matches!(record.projection, Projection::Unsupported { .. }),
            "{case}: {:?}",
            record.projection
        );
        assert_eq!(record.snapshot, input);
        assert!(!record.coverage_complete);
        assert_eq!(journal.replay(&record.id).await.unwrap(), record);
    }
    for slot in [SLOT - 1, SLOT + 1] {
        let journal = Journal::open("sqlite::memory:").await.unwrap();
        let mut outside = snapshot.clone();
        set_slot(&mut outside, slot);
        let record = journal.ingest(outside, &catalog()).await.unwrap();
        assert!(matches!(record.projection, Projection::Unsupported { .. }));
        assert!(record.versions.is_empty());
        assert_eq!(journal.replay(&record.id).await.unwrap(), record);
    }
}

#[tokio::test]
async fn malformed_loader_accounts_are_invalid_and_new_code_is_visible_after_deployment_slot() {
    let snapshot = snapshot().await;
    for case in [
        "program-executable",
        "program-lamports",
        "program-length",
        "program-state",
        "derived-address",
        "programdata-owner",
        "programdata-executable",
        "programdata-lamports",
        "programdata-truncated",
        "programdata-state",
        "programdata-header",
    ] {
        let mut input = snapshot.clone();
        match case {
            "program-executable" => account(&mut input, program()).executable = false,
            "program-lamports" => account(&mut input, program()).lamports = 0,
            "program-length" => account(&mut input, program()).data.truncate(35),
            "program-state" => {
                account(&mut input, program()).data[..4].copy_from_slice(&u32::MAX.to_le_bytes())
            }
            "derived-address" => {
                account(&mut input, program()).data =
                    bincode::serialize(&UpgradeableLoaderState::Program {
                        programdata_address: Pubkey::new_from_array([99; 32]),
                    })
                    .unwrap()
            }
            "programdata-owner" => account(&mut input, programdata()).owner = Pubkey::default(),
            "programdata-executable" => account(&mut input, programdata()).executable = true,
            "programdata-lamports" => account(&mut input, programdata()).lamports = 0,
            "programdata-truncated" => account(&mut input, programdata())
                .data
                .truncate(UpgradeableLoaderState::size_of_programdata_metadata()),
            "programdata-state" => {
                account(&mut input, programdata()).data[..4].copy_from_slice(&0u32.to_le_bytes())
            }
            "programdata-header" => account(&mut input, programdata()).data[..4]
                .copy_from_slice(&u32::MAX.to_le_bytes()),
            _ => unreachable!(),
        }
        let journal = Journal::open("sqlite::memory:").await.unwrap();
        let record = journal.ingest(input.clone(), &catalog()).await.unwrap();
        assert!(
            matches!(record.projection, Projection::Invalid { .. }),
            "{case}: {:?}",
            record.projection
        );
        assert_eq!(record.snapshot, input);
        assert!(!record.coverage_complete);
        assert_eq!(journal.replay(&record.id).await.unwrap(), record);
    }
    // Synthetic banks isolate loader visibility while retaining the captured executable.
    for slot in [DEPLOYED, DEPLOYED + 1] {
        let mut input = snapshot.clone();
        set_slot(&mut input, slot);
        let mut version = version();
        version.supported_from_slot = slot;
        version.supported_until_slot_exclusive = slot + 1;
        let journal = Journal::open("sqlite::memory:").await.unwrap();
        let record = journal
            .ingest(input, &Catalog::new(vec![version]).unwrap())
            .await
            .unwrap();
        if slot == DEPLOYED {
            assert!(matches!(record.projection, Projection::Unsupported { .. }));
        } else {
            assert!(matches!(record.projection, Projection::Incomplete { .. }));
        }
        assert_eq!(journal.replay(&record.id).await.unwrap(), record);
    }
}

#[tokio::test]
async fn native_json_u64_values_survive_ui_observation_journal_and_replay() {
    let mut response = response(&scope());
    response["context"]["slot"] = json!(u64::MAX);
    let keys = scope().accounts().unwrap();
    let index = keys
        .iter()
        .position(|key| *key == sysvar::clock::id())
        .unwrap();
    let ui: UiAccount = serde_json::from_value(response["value"][index].clone()).unwrap();
    let mut clock: Clock = bincode::deserialize(&ui.data.decode().unwrap()).unwrap();
    clock.slot = u64::MAX;
    response["value"][index]["data"] = json!([
        bs58::encode(bincode::serialize(&clock).unwrap()).into_string(),
        "base58"
    ]);
    response["value"][index]["lamports"] = json!(u64::MAX);
    let wire = serde_json::to_vec(&response).unwrap();
    assert!(
        std::str::from_utf8(&wire)
            .unwrap()
            .contains("18446744073709551615")
    );
    let mut observed = rpc::observe(&client(serde_json::from_slice(&wire).unwrap()), scope())
        .await
        .unwrap();
    assert_eq!(observed.observation.slot, u64::MAX);
    assert_eq!(
        account(&mut observed, sysvar::clock::id()).lamports,
        u64::MAX
    );
    assert_eq!(account(&mut observed, programdata()).rent_epoch, u64::MAX);
    assert_eq!(
        serde_json::from_slice::<Snapshot>(&serde_json::to_vec(&observed).unwrap()).unwrap(),
        observed
    );
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    let record = journal.ingest(observed.clone(), &catalog()).await.unwrap();
    assert_eq!(record.snapshot, observed);
    assert!(matches!(record.projection, Projection::Unsupported { .. }));
    assert!(record.versions.is_empty());
    assert!(!record.coverage_complete);
    assert_eq!(journal.get(&record.id).await.unwrap(), Some(record.clone()));
    assert_eq!(journal.replay(&record.id).await.unwrap(), record);
}

#[tokio::test]
async fn malformed_snapshot_structure_never_enters_the_journal() {
    let snapshot = snapshot().await;
    let journal = Journal::open("sqlite::memory:").await.unwrap();
    for case in [
        "accounts",
        "targets",
        "bank",
        "cluster",
        "duplicate-account",
        "duplicate-target",
    ] {
        let mut input = snapshot.clone();
        match case {
            "accounts" => input.accounts.clear(),
            "targets" => input.targets.clear(),
            "bank" => input.observation.bank.clear(),
            "cluster" => input.observation.cluster.clear(),
            "duplicate-account" => input.accounts.push(input.accounts[0].clone()),
            "duplicate-target" => input.targets.push(input.targets[0].clone()),
            _ => unreachable!(),
        }
        assert!(
            matches!(
                journal.ingest(input, &catalog()).await,
                Err(Error::InvalidSnapshot)
            ),
            "{case}"
        );
    }
    let mut json = serde_json::to_value(&snapshot).unwrap();
    json["observation"]["slot"] = json!("454547887");
    assert!(serde_json::from_value::<Snapshot>(json).is_err());
}

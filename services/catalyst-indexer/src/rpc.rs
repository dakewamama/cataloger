use crate::{Coverage, Error, Journal, Observation, Origin, Record, Snapshot, Target};
use arm::AuthorizationChange;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use cataloger::{Catalog, ProgramVersion};
use catalyst_sdk::{self as sdk, Adapter, Error as AdapterError, spl, subscriptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solana_account::Account;
use solana_account_decoder_client_types::UiAccountEncoding;
use solana_clock::Clock;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_loader_v3_interface::{get_program_data_address, state::UpgradeableLoaderState};
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::{
    config::{
        RpcAccountInfoConfig, RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig,
        RpcTransactionConfig,
    },
    request::RpcRequest,
    response::{Response, RpcSimulateTransactionResult},
};
use solana_sdk_ids::{bpf_loader_upgradeable, sysvar};
use solana_transaction::Transaction;
use solana_transaction_status_client_types::{
    EncodedConfirmedTransactionWithStatusMeta, EncodedTransaction, TransactionBinaryEncoding,
    UiTransactionEncoding,
};
use spl_token_interface::state::Account as TokenAccount;
use std::{collections::BTreeSet, sync::Arc};

/// Interpret an ELF with Agave 3.1.10's explicit local environment, not a live-bank claim.
pub fn executable_identity(
    elf: &[u8],
) -> Result<cataloger::executable::ExecutableIdentity, Box<dyn std::error::Error>> {
    let loader = agave_syscalls::create_program_runtime_environment_v1(
        &solana_svm_feature_set::SVMFeatureSet::all_enabled(),
        &solana_program_runtime::execution_budget::SVMTransactionExecutionBudget::new_with_defaults(
            true,
        ),
        false,
        false,
    )?;
    cataloger::executable::inspect(elf, std::sync::Arc::new(loader))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub cluster: String,
    pub accounts: Vec<Pubkey>,
    pub targets: Vec<Target>,
    pub min_context_slot: Option<u64>,
}

impl Scope {
    pub fn accounts(&self) -> Result<Vec<Pubkey>, Error> {
        if self.cluster.is_empty() || self.targets.is_empty() {
            return Err(Error::InvalidObservation("empty scope"));
        }
        let mut targets = BTreeSet::new();
        let mut keys: BTreeSet<_> = self.accounts.iter().copied().collect();
        keys.insert(sysvar::clock::id());
        keys.insert(spl::DelegateAdapter.protocol().program_id);
        for target in &self.targets {
            if !targets.insert(target.key()) {
                return Err(Error::InvalidObservation("duplicate target"));
            }
            match target {
                Target::SplDelegate { source } => {
                    keys.insert(*source);
                }
                Target::Subscription {
                    delegation,
                    authority,
                    source,
                    mint,
                    plan,
                } => {
                    keys.extend([*delegation, *authority, *source, *mint]);
                    keys.extend(*plan);
                    keys.insert(subscriptions::DelegationAdapter.protocol().program_id);
                }
            }
        }
        for program in [
            spl::DelegateAdapter.protocol().program_id,
            subscriptions::DelegationAdapter.protocol().program_id,
        ] {
            if keys.contains(&program) {
                keys.insert(get_program_data_address(&program));
            }
        }
        // Splitting the RPC limit into chunks would combine different banks.
        if keys.len() > 100 {
            return Err(Error::InvalidObservation("scope exceeds one RPC batch"));
        }
        Ok(keys.into_iter().collect())
    }
}

fn clock(accounts: &[(Pubkey, Account)], slot: u64) -> Result<Clock, AdapterError> {
    let account = accounts
        .iter()
        .find(|(key, _)| *key == sysvar::clock::id())
        .map(|(_, account)| account)
        .ok_or(AdapterError::InsufficientEvidence)?;
    if account.lamports == 0
        || account.owner != sysvar::id()
        || account.executable
        || account.data.len() != solana_clock::SIZE
    {
        return Err(AdapterError::InvalidState("clock account".into()));
    }
    let clock: Clock = bincode::deserialize(&account.data)
        .map_err(|_| AdapterError::InvalidState("clock data".into()))?;
    if clock.slot != slot {
        return Err(AdapterError::InvalidState(
            "clock/context slot mismatch".into(),
        ));
    }
    Ok(clock)
}

pub async fn observe(client: &RpcClient, scope: Scope) -> Result<Snapshot, Error> {
    let keys = scope.accounts()?;
    if !client.commitment().is_finalized() {
        return Err(Error::InvalidObservation("finalized commitment required"));
    }
    if client.get_genesis_hash().await?.to_string() != scope.cluster {
        return Err(Error::InvalidObservation("cluster genesis mismatch"));
    }
    let response = client
        .get_multiple_ui_accounts_with_config(
            &keys,
            RpcAccountInfoConfig {
                encoding: Some(UiAccountEncoding::Base64),
                commitment: Some(client.commitment()),
                min_context_slot: scope.min_context_slot,
                ..Default::default()
            },
        )
        .await?;
    if response.value.len() != keys.len()
        || scope
            .min_context_slot
            .is_some_and(|minimum| response.context.slot < minimum)
    {
        return Err(Error::InvalidObservation("invalid finalized response"));
    }
    let accounts = keys
        .into_iter()
        .zip(response.value)
        .map(|(key, value)| {
            let account = match value {
                None => Account::default(),
                Some(value) => {
                    let account = value
                        .to_account()
                        .ok_or(Error::InvalidObservation("undecodable RPC account"))?;
                    if value
                        .space
                        .is_some_and(|space| space != account.data.len() as u64)
                    {
                        return Err(Error::InvalidObservation("truncated RPC account"));
                    }
                    account
                }
            };
            Ok((key, account))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let clock = clock(&accounts, response.context.slot)
        .map_err(|_| Error::InvalidObservation("invalid bank clock"))?;
    Ok(Snapshot {
        observation: Observation {
            cluster: scope.cluster,
            bank: format!("rpc:finalized:{}", clock.slot),
            slot: clock.slot,
            unix_timestamp: clock.unix_timestamp,
            origin: Origin::Finalized,
        },
        accounts,
        targets: scope.targets,
    })
}

// The fixed budget prevents wallet-added fees from changing the reviewed message.
const REVOKE_UNIT_LIMIT: u32 = 10_000;
const REVOKE_UNIT_PRICE: u64 = 1_000_000;

fn revoke_transaction(action: &sdk::Action, payer: Pubkey) -> Transaction {
    let mut instructions = vec![
        ComputeBudgetInstruction::set_compute_unit_limit(REVOKE_UNIT_LIMIT),
        ComputeBudgetInstruction::set_compute_unit_price(REVOKE_UNIT_PRICE),
    ];
    instructions.extend_from_slice(&action.instructions);
    Transaction::new_with_payer(&instructions, Some(&payer))
}

#[derive(Serialize)]
pub struct SimulatedRevoke {
    pub id: String,
    pub state_version: String,
    pub observation: Observation,
    pub coverage: Coverage,
    pub versions: Vec<ProgramVersion>,
    pub runtime_version: String,
    pub sdk_revision: String,
    pub arm_schema_version: &'static str,
    pub transaction: Transaction,
    pub signing_transaction: String,
    pub config: RpcSimulateTransactionConfig,
    pub response: Response<RpcSimulateTransactionResult>,
    pub changes: Vec<AuthorizationChange>,
}

/// Simulate one canonical SPL revoke against the retained finalized bank.
/// The result is unsigned simulation evidence, never finalized journal state.
pub async fn simulate_revoke(
    client: &RpcClient,
    journal: &Journal,
    id: &str,
    source: Pubkey,
) -> Result<SimulatedRevoke, Error> {
    let record = journal.replay(id).await?;
    let snapshot = &record.snapshot;
    if snapshot.observation.origin != Origin::Finalized || !client.commitment().is_finalized() {
        return Err(Error::InvalidObservation("finalized evidence required"));
    }
    if snapshot.targets != [Target::SplDelegate { source }] {
        return Err(AdapterError::UnsupportedOperation.into());
    }
    if journal
        .latest(&snapshot.scope_id()?)
        .await?
        .is_none_or(|latest| latest.id != id)
    {
        return Err(Error::Conflict);
    }
    let adapter = spl::DelegateAdapter;
    let catalog = Catalog::new(record.versions.clone()).map_err(|_| Error::ReplayMismatch)?;
    let context = crate::context(
        adapter.protocol().program_id,
        snapshot,
        &catalog,
        id,
        &mut Vec::new(),
    )?;
    let before = crate::spl_state(snapshot, source)?;
    let authorization = sdk::compile_state(&adapter, &before, &context)?
        .into_iter()
        .next()
        .ok_or(Error::NotFound)?;
    let action = sdk::actions(&adapter, &authorization, &before, &context)?.remove(0);
    let mut native = TokenAccount::unpack(&before.account.data)
        .map_err(|error| AdapterError::InvalidState(error.to_string()))?;
    let transaction = revoke_transaction(&action, native.owner);
    let keys = [source, native.owner, sysvar::clock::id()];
    let config = RpcSimulateTransactionConfig {
        sig_verify: false,
        replace_recent_blockhash: true,
        commitment: Some(client.commitment()),
        accounts: Some(RpcSimulateTransactionAccountsConfig {
            encoding: Some(UiAccountEncoding::Base64),
            addresses: keys.iter().map(ToString::to_string).collect(),
        }),
        min_context_slot: Some(snapshot.observation.slot),
        ..Default::default()
    };
    if client.get_genesis_hash().await?.to_string() != snapshot.observation.cluster {
        return Err(Error::InvalidObservation("cluster genesis mismatch"));
    }
    let response = client
        .simulate_transaction_with_config(&transaction, config.clone())
        .await?;
    if response.value.err.is_some() {
        return Err(Error::SimulationFailed(Box::new(response)));
    }
    // minContextSlot does not pin a bank; only the same finalized slot can agree.
    if response.context.slot != snapshot.observation.slot {
        return Err(Error::InvalidObservation(
            "simulation/context slot mismatch",
        ));
    }
    let values = response
        .value
        .accounts
        .as_ref()
        .filter(|values| values.len() == keys.len())
        .ok_or(Error::InvalidObservation("missing simulated accounts"))?;
    let mut after = snapshot.clone();
    for (key, value) in keys.into_iter().zip(values) {
        let account = match value {
            Some(value) => {
                let account = value
                    .to_account()
                    .ok_or(Error::InvalidObservation("undecodable RPC account"))?;
                if value
                    .space
                    .is_some_and(|space| space != account.data.len() as u64)
                {
                    return Err(Error::InvalidObservation("truncated RPC account"));
                }
                account
            }
            None if key == source => {
                return Err(Error::InvalidObservation("missing simulated source"));
            }
            None => Account::default(),
        };
        let entry = after
            .accounts
            .iter_mut()
            .find(|(address, _)| *address == key)
            .ok_or(AdapterError::InsufficientEvidence)?;
        entry.1 = account;
    }
    if clock(&after.accounts, response.context.slot)?
        != clock(&snapshot.accounts, snapshot.observation.slot)?
    {
        return Err(Error::InvalidObservation("simulation clock mismatch"));
    }
    // The delegate projection omits owner authority; require Revoke's full token-data change.
    native.delegate = None.into();
    native.delegated_amount = 0;
    let mut expected = before.account.data.clone();
    TokenAccount::pack(native, &mut expected)
        .map_err(|error| AdapterError::InvalidState(error.to_string()))?;
    let after_source = &after
        .accounts
        .iter()
        .find(|(key, _)| *key == source)
        .ok_or(AdapterError::InsufficientEvidence)?
        .1;
    if after_source.data != expected {
        return Err(AdapterError::DiffMismatch.into());
    }
    let after_state = crate::spl_state(&after, source)?;
    let simulation_id = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            &record.id,
            &transaction,
            &config,
            &response
        ))?)
    );
    let mut observed = context.clone();
    observed
        .evidence
        .references
        .push(format!("simulation:sha256:{simulation_id}"));
    observed.evidence.observed_at = format!(
        "{}:rpc:simulation:{}",
        snapshot.observation.cluster, response.context.slot
    );
    let changes = sdk::verify_transaction_diff(
        &adapter,
        &action.instructions,
        &before,
        &context,
        &after_state,
        &observed,
    )?;
    let replacement = response
        .value
        .replacement_blockhash
        .as_ref()
        .filter(|replacement| replacement.last_valid_block_height > 0)
        .ok_or(Error::InvalidObservation(
            "missing simulation blockhash lifetime",
        ))?;
    let mut signing_transaction = transaction.clone();
    signing_transaction.message.recent_blockhash = replacement
        .blockhash
        .parse()
        .map_err(|_| Error::InvalidObservation("invalid simulation blockhash"))?;
    if signing_transaction.message.recent_blockhash == Default::default() {
        return Err(Error::InvalidObservation("invalid simulation blockhash"));
    }
    let signing_transaction = STANDARD.encode(
        bincode::serialize(&signing_transaction)
            .map_err(|_| Error::InvalidObservation("transaction encoding failed"))?,
    );
    // A checkpoint observed during RPC must invalidate the older simulation input.
    if journal
        .latest(&snapshot.scope_id()?)
        .await?
        .is_none_or(|latest| latest.id != id)
    {
        return Err(Error::Conflict);
    }
    Ok(SimulatedRevoke {
        id: simulation_id,
        state_version: record.id,
        observation: snapshot.observation.clone(),
        coverage: Coverage {
            complete: false,
            accounts: snapshot.accounts.iter().map(|(key, _)| *key).collect(),
            targets: snapshot.targets.clone(),
        },
        versions: record.versions,
        runtime_version: record.runtime_version,
        sdk_revision: record.sdk_revision,
        arm_schema_version: arm::SCHEMA_VERSION,
        transaction,
        signing_transaction,
        config,
        response,
        changes,
    })
}

pub const CURRENT_BANK_ATTEMPTS: usize = 8;

/// Observe the scope's finalized bank, then simulate one revoke in that same bank.
/// Finalized slots advance between requests, so a retained record cannot be simulated later.
pub async fn simulate_current_revoke(
    client: &RpcClient,
    journal: &Journal,
    scope_id: &str,
    source: Pubkey,
) -> Result<SimulatedRevoke, Error> {
    for _ in 0..CURRENT_BANK_ATTEMPTS {
        let record = journal.latest(scope_id).await?.ok_or(Error::NotFound)?;
        let snapshot = record.snapshot;
        if snapshot.observation.origin != Origin::Finalized {
            return Err(Error::InvalidObservation("finalized evidence required"));
        }
        if snapshot.targets != [Target::SplDelegate { source }] {
            return Err(AdapterError::UnsupportedOperation.into());
        }
        let catalog = Catalog::new(record.versions).map_err(|_| Error::ReplayMismatch)?;
        let current = match observe(
            client,
            Scope {
                cluster: snapshot.observation.cluster,
                accounts: snapshot.accounts.iter().map(|(key, _)| *key).collect(),
                targets: snapshot.targets,
                min_context_slot: Some(snapshot.observation.slot),
            },
        )
        .await
        {
            // A node can answer with a Clock from the bank next to the one it names; read again.
            Err(Error::InvalidObservation("invalid bank clock")) => continue,
            current => current?,
        };
        let id = journal.ingest(current, &catalog).await?.id;
        match simulate_revoke(client, journal, &id, source).await {
            Err(Error::InvalidObservation("simulation/context slot mismatch")) => continue,
            result => return result,
        }
    }
    Err(Error::InvalidObservation(
        "simulation/context slot mismatch",
    ))
}

#[derive(Serialize)]
pub struct VerifiedRevoke {
    pub verification_id: String,
    pub state_version: String,
    pub transaction: EncodedConfirmedTransactionWithStatusMeta,
    pub observation: Record,
    pub changes: Vec<AuthorizationChange>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeEvidence {
    state_version: String,
    observation_id: String,
    transaction: EncodedConfirmedTransactionWithStatusMeta,
}

impl Journal {
    pub async fn replay_revoke(&self, id: &str) -> Result<VerifiedRevoke, Error> {
        let raw: String =
            sqlx::query_scalar("SELECT evidence FROM revoke_verifications WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.database)
                .await?
                .ok_or(Error::NotFound)?;
        if format!("{:x}", Sha256::digest(raw.as_bytes())) != id {
            return Err(Error::ReplayMismatch);
        }
        let evidence: RevokeEvidence = serde_json::from_str(&raw)?;
        let record = self
            .replay(&evidence.state_version)
            .await
            .map_err(|error| match error {
                Error::NotFound => Error::ReplayMismatch,
                error => error,
            })?;
        let observation =
            self.replay(&evidence.observation_id)
                .await
                .map_err(|error| match error {
                    Error::NotFound => Error::ReplayMismatch,
                    error => error,
                })?;
        let [Target::SplDelegate { source }] = record.snapshot.targets.as_slice() else {
            return Err(Error::ReplayMismatch);
        };
        let signed = evidence
            .transaction
            .transaction
            .transaction
            .decode()
            .and_then(|transaction| transaction.into_legacy_transaction())
            .ok_or(Error::ReplayMismatch)?;
        let action =
            checked_revoke(&record, *source, &signed).map_err(|_| Error::ReplayMismatch)?;
        check_receipt(
            &evidence.transaction,
            &signed,
            record.snapshot.observation.slot,
        )
        .map_err(|_| Error::ReplayMismatch)?;
        let changes = revoke_changes(&record, &observation, &action, evidence.transaction.slot)
            .map_err(|_| Error::ReplayMismatch)?;
        Ok(VerifiedRevoke {
            verification_id: id.into(),
            state_version: record.id,
            transaction: evidence.transaction,
            observation,
            changes,
        })
    }
}

pub(crate) async fn retained_revoke(
    State(journal): State<Journal>,
    Path(id): Path<String>,
) -> Result<Json<VerifiedRevoke>, (StatusCode, Json<serde_json::Value>)> {
    journal.replay_revoke(&id).await.map(Json).map_err(|error| {
        if matches!(error, Error::NotFound) {
            action_error(error)
        } else {
            action_error(Error::ReplayMismatch)
        }
    })
}

fn checked_revoke(
    record: &Record,
    source: Pubkey,
    signed: &Transaction,
) -> Result<sdk::Action, Error> {
    let snapshot = &record.snapshot;
    if snapshot.observation.origin != Origin::Finalized {
        return Err(Error::InvalidObservation("finalized evidence required"));
    }
    if snapshot.targets != [Target::SplDelegate { source }] {
        return Err(AdapterError::UnsupportedOperation.into());
    }
    signed
        .verify()
        .map_err(|_| Error::InvalidObservation("invalid transaction signature"))?;
    let catalog = Catalog::new(record.versions.clone()).map_err(|_| Error::ReplayMismatch)?;
    let adapter = spl::DelegateAdapter;
    let context = crate::context(
        adapter.protocol().program_id,
        snapshot,
        &catalog,
        &record.id,
        &mut Vec::new(),
    )?;
    let before = crate::spl_state(snapshot, source)?;
    let authorization = sdk::compile_state(&adapter, &before, &context)?
        .into_iter()
        .next()
        .ok_or(Error::NotFound)?;
    let action = sdk::actions(&adapter, &authorization, &before, &context)?.remove(0);
    let native = TokenAccount::unpack(&before.account.data)
        .map_err(|error| AdapterError::InvalidState(error.to_string()))?;
    // Earlier receipts contain only Revoke; both historical formats remain exact.
    let mut expected = if signed.message.instructions.len() == 1 {
        Transaction::new_with_payer(&action.instructions, Some(&native.owner))
    } else {
        revoke_transaction(&action, native.owner)
    };
    expected.message.recent_blockhash = signed.message.recent_blockhash;
    if signed.message != expected.message {
        return Err(AdapterError::DiffMismatch.into());
    }
    Ok(action)
}

fn check_receipt(
    transaction: &EncodedConfirmedTransactionWithStatusMeta,
    signed: &Transaction,
    before_slot: u64,
) -> Result<(), Error> {
    let executed = transaction
        .transaction
        .transaction
        .decode()
        .and_then(|transaction| transaction.into_legacy_transaction())
        .ok_or(Error::InvalidObservation("invalid finalized transaction"))?;
    if executed != *signed || transaction.slot <= before_slot {
        return Err(Error::InvalidObservation("finalized transaction mismatch"));
    }
    let meta = transaction
        .transaction
        .meta
        .as_ref()
        .ok_or(Error::InvalidObservation(
            "missing transaction execution metadata",
        ))?;
    if meta.err.is_some() || meta.status.is_err() {
        return Err(Error::InvalidObservation("finalized transaction failed"));
    }
    Ok(())
}

fn revoke_changes(
    record: &Record,
    observation: &Record,
    action: &sdk::Action,
    transaction_slot: u64,
) -> Result<Vec<AuthorizationChange>, Error> {
    let snapshot = &record.snapshot;
    let after = &observation.snapshot;
    if after.observation.origin != Origin::Finalized
        || after.scope_id()? != snapshot.scope_id()?
        || after.observation.slot < transaction_slot
    {
        return Err(Error::InvalidObservation("revoke observation mismatch"));
    }
    let [Target::SplDelegate { source }] = snapshot.targets.as_slice() else {
        return Err(AdapterError::UnsupportedOperation.into());
    };
    let catalog = Catalog::new(record.versions.clone()).map_err(|_| Error::ReplayMismatch)?;
    let adapter = spl::DelegateAdapter;
    let context = crate::context(
        adapter.protocol().program_id,
        snapshot,
        &catalog,
        &record.id,
        &mut Vec::new(),
    )?;
    let after_context = crate::context(
        adapter.protocol().program_id,
        after,
        &catalog,
        &observation.id,
        &mut Vec::new(),
    )?;
    let before = crate::spl_state(snapshot, *source)?;
    let after_state = crate::spl_state(after, *source)?;
    let mut expected = TokenAccount::unpack(&before.account.data)
        .map_err(|error| AdapterError::InvalidState(error.to_string()))?;
    expected.delegate = None.into();
    expected.delegated_amount = 0;
    let mut expected_data = before.account.data.clone();
    TokenAccount::pack(expected, &mut expected_data)
        .map_err(|error| AdapterError::InvalidState(error.to_string()))?;
    if after_state.account.data != expected_data {
        return Err(AdapterError::DiffMismatch.into());
    }
    Ok(sdk::verify_transaction_diff(
        &adapter,
        &action.instructions,
        &before,
        &context,
        &after_state,
        &after_context,
    )?)
}

/// Verify a finalized canonical revoke and a later scoped observation.
/// The observation is not the transaction's immediate post-state.
pub async fn verify_revoke(
    client: &RpcClient,
    journal: &Journal,
    id: &str,
    source: Pubkey,
    signed_transaction: &str,
) -> Result<VerifiedRevoke, Error> {
    let record = journal.replay(id).await?;
    let snapshot = &record.snapshot;
    if snapshot.observation.origin != Origin::Finalized || !client.commitment().is_finalized() {
        return Err(Error::InvalidObservation("finalized evidence required"));
    }
    if snapshot.targets != [Target::SplDelegate { source }] {
        return Err(AdapterError::UnsupportedOperation.into());
    }
    let signed =
        EncodedTransaction::Binary(signed_transaction.into(), TransactionBinaryEncoding::Base64)
            .decode()
            .and_then(|transaction| transaction.into_legacy_transaction())
            .ok_or(Error::InvalidObservation(
                "invalid signed legacy transaction",
            ))?;
    let supplied = STANDARD
        .decode(signed_transaction)
        .map_err(|_| Error::InvalidObservation("invalid transaction encoding"))?;
    let canonical = bincode::serialize(&signed)
        .map_err(|_| Error::InvalidObservation("transaction encoding failed"))?;
    if supplied != canonical {
        return Err(Error::InvalidObservation("noncanonical signed transaction"));
    }
    let action = checked_revoke(&record, source, &signed)?;
    let catalog = Catalog::new(record.versions.clone()).map_err(|_| Error::ReplayMismatch)?;
    if client.get_genesis_hash().await?.to_string() != snapshot.observation.cluster {
        return Err(Error::InvalidObservation("cluster genesis mismatch"));
    }
    let transaction: Option<EncodedConfirmedTransactionWithStatusMeta> = client
        .send(
            RpcRequest::GetTransaction,
            serde_json::json!([
                signed.signatures[0].to_string(),
                RpcTransactionConfig {
                    encoding: Some(UiTransactionEncoding::Base64),
                    commitment: Some(client.commitment()),
                    max_supported_transaction_version: None,
                }
            ]),
        )
        .await?;
    let transaction = transaction.ok_or(Error::NotFound)?;
    check_receipt(&transaction, &signed, snapshot.observation.slot)?;
    let after = observe(
        client,
        Scope {
            cluster: snapshot.observation.cluster.clone(),
            accounts: snapshot.accounts.iter().map(|(key, _)| *key).collect(),
            targets: snapshot.targets.clone(),
            min_context_slot: Some(transaction.slot),
        },
    )
    .await?;
    // Retain upgrades and incomplete evidence too, so stale grants stop being current.
    let observation = journal.ingest(after, &catalog).await?;
    let changes = revoke_changes(&record, &observation, &action, transaction.slot)?;
    if journal
        .latest(&snapshot.scope_id()?)
        .await?
        .is_none_or(|latest| latest.id != observation.id)
    {
        return Err(Error::Conflict);
    }
    let evidence = RevokeEvidence {
        state_version: record.id,
        observation_id: observation.id.clone(),
        transaction,
    };
    let raw = serde_json::to_string(&evidence)?;
    let verification_id = format!("{:x}", Sha256::digest(raw.as_bytes()));
    sqlx::query(
        "INSERT INTO revoke_verifications (id, evidence) VALUES (?, ?) ON CONFLICT(id) DO NOTHING",
    )
    .bind(&verification_id)
    .bind(&raw)
    .execute(&journal.database)
    .await?;
    let retained: String =
        sqlx::query_scalar("SELECT evidence FROM revoke_verifications WHERE id = ?")
            .bind(&verification_id)
            .fetch_one(&journal.database)
            .await?;
    if retained != raw {
        return Err(Error::ReplayMismatch);
    }
    Ok(VerifiedRevoke {
        verification_id,
        state_version: evidence.state_version,
        transaction: evidence.transaction,
        observation,
        changes,
    })
}

pub fn action_router(journal: Journal, client: RpcClient) -> Router {
    Router::new()
        .route("/snapshots/:id/actions/revoke/:source", post(revoke))
        .route("/snapshots/:id/actions/revoke/:source/verify", post(verify))
        .route(
            "/scopes/:scope_id/actions/revoke/:source",
            post(current_revoke),
        )
        .with_state((journal, Arc::new(client)))
}

async fn current_revoke(
    State((journal, client)): State<(Journal, Arc<RpcClient>)>,
    Path((scope_id, source)): Path<(String, String)>,
) -> Result<Json<SimulatedRevoke>, (StatusCode, Json<serde_json::Value>)> {
    let source = source.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid token account address"})),
        )
    })?;
    simulate_current_revoke(&client, &journal, &scope_id, source)
        .await
        .map(Json)
        .map_err(action_error)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedRevoke {
    signed_transaction: String,
}

async fn verify(
    State((journal, client)): State<(Journal, Arc<RpcClient>)>,
    Path((id, source)): Path<(String, String)>,
    Json(request): Json<SignedRevoke>,
) -> Result<Json<VerifiedRevoke>, (StatusCode, Json<serde_json::Value>)> {
    let source = source.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid token account address"})),
        )
    })?;
    verify_revoke(&client, &journal, &id, source, &request.signed_transaction)
        .await
        .map(Json)
        .map_err(action_error)
}

async fn revoke(
    State((journal, client)): State<(Journal, Arc<RpcClient>)>,
    Path((id, source)): Path<(String, String)>,
) -> Result<Json<SimulatedRevoke>, (StatusCode, Json<serde_json::Value>)> {
    let source = source.parse().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid token account address"})),
        )
    })?;
    simulate_revoke(&client, &journal, &id, source)
        .await
        .map(Json)
        .map_err(action_error)
}

fn action_error(error: Error) -> (StatusCode, Json<serde_json::Value>) {
    let status = match &error {
        Error::NotFound => StatusCode::NOT_FOUND,
        Error::Conflict
        | Error::AmbiguousScope
        | Error::InvalidObservation("simulation/context slot mismatch") => StatusCode::CONFLICT,
        Error::Rpc(_) => StatusCode::BAD_GATEWAY,
        Error::Adapter(_) | Error::InvalidObservation(_) | Error::SimulationFailed(_) => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let body = match error {
        Error::SimulationFailed(response) => {
            serde_json::json!({"error": "Native simulation failed", "response": response})
        }
        _ if status == StatusCode::INTERNAL_SERVER_ERROR => {
            serde_json::json!({"error": "Journal verification failed"})
        }
        Error::Rpc(_) => serde_json::json!({"error": "RPC provider unavailable"}),
        error => serde_json::json!({"error": error.to_string()}),
    };
    (status, Json(body))
}

pub(crate) fn verify_program(
    snapshot: &Snapshot,
    version: &ProgramVersion,
) -> Result<(), AdapterError> {
    let observed = &snapshot.observation;
    if clock(&snapshot.accounts, observed.slot)?.unix_timestamp != observed.unix_timestamp {
        return Err(AdapterError::InvalidState(
            "clock timestamp mismatch".into(),
        ));
    }
    let account = |key| {
        snapshot
            .accounts
            .iter()
            .find(|(address, _)| *address == key)
            .map(|(_, account)| account)
            .ok_or(AdapterError::InsufficientEvidence)
    };
    let program = account(version.program_id)?;
    if program.owner != bpf_loader_upgradeable::id() {
        return Err(AdapterError::UnsupportedVersion);
    }
    if program.lamports == 0
        || !program.executable
        || program.data.len() != UpgradeableLoaderState::size_of_program()
    {
        return Err(AdapterError::InvalidState("loader program account".into()));
    }
    let state: UpgradeableLoaderState = bincode::deserialize(&program.data)
        .map_err(|_| AdapterError::InvalidState("loader program data".into()))?;
    let UpgradeableLoaderState::Program {
        programdata_address,
    } = state
    else {
        return Err(AdapterError::InvalidState("loader program state".into()));
    };
    if programdata_address != get_program_data_address(&version.program_id) {
        return Err(AdapterError::InvalidState(
            "loader programdata address".into(),
        ));
    }
    let data = account(programdata_address)?;
    let offset = UpgradeableLoaderState::size_of_programdata_metadata();
    if data.lamports == 0
        || data.owner != bpf_loader_upgradeable::id()
        || data.executable
        || data.data.len() <= offset
    {
        return Err(AdapterError::InvalidState(
            "loader programdata account".into(),
        ));
    }
    let state: UpgradeableLoaderState = bincode::deserialize(&data.data[..offset])
        .map_err(|_| AdapterError::InvalidState("loader programdata metadata".into()))?;
    let UpgradeableLoaderState::ProgramData { slot, .. } = state else {
        return Err(AdapterError::InvalidState(
            "loader programdata state".into(),
        ));
    };
    // Loader-v3 activates new code in the slot after its deployment.
    if slot >= observed.slot
        || version.deployment != format!("solana:loader-v3:{programdata_address}:{slot}")
        || version.version != format!("sha256:{:x}", Sha256::digest(&data.data[offset..]))
    {
        return Err(AdapterError::UnsupportedVersion);
    }
    let identity =
        executable_identity(&data.data[offset..]).map_err(|_| AdapterError::UnsupportedVersion)?;
    version
        .verify_execution(programdata_address, observed.slot, &identity)
        .map_err(|error| match error {
            cataloger::Error::InsufficientExecutableEvidence => AdapterError::InsufficientEvidence,
            _ => AdapterError::UnsupportedVersion,
        })?;
    Ok(())
}

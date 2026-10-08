use crate::{Error, Observation, Origin, Snapshot, Target};
use cataloger::ProgramVersion;
use catalyst_sdk::{Adapter, Error as AdapterError, spl, subscriptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solana_account::Account;
use solana_account_decoder_client_types::UiAccountEncoding;
use solana_clock::Clock;
use solana_loader_v3_interface::{get_program_data_address, state::UpgradeableLoaderState};
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::config::RpcAccountInfoConfig;
use solana_sdk_ids::{bpf_loader_upgradeable, sysvar};
use std::collections::BTreeSet;

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
    Ok(())
}

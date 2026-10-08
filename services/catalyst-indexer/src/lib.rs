use arm::{Authorization, EvidenceBundle, NativeContext, Principal, Subject};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use cataloger::{Catalog, ProgramVersion};
use catalyst_sdk::{self as sdk, Adapter, Context, spl, subscriptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solana_account::Account;
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use spl_token_interface::state::Account as TokenAccount;
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use std::{
    collections::{BTreeMap, btree_map::Entry},
    str::FromStr,
    time::Duration,
};

pub const SDK_REVISION: &str = "f6f47781e53a9995a15882e626adea1fcdd69d44";
pub const RUNTIME_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Fixture,
    Finalized,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub cluster: String,
    pub bank: String,
    pub slot: u64,
    pub unix_timestamp: i64,
    pub origin: Origin,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    SplDelegate {
        source: Pubkey,
    },
    Subscription {
        delegation: Pubkey,
        authority: Pubkey,
        source: Pubkey,
        mint: Pubkey,
        plan: Option<Pubkey>,
    },
}

impl Target {
    fn key(&self) -> String {
        match self {
            Self::SplDelegate { source } => format!("spl:{source}"),
            Self::Subscription { delegation, .. } => format!("subscription:{delegation}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub observation: Observation,
    pub accounts: Vec<(Pubkey, Account)>,
    pub targets: Vec<Target>,
}

impl Snapshot {
    fn normalize(&mut self) -> Result<(), Error> {
        if self.observation.cluster.is_empty()
            || self.observation.bank.is_empty()
            || self.accounts.is_empty()
            || self.targets.is_empty()
        {
            return Err(Error::InvalidSnapshot);
        }
        self.accounts.sort_by_key(|(key, _)| *key);
        self.targets.sort_by_key(Target::key);
        if self.accounts.windows(2).any(|pair| pair[0].0 == pair[1].0)
            || self
                .targets
                .windows(2)
                .any(|pair| pair[0].key() == pair[1].key())
        {
            return Err(Error::InvalidSnapshot);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Projection {
    Compiled { authorizations: Vec<Authorization> },
    Unsupported { reason: String },
    Incomplete { reason: String },
    Invalid { reason: String },
}

impl From<sdk::Error> for Projection {
    fn from(error: sdk::Error) -> Self {
        let reason = error.to_string();
        match error {
            sdk::Error::UnsupportedVersion | sdk::Error::UnsupportedOperation => {
                Self::Unsupported { reason }
            }
            sdk::Error::InsufficientEvidence => Self::Incomplete { reason },
            sdk::Error::InvalidState(_) | sdk::Error::InvalidProjection => Self::Invalid { reason },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub coverage_complete: bool,
    pub snapshot: Snapshot,
    pub versions: Vec<ProgramVersion>,
    pub projection: Projection,
    pub runtime_version: String,
    pub sdk_revision: String,
}

fn context(
    program_id: Pubkey,
    snapshot: &Snapshot,
    catalog: &Catalog,
    id: &str,
    versions: &mut Vec<ProgramVersion>,
) -> Result<Context, sdk::Error> {
    let observed = &snapshot.observation;
    let record = catalog
        .resolve(&observed.cluster, &program_id, observed.slot, None)
        .map_err(|_| sdk::Error::UnsupportedVersion)?;
    if !versions.contains(record) {
        versions.push(record.clone());
    }
    // Current adapters are verified against local ELFs, not a finalized live deployment.
    if observed.origin == Origin::Finalized && record.deployment.starts_with("fixture:") {
        return Err(sdk::Error::UnsupportedVersion);
    }
    Ok(Context {
        program_id,
        native: NativeContext {
            protocol: record.adapter.protocol.clone(),
            deployment: record.deployment.clone(),
            program_version: record.version.clone(),
            adapter_version: record.adapter.version.clone(),
        },
        evidence: EvidenceBundle {
            references: vec![
                format!("journal:sha256:{id}"),
                record.provenance.evidence_reference.clone(),
                format!("catalyst-sdk:git:{SDK_REVISION}"),
            ],
            observed_at: format!("{}:{}:{}", observed.cluster, observed.bank, observed.slot),
        },
    })
}

fn compile(snapshot: &Snapshot, catalog: &Catalog, id: &str) -> (Vec<ProgramVersion>, Projection) {
    let accounts: BTreeMap<_, _> = snapshot
        .accounts
        .iter()
        .map(|(key, value)| (*key, value))
        .collect();
    let account = |key: Pubkey| {
        accounts
            .get(&key)
            .map(|value| (*value).clone())
            .ok_or(sdk::Error::InsufficientEvidence)
    };
    let mut versions = Vec::new();
    let result = (|| {
        let mut authorizations = BTreeMap::new();
        for target in &snapshot.targets {
            let compiled = match target {
                Target::SplDelegate { source } => {
                    let adapter = spl::DelegateAdapter;
                    let context = context(
                        adapter.protocol().program_id,
                        snapshot,
                        catalog,
                        id,
                        &mut versions,
                    )?;
                    if !adapter.supports(&context.native) {
                        return Err(sdk::Error::UnsupportedVersion);
                    }
                    let mut state = spl::State {
                        address: *source,
                        account: account(*source)?,
                        owner: Account::default(),
                        delegate: Account::default(),
                    };
                    if state.account.owner == context.program_id && !state.account.executable {
                        let native = TokenAccount::unpack(&state.account.data)
                            .map_err(|error| sdk::Error::InvalidState(error.to_string()))?;
                        state.owner = account(native.owner)?;
                        state.delegate = Option::<Pubkey>::from(native.delegate)
                            .map(account)
                            .transpose()?
                            .unwrap_or_default();
                    }
                    sdk::compile_state(&adapter, &state, &context)?
                }
                Target::Subscription {
                    delegation,
                    authority,
                    source,
                    mint,
                    plan,
                } => {
                    let adapter = subscriptions::DelegationAdapter;
                    let grant_context = context(
                        adapter.protocol().program_id,
                        snapshot,
                        catalog,
                        id,
                        &mut versions,
                    )?;
                    if !adapter.supports(&grant_context.native) {
                        return Err(sdk::Error::UnsupportedVersion);
                    }
                    let token_adapter = spl::DelegateAdapter;
                    let token_context = context(
                        token_adapter.protocol().program_id,
                        snapshot,
                        catalog,
                        id,
                        &mut versions,
                    )?;
                    if !token_adapter.supports(&token_context.native) {
                        return Err(sdk::Error::UnsupportedVersion);
                    }
                    let mut state = subscriptions::DelegationState {
                        delegation: (*delegation, account(*delegation)?),
                        authority: (*authority, account(*authority)?),
                        source: spl::AuthorityState {
                            address: *source,
                            account: account(*source)?,
                            authorities: Vec::new(),
                        },
                        mint: (*mint, account(*mint)?),
                        plan: plan
                            .map(|key| account(key).map(|value| (key, value)))
                            .transpose()?,
                        token_program_version: token_context.native.program_version,
                        unix_timestamp: Some(snapshot.observation.unix_timestamp),
                    };
                    for required in adapter.source_requirements(&state).accounts {
                        state
                            .source
                            .authorities
                            .push((required, account(required)?));
                    }
                    sdk::compile_state(&adapter, &state, &grant_context)?
                }
            };
            for authorization in compiled {
                match authorizations.entry(authorization.id.clone()) {
                    Entry::Vacant(entry) => {
                        entry.insert(authorization);
                    }
                    Entry::Occupied(entry) => {
                        if entry.get() != &authorization {
                            return Err(sdk::Error::InvalidProjection);
                        }
                    }
                }
            }
        }
        Ok(authorizations.into_values().collect())
    })();
    versions.sort_by_key(|record| record.program_id);
    let projection = match result {
        Ok(authorizations) => Projection::Compiled { authorizations },
        Err(error) => Projection::from(error),
    };
    (versions, projection)
}

#[derive(Debug)]
pub enum Error {
    Database(sqlx::Error),
    Json(serde_json::Error),
    InvalidSnapshot,
    Conflict,
    ReplayMismatch,
    NotFound,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(error) => error.fmt(f),
            Self::Json(error) => error.fmt(f),
            _ => write!(f, "{self:?}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<sqlx::Error> for Error {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Clone)]
pub struct Journal {
    database: SqlitePool,
}

impl Journal {
    pub async fn open(url: &str) -> Result<Self, Error> {
        let options = SqliteConnectOptions::from_str(url)?
            .create_if_missing(true)
            .busy_timeout(Duration::from_secs(5))
            .pragma("journal_mode", "WAL")
            .pragma("synchronous", "FULL");
        let database = SqlitePool::connect_with(options).await?;
        sqlx::migrate!("./migrations")
            .run(&database)
            .await
            .map_err(sqlx::Error::from)?;
        Ok(Self { database })
    }

    pub async fn ingest(&self, mut snapshot: Snapshot, catalog: &Catalog) -> Result<Record, Error> {
        snapshot.normalize()?;
        let id = format!("{:x}", Sha256::digest(serde_json::to_vec(&snapshot)?));
        let (versions, projection) = compile(&snapshot, catalog, &id);
        let record = Record {
            id,
            coverage_complete: false,
            snapshot,
            versions,
            projection,
            runtime_version: RUNTIME_VERSION.into(),
            sdk_revision: SDK_REVISION.into(),
        };
        let raw = serde_json::to_string(&record)?;
        let inserted = sqlx::query(
            "INSERT INTO authority_snapshots (id, record, record_sha256) VALUES (?, ?, ?) ON CONFLICT(id) DO NOTHING",
        )
        .bind(&record.id)
        .bind(&raw)
        .bind(format!("{:x}", Sha256::digest(raw.as_bytes())))
        .execute(&self.database)
        .await?
        .rows_affected();
        if inserted == 0 && self.get(&record.id).await?.as_ref() != Some(&record) {
            return Err(Error::Conflict);
        }
        Ok(record)
    }

    pub async fn get(&self, id: &str) -> Result<Option<Record>, Error> {
        let raw: Option<(String, String)> =
            sqlx::query_as("SELECT record, record_sha256 FROM authority_snapshots WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.database)
                .await?;
        let Some((raw, checksum)) = raw else {
            return Ok(None);
        };
        if format!("{:x}", Sha256::digest(raw.as_bytes())) != checksum {
            return Err(Error::ReplayMismatch);
        }
        let record: Record = serde_json::from_str(&raw)?;
        if record.id != id
            || record.coverage_complete
            || format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&record.snapshot)?)
            ) != id
        {
            return Err(Error::ReplayMismatch);
        }
        Ok(Some(record))
    }

    pub async fn replay(&self, id: &str) -> Result<Record, Error> {
        let record = self.get(id).await?.ok_or(Error::NotFound)?;
        let catalog = Catalog::new(record.versions.clone()).map_err(|_| Error::ReplayMismatch)?;
        let (versions, projection) = compile(&record.snapshot, &catalog, id);
        if record.sdk_revision != SDK_REVISION
            || record.runtime_version != RUNTIME_VERSION
            || record.versions != versions
            || record.projection != projection
        {
            return Err(Error::ReplayMismatch);
        }
        Ok(record)
    }
}

#[derive(Serialize)]
pub struct Coverage {
    pub complete: bool,
    pub accounts: Vec<Pubkey>,
    pub targets: Vec<Target>,
}

#[derive(Serialize)]
pub struct AuthorizationView {
    pub state_version: String,
    pub observation: Observation,
    pub coverage: Coverage,
    pub versions: Vec<ProgramVersion>,
    pub projection: Projection,
    pub runtime_version: String,
    pub sdk_revision: String,
    pub arm_schema_version: &'static str,
}

fn principal_contains(principal: &Principal, address: &str) -> bool {
    match principal {
        Principal::Identity(identity) => identity == address,
        Principal::AnyOf(principals) | Principal::AllOf(principals) => principals
            .iter()
            .any(|principal| principal_contains(principal, address)),
    }
}

fn mentions(authorization: &Authorization, address: &str) -> bool {
    principal_contains(&authorization.principal, address)
        || authorization.resource.id == address
        || match &authorization.subject {
            Subject::Identity(identity) => identity == address,
            Subject::Resource(resource) => resource.id == address,
        }
}

async fn snapshot(
    State(journal): State<Journal>,
    Path(id): Path<String>,
) -> Result<Json<Record>, StatusCode> {
    journal
        .get(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn authorizations(
    State(journal): State<Journal>,
    Path((id, address)): Path<(String, String)>,
) -> Result<Json<AuthorizationView>, StatusCode> {
    let address = address
        .parse::<Pubkey>()
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .to_string();
    let record = journal
        .get(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let projection = match record.projection {
        Projection::Compiled { authorizations } => Projection::Compiled {
            authorizations: authorizations
                .into_iter()
                .filter(|authorization| mentions(authorization, &address))
                .collect(),
        },
        projection => projection,
    };
    Ok(Json(AuthorizationView {
        state_version: record.id,
        observation: record.snapshot.observation,
        coverage: Coverage {
            complete: false,
            accounts: record
                .snapshot
                .accounts
                .into_iter()
                .map(|(key, _)| key)
                .collect(),
            targets: record.snapshot.targets,
        },
        versions: record.versions,
        projection,
        runtime_version: record.runtime_version,
        sdk_revision: record.sdk_revision,
        arm_schema_version: arm::SCHEMA_VERSION,
    }))
}

pub fn router(journal: Journal) -> Router {
    Router::new()
        .route("/snapshots/:id", get(snapshot))
        .route(
            "/snapshots/:id/authorizations/:address",
            get(authorizations),
        )
        .with_state(journal)
}

//! Deterministic resolution of verified, bounded native deployment history.

use arm::{Capability, NativeContext};
use solana_pubkey::Pubkey;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaSource {
    pub uri: String,
    pub revision: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterRef {
    pub protocol: String,
    pub version: String,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    pub source: String,
    pub revision: String,
    pub evidence_reference: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramVersion {
    /// Genesis hash, rather than a mutable provider label such as "mainnet".
    pub cluster: String,
    pub program_id: Pubkey,
    pub deployment: String,
    pub version: String,
    pub start_slot: u64,
    /// Exclusive, bounded by verified coverage. No inference about future upgrades.
    pub end_slot: u64,
    pub schema: SchemaSource,
    pub adapter: AdapterRef,
    pub supported_semantics: Vec<Capability>,
    pub provenance: Provenance,
}

impl ProgramVersion {
    pub fn native_context(&self) -> NativeContext {
        NativeContext {
            protocol: self.adapter.protocol.clone(),
            deployment: self.deployment.clone(),
            program_version: self.version.clone(),
            adapter_version: self.adapter.version.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    UnsupportedVersion,
    InvalidRecord,
    OverlappingHistory,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Debug)]
pub struct Catalog {
    versions: Vec<ProgramVersion>,
}

impl Catalog {
    pub fn new(mut versions: Vec<ProgramVersion>) -> Result<Self, Error> {
        for version in &versions {
            if version.start_slot >= version.end_slot
                || [
                    &version.cluster,
                    &version.deployment,
                    &version.version,
                    &version.schema.uri,
                    &version.schema.revision,
                    &version.adapter.protocol,
                    &version.adapter.version,
                    &version.adapter.source,
                    &version.provenance.source,
                    &version.provenance.revision,
                    &version.provenance.evidence_reference,
                ]
                .iter()
                .any(|value| value.is_empty())
            {
                return Err(Error::InvalidRecord);
            }
            if version
                .supported_semantics
                .iter()
                .enumerate()
                .any(|(i, capability)| version.supported_semantics[..i].contains(capability))
            {
                return Err(Error::InvalidRecord);
            }
        }
        versions.sort_by(|left, right| {
            (&left.cluster, left.program_id, left.start_slot).cmp(&(
                &right.cluster,
                right.program_id,
                right.start_slot,
            ))
        });
        for pair in versions.windows(2) {
            if pair[0].cluster == pair[1].cluster
                && pair[0].program_id == pair[1].program_id
                && pair[0].end_slot > pair[1].start_slot
            {
                return Err(Error::OverlappingHistory);
            }
        }
        Ok(Self { versions })
    }

    pub fn resolve(
        &self,
        cluster: &str,
        program_id: &Pubkey,
        slot: u64,
        version: Option<&str>,
    ) -> Result<&ProgramVersion, Error> {
        self.versions
            .iter()
            .find(|known| {
                known.cluster == cluster
                    && &known.program_id == program_id
                    && known.start_slot <= slot
                    && slot < known.end_slot
                    && version.is_none_or(|requested| requested == known.version)
            })
            .ok_or(Error::UnsupportedVersion)
    }
}

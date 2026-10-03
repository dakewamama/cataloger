//! Deterministic resolution of verified, bounded native deployment history.

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
    pub supported_from_slot: u64,
    /// Verification boundary, not evidence that the native deployment ended here.
    pub supported_until_slot_exclusive: u64,
    pub schema: SchemaSource,
    pub adapter: AdapterRef,
    pub provenance: Provenance,
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
            if version.supported_from_slot >= version.supported_until_slot_exclusive
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
        }
        versions.sort_by(|left, right| {
            (&left.cluster, left.program_id, left.supported_from_slot).cmp(&(
                &right.cluster,
                right.program_id,
                right.supported_from_slot,
            ))
        });
        for pair in versions.windows(2) {
            if pair[0].cluster == pair[1].cluster
                && pair[0].program_id == pair[1].program_id
                && pair[0].supported_until_slot_exclusive > pair[1].supported_from_slot
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
                    && known.supported_from_slot <= slot
                    && slot < known.supported_until_slot_exclusive
                    && version.is_none_or(|requested| requested == known.version)
            })
            .ok_or(Error::UnsupportedVersion)
    }
}

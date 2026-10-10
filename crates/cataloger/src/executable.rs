use crate::{Error, ProgramVersion};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solana_pubkey::Pubkey;
use solana_sbpf::{
    aligned_memory::AlignedMemory,
    elf::Executable,
    elf_parser::Elf64,
    program::{BuiltinProgram, SBPFVersion},
    verifier::RequisiteVerifier,
    vm::ContextObject,
};
use std::{collections::BTreeMap, sync::Arc};

pub const SBPF_REVISION: &str = "acd2c551a0f8df2a8f077a6e4b55f546d4deb98d";
pub const AGAVE_REVISION: &str = "7bc9c805218ca06769956e2cb61601329f5a0f6c";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutableIdentity {
    pub raw_elf_hash: String,
    pub sbpf_version: u32,
    pub relocations: BTreeMap<u32, u64>,
    pub loader_revision: String,
    pub environment_hash: String,
    pub loaded_text_hash: String,
    pub loaded_ro_hash: String,
    pub effective_executable_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEvidence {
    pub revision: String,
    pub environment_hash: String,
    pub evidence_reference: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutableEvidence {
    pub programdata_address: Pubkey,
    pub observed_slot: u64,
    pub source_revision: Option<String>,
    pub build_toolchain: Option<String>,
    pub identity: ExecutableIdentity,
    // A local loader result does not establish the observed bank's runtime environment.
    pub runtime: Option<RuntimeEvidence>,
}

fn bytes(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u64).to_le_bytes());
    hash.update(value);
}

/// Fingerprint the upstream loader's verified image under the supplied environment.
/// This does not authenticate that environment against a validator or compile JIT code.
pub fn inspect<C: ContextObject>(
    elf: &[u8],
    loader: Arc<BuiltinProgram<C>>,
) -> Result<ExecutableIdentity, Box<dyn std::error::Error>> {
    let executable = Executable::load(elf, loader.clone())?;
    executable.verify::<RequisiteVerifier>()?;
    let sbpf_version: u32 = match executable.get_sbpf_version() {
        SBPFVersion::V0 => 0,
        SBPFVersion::V3 => 3,
        // Only the legacy and static paths with retained fixtures are supported here.
        _ => return Err(Error::UnsupportedVersion.into()),
    };
    let mut relocations = BTreeMap::new();
    if sbpf_version == 0 {
        let aligned = AlignedMemory::<{ solana_sbpf::ebpf::HOST_ALIGN }>::from_slice(elf);
        let parsed = Elf64::parse(aligned.as_slice())?;
        for relocation in parsed.dynamic_relocations_table().unwrap_or_default() {
            *relocations.entry(relocation.r_type()).or_insert(0) += 1;
        }
    }
    let config = loader.get_config();
    let mut environment = Sha256::new();
    bytes(&mut environment, b"cataloger:sbpf-environment:1");
    bytes(&mut environment, SBPF_REVISION.as_bytes());
    for value in [
        config.max_call_depth,
        config.stack_frame_size,
        config.instruction_meter_checkpoint_distance,
    ] {
        environment.update((value as u64).to_le_bytes());
    }
    environment.update([
        config.enable_address_translation as u8,
        config.enable_stack_frame_gaps as u8,
        config.enable_instruction_meter as u8,
        config.enable_register_tracing as u8,
        config.enable_symbol_and_section_labels as u8,
        config.reject_broken_elfs as u8,
        config.optimize_rodata as u8,
        config.aligned_memory_mapping as u8,
        *config.enabled_sbpf_versions.start() as u8,
        *config.enabled_sbpf_versions.end() as u8,
    ]);
    for (key, (name, _)) in loader.get_function_registry().iter() {
        environment.update(key.to_le_bytes());
        bytes(&mut environment, name);
    }
    let environment_hash = format!("sha256:{:x}", environment.finalize());
    let mut image = Sha256::new();
    bytes(&mut image, b"cataloger:sbpf-image:1");
    bytes(&mut image, environment_hash.as_bytes());
    image.update(sbpf_version.to_le_bytes());
    image.update((executable.get_entrypoint_instruction_offset() as u64).to_le_bytes());
    image.update(executable.get_ro_region().vm_addr.to_le_bytes());
    bytes(&mut image, executable.get_ro_section());
    let (address, text) = executable.get_text_bytes();
    image.update(address.to_le_bytes());
    bytes(&mut image, text);
    for (key, (_, pc)) in executable.get_function_registry().iter() {
        image.update(key.to_le_bytes());
        image.update((pc as u64).to_le_bytes());
    }
    Ok(ExecutableIdentity {
        raw_elf_hash: format!("sha256:{:x}", Sha256::digest(elf)),
        sbpf_version,
        relocations,
        loader_revision: SBPF_REVISION.into(),
        environment_hash,
        loaded_text_hash: format!("sha256:{:x}", Sha256::digest(text)),
        loaded_ro_hash: format!("sha256:{:x}", Sha256::digest(executable.get_ro_section())),
        effective_executable_hash: format!("sha256:{:x}", image.finalize()),
    })
}

impl ProgramVersion {
    pub fn verify_execution(
        &self,
        programdata_address: Pubkey,
        observed_slot: u64,
        actual: &ExecutableIdentity,
    ) -> Result<(), Error> {
        let evidence = self
            .executable
            .as_ref()
            .ok_or(Error::InsufficientExecutableEvidence)?;
        if evidence.programdata_address != programdata_address
            || evidence.observed_slot > observed_slot
            || observed_slot < self.supported_from_slot
            || observed_slot >= self.supported_until_slot_exclusive
            || self.version != actual.raw_elf_hash
            || evidence.identity != *actual
        {
            return Err(Error::UnsupportedVersion);
        }
        let runtime = evidence
            .runtime
            .as_ref()
            .ok_or(Error::InsufficientExecutableEvidence)?;
        if runtime.revision != AGAVE_REVISION || runtime.environment_hash != actual.environment_hash
        {
            return Err(Error::UnsupportedVersion);
        }
        if runtime.evidence_reference.is_empty()
            || evidence
                .source_revision
                .as_ref()
                .is_none_or(String::is_empty)
            || evidence
                .build_toolchain
                .as_ref()
                .is_none_or(String::is_empty)
        {
            return Err(Error::InsufficientExecutableEvidence);
        }
        Ok(())
    }
}

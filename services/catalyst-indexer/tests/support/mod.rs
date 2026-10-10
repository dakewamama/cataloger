use cataloger::{
    ProgramVersion,
    executable::{AGAVE_REVISION, ExecutableEvidence, RuntimeEvidence},
};
use catalyst_indexer::rpc;
use solana_loader_v3_interface::get_program_data_address;

// These operator attestations exercise trust checks; they are not evidence about a public bank.
pub fn attest_fixture(version: &mut ProgramVersion, elf: &[u8], observed_slot: u64) {
    let programdata_address = get_program_data_address(&version.program_id);
    let identity = rpc::executable_identity(elf).unwrap();
    version.executable = Some(ExecutableEvidence {
        programdata_address,
        observed_slot,
        source_revision: Some("fixture:source-attestation".into()),
        build_toolchain: Some("fixture:build-attestation".into()),
        runtime: Some(RuntimeEvidence {
            revision: AGAVE_REVISION.into(),
            environment_hash: identity.environment_hash.clone(),
            evidence_reference: "fixture:runtime-attestation".into(),
        }),
        identity,
    });
}

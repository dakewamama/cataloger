use cataloger::{
    AdapterRef, Catalog, Error, ProgramVersion, Provenance, SchemaSource,
    executable::{AGAVE_REVISION, ExecutableEvidence, RuntimeEvidence, SBPF_REVISION},
};
use catalyst_indexer::rpc;
use serde_json::Value;
use sha2::{Digest, Sha256};
use solana_account::Account;
use solana_account_decoder_client_types::UiAccount;
use solana_loader_v3_interface::state::UpgradeableLoaderState;

fn elf() -> Vec<u8> {
    let response: Value =
        serde_json::from_str(include_str!("fixtures/devnet-subscriptions-response.json")).unwrap();
    let account: UiAccount =
        serde_json::from_value(response["result"]["value"][1].clone()).unwrap();
    let account: Account = account.to_account().unwrap();
    account.data[UpgradeableLoaderState::size_of_programdata_metadata()..].to_vec()
}

fn evidence() -> ExecutableEvidence {
    serde_json::from_str(include_str!("fixtures/subscriptions-executable.json")).unwrap()
}

fn version() -> ProgramVersion {
    ProgramVersion {
        cluster: "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG".into(),
        program_id: "De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44"
            .parse()
            .unwrap(),
        deployment: "solana:loader-v3:HaYb5J9eXooZuNzN3z6TfuzVDcaTfiDdDPWCFtexFfMg:506642674"
            .into(),
        version: evidence().identity.raw_elf_hash,
        supported_from_slot: 509_022_453,
        supported_until_slot_exclusive: 509_022_454,
        schema: SchemaSource {
            uri: "https://docs.rs/subscriptions/0.5.0".into(),
            revision: "0.5.0".into(),
        },
        adapter: AdapterRef {
            protocol: "subscriptions".into(),
            version: "0.3".into(),
            source: "fixture:adapter".into(),
        },
        provenance: Provenance {
            source: "fixture:finalized-capture".into(),
            revision: evidence().source_revision.unwrap(),
            evidence_reference: "fixture:devnet-subscriptions-response.json".into(),
        },
        executable: Some(evidence()),
    }
}

#[test]
fn captured_elf_matches_the_loaded_golden_and_reproduced_build_hash() {
    let elf = elf();
    let actual = rpc::executable_identity(&elf).unwrap();
    assert_eq!(actual, evidence().identity);
    assert_eq!(actual, rpc::executable_identity(&elf).unwrap());
    assert_eq!(actual.loader_revision, SBPF_REVISION);
    assert_eq!(actual.sbpf_version, 0);
    assert_eq!(
        actual.relocations.into_iter().collect::<Vec<_>>(),
        [(8, 250), (10, 48)]
    );
    let unpadded = elf.iter().rposition(|byte| *byte != 0).unwrap() + 1;
    assert_eq!(
        format!("{:x}", Sha256::digest(&elf[..unpadded])),
        "e705f5a309f84f849b402f20de4bea5f2cc1d1d4f691ba7caabcb07c8b46af51"
    );
    let raw_text = format!("sha256:{:x}", Sha256::digest(&elf[0x120..0x1d3b8]));
    assert_ne!(actual.loaded_text_hash, raw_text);
}

#[test]
fn padding_changes_raw_identity_without_changing_the_loaded_image() {
    let mut changed = elf();
    *changed.last_mut().unwrap() ^= 1;
    let changed = rpc::executable_identity(&changed).unwrap();
    assert_ne!(changed.raw_elf_hash, evidence().identity.raw_elf_hash);
    assert_eq!(
        changed.effective_executable_hash,
        evidence().identity.effective_executable_hash
    );
}

#[test]
fn relocation_metadata_changes_loaded_data_without_changing_raw_code_or_data() {
    let original = elf();
    let mut changed = original.clone();
    // Redirect one real RELATIVE relocation into raw read-only data; this is not deployment evidence.
    changed[0x1e368..0x1e370].copy_from_slice(&0x1d3b8_u64.to_le_bytes());
    assert_eq!(original[0x120..0x1e0f0], changed[0x120..0x1e0f0]);
    let changed = rpc::executable_identity(&changed).unwrap();
    assert_ne!(changed.loaded_ro_hash, evidence().identity.loaded_ro_hash);
    assert_ne!(
        changed.effective_executable_hash,
        evidence().identity.effective_executable_hash
    );
}

#[test]
fn malformed_unknown_versions_and_unknown_relocations_fail_closed() {
    for bytes in [&[][..], b"not an ELF", &elf()[..64]] {
        assert!(rpc::executable_identity(bytes).is_err());
    }
    for flags in [1_u32, 2, 4, 5, u32::MAX] {
        let mut changed = elf();
        changed[48..52].copy_from_slice(&flags.to_le_bytes());
        assert!(rpc::executable_identity(&changed).is_err(), "flags {flags}");
    }
    let mut changed = elf();
    changed[0x1e370..0x1e374].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(rpc::executable_identity(&changed).is_err());
    let mut changed = elf();
    changed[0x1e368..0x1e370].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(rpc::executable_identity(&changed).is_err());
}

#[test]
fn raw_identity_and_local_loading_cannot_establish_live_execution_support() {
    let mut version = version();
    let evidence = evidence();
    assert_eq!(
        version.verify_execution(
            evidence.programdata_address,
            evidence.observed_slot,
            &evidence.identity
        ),
        Err(Error::InsufficientExecutableEvidence)
    );
    version.executable = None;
    assert_eq!(
        version.verify_execution(
            evidence.programdata_address,
            evidence.observed_slot,
            &evidence.identity
        ),
        Err(Error::InsufficientExecutableEvidence)
    );
    assert!(
        Catalog::new(vec![version.clone()])
            .unwrap()
            .resolve(
                &version.cluster,
                &version.program_id,
                evidence.observed_slot,
                None
            )
            .is_ok()
    );
}

#[test]
fn independent_runtime_attestation_is_required_and_cannot_cross_an_upgrade() {
    let mut version = version();
    let evidence = evidence();
    // Exercise the operator-attestation contract, without claiming that devnet supplied this witness.
    version.executable.as_mut().unwrap().runtime = Some(RuntimeEvidence {
        revision: AGAVE_REVISION.into(),
        environment_hash: evidence.identity.environment_hash.clone(),
        evidence_reference: "fixture:operator-runtime-attestation".into(),
    });
    assert_eq!(
        version.verify_execution(
            evidence.programdata_address,
            evidence.observed_slot,
            &evidence.identity
        ),
        Ok(())
    );
    for field in 0..6 {
        let mut unknown = version.clone();
        let record = unknown.executable.as_mut().unwrap();
        match field {
            0 => record.identity.effective_executable_hash.push('0'),
            1 => record.runtime.as_mut().unwrap().revision = "unknown".into(),
            2 => record.runtime.as_mut().unwrap().environment_hash.push('0'),
            3 => record.programdata_address = Default::default(),
            4 => record.source_revision = None,
            _ => record.build_toolchain = None,
        }
        assert!(
            unknown
                .verify_execution(
                    evidence.programdata_address,
                    evidence.observed_slot,
                    &evidence.identity
                )
                .is_err()
        );
    }
    let mut upgraded = elf();
    *upgraded.last_mut().unwrap() ^= 1;
    let upgraded = rpc::executable_identity(&upgraded).unwrap();
    assert_eq!(
        version.verify_execution(
            evidence.programdata_address,
            evidence.observed_slot,
            &upgraded
        ),
        Err(Error::UnsupportedVersion)
    );
    assert!(
        version
            .verify_execution(
                evidence.programdata_address,
                evidence.observed_slot + 1,
                &evidence.identity
            )
            .is_err()
    );
    assert!(
        version
            .verify_execution(
                evidence.programdata_address,
                evidence.observed_slot - 1,
                &evidence.identity
            )
            .is_err()
    );
}

#[test]
fn pinned_static_loader_path_does_not_invent_relocations() {
    let mut elf = vec![0_u8; 320];
    elf[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    for (offset, value) in [
        (16, 3_u16),
        (18, 263),
        (52, 64),
        (54, 56),
        (56, 4),
        (58, 64),
        (60, 1),
    ] {
        elf[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    elf[20..24].copy_from_slice(&1_u32.to_le_bytes());
    elf[32..40].copy_from_slice(&64_u64.to_le_bytes());
    elf[48..52].copy_from_slice(&3_u32.to_le_bytes());
    for (index, flags, address, size) in [
        (0, 1_u32, 0_u64, 24_u64),
        (1, 4, 1 << 32, 8),
        (2, 6, 2 << 32, 0),
        (3, 6, 3 << 32, 0),
    ] {
        let offset = 64 + index * 56;
        elf[offset..offset + 4].copy_from_slice(&1_u32.to_le_bytes());
        elf[offset + 4..offset + 8].copy_from_slice(&flags.to_le_bytes());
        for (field, value) in [
            (8, 288),
            (16, address),
            (24, address),
            (32, size),
            (40, size),
        ] {
            elf[offset + field..offset + field + 8].copy_from_slice(&value.to_le_bytes());
        }
    }
    elf[288..312].copy_from_slice(&[
        0x07, 0x0a, 0, 0, 0xc0, 0xff, 0xff, 0xff, 0xb7, 0, 0, 0, 0, 0, 0, 0, 0x9d, 0, 0, 0, 0, 0,
        0, 0,
    ]);
    let identity = rpc::executable_identity(&elf).unwrap();
    assert_eq!(identity.sbpf_version, 3);
    assert!(identity.relocations.is_empty());
    assert_eq!(
        identity.loaded_text_hash,
        format!("sha256:{:x}", Sha256::digest(&elf[288..312]))
    );
}

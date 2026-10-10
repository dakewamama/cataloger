use cataloger::{AdapterRef, Catalog, Error, ProgramVersion, Provenance, SchemaSource};
use solana_pubkey::Pubkey;

fn record(start: u64, end: u64, version: &str) -> ProgramVersion {
    ProgramVersion {
        cluster: "fixture:genesis".into(),
        program_id: Pubkey::new_from_array([7; 32]),
        deployment: format!("fixture:deployment:{version}"),
        version: version.into(),
        executable: None,
        supported_from_slot: start,
        supported_until_slot_exclusive: end,
        schema: SchemaSource {
            uri: "fixture:schema".into(),
            revision: version.into(),
        },
        adapter: AdapterRef {
            protocol: "fixture".into(),
            version: "0.1".into(),
            source: "fixture:adapter-commit".into(),
        },
        provenance: Provenance {
            source: "fixture:deployment-observation".into(),
            revision: "fixture:commit".into(),
            evidence_reference: "fixture:evidence".into(),
        },
    }
}

#[test]
fn bounded_support_resolves_exact_boundary_slots() {
    let catalog = Catalog::new(vec![record(20, 30, "2"), record(10, 20, "1")]).unwrap();
    let program = Pubkey::new_from_array([7; 32]);
    for (slot, version) in [(10, "1"), (19, "1"), (20, "2"), (29, "2")] {
        let resolved = catalog
            .resolve("fixture:genesis", &program, slot, None)
            .unwrap();
        assert_eq!(resolved.version, version);
    }
    for slot in [0, 9, 30, u64::MAX] {
        assert_eq!(
            catalog.resolve("fixture:genesis", &program, slot, None),
            Err(Error::UnsupportedVersion)
        );
    }
}

#[test]
fn unknown_cluster_program_and_explicit_version_reject() {
    let catalog = Catalog::new(vec![record(10, 20, "1")]).unwrap();
    let program = Pubkey::new_from_array([7; 32]);
    assert_eq!(
        catalog.resolve("other", &program, 10, None),
        Err(Error::UnsupportedVersion)
    );
    assert_eq!(
        catalog.resolve(
            "fixture:genesis",
            &Pubkey::new_from_array([8; 32]),
            10,
            None
        ),
        Err(Error::UnsupportedVersion)
    );
    assert_eq!(
        catalog.resolve("fixture:genesis", &program, 10, Some("future")),
        Err(Error::UnsupportedVersion)
    );
    assert_eq!(
        catalog
            .resolve("fixture:genesis", &program, 10, Some("1"))
            .unwrap()
            .version,
        "1"
    );
}

#[test]
fn resolution_is_independent_of_record_order_and_keeps_provenance() {
    let first = record(10, 20, "1");
    let second = record(20, 30, "2");
    let a = Catalog::new(vec![first.clone(), second.clone()]).unwrap();
    let b = Catalog::new(vec![second, first.clone()]).unwrap();
    let resolved = a
        .resolve(&first.cluster, &first.program_id, 10, None)
        .unwrap();
    assert_eq!(
        resolved,
        b.resolve(&first.cluster, &first.program_id, 10, None)
            .unwrap()
    );
    assert_eq!(resolved, &first);
}

#[test]
fn ambiguous_deployment_history_is_rejected() {
    for records in [
        vec![record(10, 20, "1"), record(19, 30, "2")],
        vec![record(10, 30, "1"), record(15, 20, "2")],
        vec![record(10, 20, "1"), record(10, 20, "1")],
    ] {
        assert_eq!(
            Catalog::new(records).unwrap_err(),
            Error::OverlappingHistory
        );
    }
}

#[test]
fn gaps_and_empty_catalogs_do_not_guess() {
    let catalog = Catalog::new(vec![record(10, 20, "1"), record(30, 40, "1")]).unwrap();
    let key = Pubkey::new_from_array([7; 32]);
    for slot in 20..30 {
        assert_eq!(
            catalog.resolve("fixture:genesis", &key, slot, None),
            Err(Error::UnsupportedVersion)
        );
    }
    assert_eq!(
        catalog
            .resolve("fixture:genesis", &key, 30, None)
            .unwrap()
            .version,
        "1"
    );
    assert_eq!(
        Catalog::new(vec![])
            .unwrap()
            .resolve("fixture:genesis", &key, 10, None),
        Err(Error::UnsupportedVersion)
    );
}

#[test]
fn incomplete_provenance_and_invalid_ranges_reject() {
    for field in 0..4 {
        let mut version = record(10, 20, "1");
        match field {
            0 => version.supported_until_slot_exclusive = 10,
            1 => version.provenance.evidence_reference.clear(),
            2 => version.schema.revision.clear(),
            _ => version.adapter.source.clear(),
        }
        assert_eq!(
            Catalog::new(vec![version]).unwrap_err(),
            Error::InvalidRecord
        );
    }
}

#[test]
fn independent_clusters_and_programs_may_share_slot_ranges() {
    let a = record(10, 20, "1");
    let mut b = a.clone();
    b.cluster = "other-genesis".into();
    let mut c = a.clone();
    c.program_id = Pubkey::new_from_array([8; 32]);
    let catalog = Catalog::new(vec![a, b.clone(), c.clone()]).unwrap();
    assert_eq!(
        catalog
            .resolve(&b.cluster, &b.program_id, 10, None)
            .unwrap(),
        &b
    );
    assert_eq!(
        catalog
            .resolve(&c.cluster, &c.program_id, 10, None)
            .unwrap(),
        &c
    );
}

//! Migration-chain unit tests (split from `migrations.rs` for the
//! SPEC §3 module-size rule).

use super::*;

/// Canonical bytes of an empty component store.
fn empty_store() -> Vec<u8> {
    codec::to_bytes(&Vec::<(u32, u8)>::new()).unwrap()
}

/// Applies a sequence of event-registration extensions (the exact
/// transformation the migration chain performs).
fn chain_extensions(original: &[u8], extensions: &[&[&str]]) -> Vec<u8> {
    let mut bytes = original.to_vec();
    for names in extensions {
        bytes = core_events::extend_registration_bytes(&bytes, names).unwrap();
    }
    bytes
}

/// Every component name the chain appends after a v2-era store list.
fn all_appended() -> Vec<&'static str> {
    V3_ADDED_COMPONENTS
        .iter()
        .chain(V4_ADDED_COMPONENTS.iter())
        .chain(V5_ADDED_COMPONENTS.iter())
        .chain(V6_ADDED_COMPONENTS.iter())
        .chain(V7_ADDED_COMPONENTS.iter())
        .chain(V8_ADDED_COMPONENTS.iter())
        .copied()
        .collect()
}

#[test]
fn v1_bodies_chain_migrate_to_current() {
    let v1 = SaveBodyV1 {
        seed: Seed::new(9),
        tick: Ticks::new(77),
        entities: vec![1, 2],
        rng: vec![3],
        components: vec![("a".into(), vec![4])],
    };
    let raw = codec::to_bytes(&v1).unwrap();
    let current = migrate_to_current(1, raw).unwrap();
    assert_eq!(current.seed, Seed::new(9));
    assert_eq!(current.tick, Ticks::new(77));
    assert_eq!(current.entities, vec![1, 2]);
    assert_eq!(current.rng, vec![3]);
    // Original components pass through verbatim, then every appended
    // store from the whole chain (v3 set, then v4 set), all empty.
    assert_eq!(current.components[0], ("a".to_owned(), vec![4]));
    let appended = all_appended();
    assert_eq!(current.components.len(), 1 + appended.len());
    for (i, name) in appended.iter().enumerate() {
        assert_eq!(
            current.components[1 + i],
            ((*name).to_owned(), empty_store())
        );
    }
    // The v1 empty-events marker survives the whole chain.
    assert!(current.events.is_empty());
}

#[test]
fn v2_bodies_migrate_to_v3_extending_event_registration() {
    // A v2 events blob with the Phase 1 registration and live state.
    let mut events = core_events::Events::new(8);
    // The historical v2 registration (fixture events only) — private
    // event types are irrelevant; only names matter for this test, so
    // reuse local stand-ins with the historical names.
    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct Churn(u32);
    impl core_events::Event for Churn {
        const NAME: &'static str = "fixture.churn";
    }
    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct Alarm(u64);
    impl core_events::Event for Alarm {
        const NAME: &'static str = "fixture.alarm";
    }
    events.register::<Churn>().unwrap();
    events.register::<Alarm>().unwrap();
    events.schedule(Ticks::new(9), &Alarm(1)).unwrap();
    let v2_events = events.to_bytes().unwrap();

    let v2 = SaveBody {
        seed: Seed::new(4),
        tick: Ticks::new(5),
        entities: vec![],
        rng: vec![],
        components: vec![("fixture.wealth".into(), empty_store())],
        events: v2_events,
    };
    let raw = codec::to_bytes(&v2).unwrap();
    let v3 = migrate_to_current(2, raw).unwrap();
    assert_eq!(v3.components.len(), 1 + all_appended().len());

    // The migrated blob restores strictly into the grown registration
    // (the full current set: people + economy events appended).
    let mut grown = core_events::Events::new(8);
    grown.register::<Churn>().unwrap();
    grown.register::<Alarm>().unwrap();
    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct Died(u8);
    impl core_events::Event for Died {
        const NAME: &'static str = "people.person_died";
    }
    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct Bought(u8);
    impl core_events::Event for Bought {
        const NAME: &'static str = "econ.goods_purchased";
    }
    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct Repriced(u8);
    impl core_events::Event for Repriced {
        const NAME: &'static str = "econ.price_changed";
    }
    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct HiredEv(u8);
    impl core_events::Event for HiredEv {
        const NAME: &'static str = "econ.hired";
    }
    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct FiredEv(u8);
    impl core_events::Event for FiredEv {
        const NAME: &'static str = "econ.fired";
    }
    grown.register::<Died>().unwrap();
    grown.register::<Bought>().unwrap();
    grown.register::<Repriced>().unwrap();
    grown.register::<HiredEv>().unwrap();
    grown.register::<FiredEv>().unwrap();
    macro_rules! money_event {
        ($ty:ident, $name:literal) => {
            #[derive(Debug, serde::Serialize, serde::Deserialize)]
            struct $ty(u8);
            impl core_events::Event for $ty {
                const NAME: &'static str = $name;
            }
            grown.register::<$ty>().unwrap();
        };
    }
    money_event!(LoanGrantedEv, "econ.loan_granted");
    money_event!(LoanDefaultedEv, "econ.loan_defaulted");
    money_event!(TenancyStartedEv, "econ.tenancy_started");
    money_event!(HomeSoldEv, "econ.home_sold");
    money_event!(HomeBuiltEv, "econ.home_built");
    money_event!(TaxCollectedEv, "econ.tax_collected");
    money_event!(MarriedEv, "social.married");
    money_event!(BornEv, "people.born");
    money_event!(SchoolEv, "people.school_attended");
    grown.restore(&v3.events).unwrap();
    assert_eq!(grown.scheduled_count(), 1);
}

#[test]
fn current_version_decodes_directly() {
    let body = SaveBody {
        seed: Seed::new(1),
        tick: Ticks::new(2),
        entities: vec![],
        rng: vec![],
        components: vec![],
        events: vec![5, 6],
    };
    let raw = codec::to_bytes(&body).unwrap();
    let back = migrate_to_current(FORMAT_VERSION, raw).unwrap();
    assert_eq!(back.events, vec![5, 6]);
}

#[test]
fn unknown_versions_error() {
    assert!(matches!(
        migrate_to_current(0, vec![]),
        Err(PersistError::UnsupportedVersion(0))
    ));
    assert!(matches!(
        migrate_to_current(FORMAT_VERSION + 1, vec![]),
        Err(PersistError::UnsupportedVersion(_))
    ));
}

/// ADR 0004 §10 content-continuity proof: the committed Phase 0 golden
/// fixture's entity/RNG/component bytes pass through the migration
/// chain byte-for-byte — migrations add empty registrations and touch
/// nothing else.
#[test]
fn v1_fixture_content_survives_migration_verbatim() {
    const V1_FIXTURE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/v1_seed7_fixture50_tick1000.embersave"
    ));
    // Peel the header (magic + u32 version) and decompress.
    let payload = &V1_FIXTURE[crate::MAGIC.len() + 4..];
    let raw = zstd::stream::decode_all(payload).expect("fixture decompresses");
    let v1: SaveBodyV1 = codec::from_bytes(&raw).expect("fixture decodes as v1");

    let raw_again = codec::to_bytes(&v1).expect("re-encode");
    let migrated = migrate_to_current(1, raw_again).expect("migration");
    assert_eq!(migrated.seed, v1.seed);
    assert_eq!(migrated.tick, v1.tick);
    assert_eq!(migrated.entities, v1.entities);
    assert_eq!(migrated.rng, v1.rng);
    // Original stores verbatim, then every appended (empty) store.
    assert_eq!(&migrated.components[..v1.components.len()], &v1.components);
    let appended = all_appended();
    assert_eq!(
        migrated.components.len(),
        v1.components.len() + appended.len()
    );
    for (name, bytes) in &migrated.components[v1.components.len()..] {
        assert!(appended.contains(&name.as_str()));
        assert_eq!(*bytes, empty_store());
    }
    assert!(migrated.events.is_empty());
}

/// ADR 0004 §10 content-continuity proof for the committed Phase 1
/// fixture: v2→v3 passes seed/tick/entities/RNG/original-component
/// bytes through verbatim, appends only the (empty) v3 stores, and the
/// events blob differs from the original by exactly the name-list
/// extension (byte-identical to `extend_registration_bytes` applied to
/// the original — the transformation `v2_to_v3` performs and
/// `extend_registration_bytes_is_lossless…` in core_events proves
/// touches nothing but the names).
#[test]
fn v2_fixture_content_survives_migration_verbatim() {
    const V2_FIXTURE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/v2_seed13_fixture60_tick2000.embersave"
    ));
    let payload = &V2_FIXTURE[crate::MAGIC.len() + 4..];
    let raw = zstd::stream::decode_all(payload).expect("fixture decompresses");
    let v2: SaveBodyV2 = codec::from_bytes(&raw).expect("fixture decodes as v2");
    let original_events = v2.events.clone();
    let original_components = v2.components.clone();

    let migrated = migrate_to_current(2, raw).expect("migration");
    assert_eq!(migrated.seed, Seed::new(13));
    assert_eq!(migrated.tick, Ticks::new(2000));
    assert_eq!(
        &migrated.components[..original_components.len()],
        &original_components
    );
    let appended = all_appended();
    assert_eq!(
        migrated.components.len(),
        original_components.len() + appended.len()
    );
    for (name, bytes) in &migrated.components[original_components.len()..] {
        assert!(appended.contains(&name.as_str()));
        assert_eq!(*bytes, empty_store());
    }
    let expected_events = chain_extensions(
        &original_events,
        &[
            &V3_ADDED_EVENTS,
            &V5_ADDED_EVENTS,
            &V6_ADDED_EVENTS,
            &V7_ADDED_EVENTS,
            &V8_ADDED_EVENTS,
        ],
    );
    assert_eq!(
        migrated.events, expected_events,
        "events blob must differ by exactly the chained registration extensions"
    );
    assert_ne!(migrated.events, original_events);
}

/// ADR 0004 §10 content-continuity proof for the committed Phase 2
/// fixture: the v3→current chain passes every field through verbatim
/// except the appended (empty) world/AI + economy stores; the events
/// blob differs by exactly the v5 name-list extension (v3→v4 added no
/// events).
#[test]
fn v3_fixture_content_survives_migration_verbatim() {
    const V3_FIXTURE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/v3_seed17_fixture40_citizens300_tick3000.embersave"
    ));
    let payload = &V3_FIXTURE[crate::MAGIC.len() + 4..];
    let raw = zstd::stream::decode_all(payload).expect("fixture decompresses");
    let v3: SaveBodyV3 = codec::from_bytes(&raw).expect("fixture decodes as v3");
    let original_events = v3.events.clone();
    let original_components = v3.components.clone();

    let migrated = migrate_to_current(3, raw).expect("migration");
    assert_eq!(migrated.seed, Seed::new(17));
    assert_eq!(migrated.tick, Ticks::new(3000));
    assert_eq!(
        &migrated.components[..original_components.len()],
        &original_components
    );
    let appended: Vec<&str> = V4_ADDED_COMPONENTS
        .iter()
        .chain(V5_ADDED_COMPONENTS.iter())
        .chain(V6_ADDED_COMPONENTS.iter())
        .chain(V7_ADDED_COMPONENTS.iter())
        .chain(V8_ADDED_COMPONENTS.iter())
        .copied()
        .collect();
    assert_eq!(
        migrated.components.len(),
        original_components.len() + appended.len()
    );
    for (name, bytes) in &migrated.components[original_components.len()..] {
        assert!(appended.contains(&name.as_str()));
        assert_eq!(*bytes, empty_store());
    }
    assert_eq!(
        migrated.events,
        chain_extensions(
            &original_events,
            &[
                &V5_ADDED_EVENTS,
                &V6_ADDED_EVENTS,
                &V7_ADDED_EVENTS,
                &V8_ADDED_EVENTS
            ],
        ),
        "the events blob must differ by exactly the chained registration extensions"
    );
}

/// ADR 0004 §10 content-continuity proof for the committed Phase 3
/// fixture: the v4→current chain passes every field through verbatim
/// except the appended (empty) economy + labor stores; the events
/// blob differs by exactly the v5+v6 name-list extensions.
#[test]
fn v4_fixture_content_survives_migration_verbatim() {
    const V4_FIXTURE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/v4_seed23_fixture30_citizens250_tick2500.embersave"
    ));
    let payload = &V4_FIXTURE[crate::MAGIC.len() + 4..];
    let raw = zstd::stream::decode_all(payload).expect("fixture decompresses");
    let v4: SaveBodyV4 = codec::from_bytes(&raw).expect("fixture decodes as v4");
    let original_events = v4.events.clone();
    let original_components = v4.components.clone();

    let migrated = migrate_to_current(4, raw).expect("migration");
    assert_eq!(migrated.seed, Seed::new(23));
    assert_eq!(migrated.tick, Ticks::new(2500));
    assert_eq!(
        &migrated.components[..original_components.len()],
        &original_components
    );
    let appended: Vec<&str> = V5_ADDED_COMPONENTS
        .iter()
        .chain(V6_ADDED_COMPONENTS.iter())
        .chain(V7_ADDED_COMPONENTS.iter())
        .chain(V8_ADDED_COMPONENTS.iter())
        .copied()
        .collect();
    assert_eq!(
        migrated.components.len(),
        original_components.len() + appended.len()
    );
    for (name, bytes) in &migrated.components[original_components.len()..] {
        assert!(appended.contains(&name.as_str()));
        assert_eq!(*bytes, empty_store());
    }
    assert_eq!(
        migrated.events,
        chain_extensions(
            &original_events,
            &[
                &V5_ADDED_EVENTS,
                &V6_ADDED_EVENTS,
                &V7_ADDED_EVENTS,
                &V8_ADDED_EVENTS
            ],
        ),
        "the events blob must differ by exactly the chained registration extensions"
    );
    assert_ne!(migrated.events, original_events);
}

/// ADR 0004 §10 content-continuity proof for the committed Phase 4
/// fixture: v5→current passes every field through verbatim except the
/// appended (empty) labor + money stores; the events blob differs by
/// exactly the v6+v7 name-list extensions.
#[test]
fn v5_fixture_content_survives_migration_verbatim() {
    const V5_FIXTURE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/v5_seed29_fixture30_citizens250_tick2600.embersave"
    ));
    let payload = &V5_FIXTURE[crate::MAGIC.len() + 4..];
    let raw = zstd::stream::decode_all(payload).expect("fixture decompresses");
    let v5: SaveBodyV5 = codec::from_bytes(&raw).expect("fixture decodes as v5");
    let original_events = v5.events.clone();
    let original_components = v5.components.clone();

    let migrated = migrate_to_current(5, raw).expect("migration");
    assert_eq!(migrated.seed, Seed::new(29));
    assert_eq!(migrated.tick, Ticks::new(2600));
    assert_eq!(
        &migrated.components[..original_components.len()],
        &original_components
    );
    let appended: Vec<&str> = V6_ADDED_COMPONENTS
        .iter()
        .chain(V7_ADDED_COMPONENTS.iter())
        .chain(V8_ADDED_COMPONENTS.iter())
        .copied()
        .collect();
    assert_eq!(
        migrated.components.len(),
        original_components.len() + appended.len()
    );
    for (name, bytes) in &migrated.components[original_components.len()..] {
        assert!(appended.contains(&name.as_str()));
        assert_eq!(*bytes, empty_store());
    }
    assert_eq!(
        migrated.events,
        chain_extensions(
            &original_events,
            &[&V6_ADDED_EVENTS, &V7_ADDED_EVENTS, &V8_ADDED_EVENTS],
        ),
        "the events blob must differ by exactly the v6+v7 registration extensions"
    );
    assert_ne!(migrated.events, original_events);
}

/// ADR 0004 §10 content-continuity proof for the committed Phase 5
/// fixture: v6→v7 passes every field through verbatim except the five
/// appended (empty) money stores; the events blob differs by exactly
/// the v7 name-list extension.
#[test]
fn v6_fixture_content_survives_migration_verbatim() {
    const V6_FIXTURE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/v6_seed37_fixture30_citizens250_tick2000.embersave"
    ));
    let payload = &V6_FIXTURE[crate::MAGIC.len() + 4..];
    let raw = zstd::stream::decode_all(payload).expect("fixture decompresses");
    let v6: SaveBodyV6 = codec::from_bytes(&raw).expect("fixture decodes as v6");
    let original_events = v6.events.clone();
    let original_components = v6.components.clone();

    let migrated = migrate_to_current(6, raw).expect("migration");
    assert_eq!(migrated.seed, Seed::new(37));
    assert_eq!(migrated.tick, Ticks::new(2000));
    assert_eq!(
        &migrated.components[..original_components.len()],
        &original_components
    );
    let appended: Vec<&str> = V7_ADDED_COMPONENTS
        .iter()
        .chain(V8_ADDED_COMPONENTS.iter())
        .copied()
        .collect();
    assert_eq!(
        migrated.components.len(),
        original_components.len() + appended.len()
    );
    for (name, bytes) in &migrated.components[original_components.len()..] {
        assert!(appended.contains(&name.as_str()));
        assert_eq!(*bytes, empty_store());
    }
    assert_eq!(
        migrated.events,
        chain_extensions(&original_events, &[&V7_ADDED_EVENTS, &V8_ADDED_EVENTS]),
        "the events blob must differ by exactly the v7 registration extension"
    );
    assert_ne!(migrated.events, original_events);
}

/// ADR 0004 §10 content-continuity proof for the committed Phase 6
/// fixture: v7→v8 passes every field through verbatim except the four
/// appended (empty) social stores; the events blob differs by exactly
/// the v8 name-list extension.
#[test]
fn v7_fixture_content_survives_migration_verbatim() {
    const V7_FIXTURE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/v7_seed41_fixture30_citizens250_tick29360.embersave"
    ));
    let payload = &V7_FIXTURE[crate::MAGIC.len() + 4..];
    let raw = zstd::stream::decode_all(payload).expect("fixture decompresses");
    let v7: SaveBodyV7 = codec::from_bytes(&raw).expect("fixture decodes as v7");
    let original_events = v7.events.clone();
    let original_components = v7.components.clone();

    let migrated = migrate_to_current(7, raw).expect("migration");
    assert_eq!(migrated.seed, Seed::new(41));
    assert_eq!(migrated.tick, Ticks::new(29360));
    assert_eq!(
        &migrated.components[..original_components.len()],
        &original_components
    );
    assert_eq!(
        migrated.components.len(),
        original_components.len() + V8_ADDED_COMPONENTS.len()
    );
    for (name, bytes) in &migrated.components[original_components.len()..] {
        assert!(V8_ADDED_COMPONENTS.contains(&name.as_str()));
        assert_eq!(*bytes, empty_store());
    }
    assert_eq!(
        migrated.events,
        chain_extensions(&original_events, &[&V8_ADDED_EVENTS]),
        "the events blob must differ by exactly the v8 registration extension"
    );
    assert_ne!(migrated.events, original_events);
}

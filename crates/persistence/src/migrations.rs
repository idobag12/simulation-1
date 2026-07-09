//! The save-format migration pipeline (SPEC §9; ADR 0004 §9).
//!
//! Invariants:
//! - Old saves must load forever: every historical `format_version` has a
//!   pure migration path chained `v1 → v2 → … → current`.
//! - Each historical version's body struct is FROZEN here verbatim; it is
//!   the intermediate representation migrations operate on. Never edit a
//!   frozen struct — a format change adds a new version and a new step.
//! - Every migration step is a pure function old-body → new-body; each is
//!   proven in CI against the committed golden fixture of its source
//!   version (`tests/tests/save_compat.rs`).
//! - A version without a path (typically: newer than this build) is a
//!   typed error, never a guess.

use core_ecs::EcsError;
use core_types::codec;
use core_types::{Seed, Ticks};
use serde::{Deserialize, Serialize};

use crate::{FORMAT_VERSION, PersistError, SaveBody};

/// Format v1 body (Phase 0), FROZEN. Identical to v2 minus event state.
#[derive(Debug, Serialize, Deserialize)]
struct SaveBodyV1 {
    seed: Seed,
    tick: Ticks,
    entities: Vec<u8>,
    rng: Vec<u8>,
    components: Vec<(String, Vec<u8>)>,
}

/// Pure step v1 → v2.
///
/// v1 predates the event system entirely, so the migrated body carries the
/// empty-marker events blob (a zero-length byte vector — unambiguous,
/// since no canonical `EventsState` encoding is ever zero-length).
/// [`crate::load_from_bytes`] interprets the marker as "no event state was
/// saved": it leaves the freshly registered event system with all queues
/// empty, which is exactly the state a v1 world was in. Nothing is
/// invented, and `World::restore_events`' strict registration check is
/// never weakened — it simply isn't consulted for pre-event saves.
fn v1_to_v2(v1: SaveBodyV1) -> SaveBody {
    SaveBody {
        seed: v1.seed,
        tick: v1.tick,
        entities: v1.entities,
        rng: v1.rng,
        components: v1.components,
        events: Vec::new(),
    }
}

/// Format v2 body (Phase 1), FROZEN. Structurally identical to v3 — the
/// v2→v3 difference is the registration set, not the body shape.
type SaveBodyV2 = SaveBody;

// Registration growth from v2 to v3 (Phase 2, ADR 0005 §9). Historical
// facts of the format, frozen here forever.
const V3_ADDED_COMPONENTS: [&str; 5] = [
    "people.identity",
    "people.needs",
    "people.personality",
    "people.household_member",
    "people.household",
];
const V3_ADDED_EVENTS: [&str; 1] = ["people.person_died"];

/// Pure step v2 → v3 (ADR 0005 §9): the registration grew by the people
/// components and events, all appended after the v2 set. A v2 world
/// carried none of them, so each new component store is empty and the
/// saved event-name list extends losslessly.
fn v2_to_v3(mut v2: SaveBodyV2) -> Result<SaveBody, PersistError> {
    // Canonical bytes of an empty store: the empty pair list. The element
    // type does not matter for an empty sequence under the canonical codec.
    let empty_store = codec::to_bytes(&Vec::<(u32, u8)>::new())?;
    for name in V3_ADDED_COMPONENTS {
        v2.components.push((name.to_owned(), empty_store.clone()));
    }
    // v1-marker (empty) events blobs pass through: the loader skips
    // restore entirely and the fresh registration already includes the
    // appended names.
    if !v2.events.is_empty() {
        v2.events = core_events::extend_registration_bytes(&v2.events, &V3_ADDED_EVENTS)
            .map_err(EcsError::from)?;
    }
    Ok(v2)
}

/// Format v3 body (Phase 2), FROZEN. Structurally identical to v4 — the
/// v3→v4 difference is the registration set, not the body shape.
type SaveBodyV3 = SaveBody;

// Registration growth from v3 to v4 (Phase 3, ADR 0006 §8). Historical
// facts of the format, frozen here forever. No events were added.
const V4_ADDED_COMPONENTS: [&str; 6] = [
    "world.position",
    "world.location",
    "world.residence",
    "ai.current_action",
    "ai.daily_plan",
    "ai.last_decision",
];

/// Pure step v3 → v4 (ADR 0006 §8): the registration grew by the world/AI
/// components, all appended after the v3 set; the event registration is
/// unchanged. A v3 world carried none of the new components, so each new
/// store is empty. (A migrated town therefore has citizens but no
/// locations or positions — the AI idles in it, honestly: migrations
/// restore what was saved, they never invent state.)
fn v3_to_v4(mut v3: SaveBodyV3) -> Result<SaveBody, PersistError> {
    let empty_store = codec::to_bytes(&Vec::<(u32, u8)>::new())?;
    for name in V4_ADDED_COMPONENTS {
        v3.components.push((name.to_owned(), empty_store.clone()));
    }
    Ok(v3)
}

/// Migrates a decompressed save body from `version` to the current
/// [`SaveBody`], chaining pure steps (`v1 → v2 → v3 → v4`).
pub(crate) fn migrate_to_current(version: u32, raw: Vec<u8>) -> Result<SaveBody, PersistError> {
    match version {
        1 => {
            let v1: SaveBodyV1 = codec::from_bytes(&raw)?;
            v3_to_v4(v2_to_v3(v1_to_v2(v1))?)
        }
        2 => {
            let v2: SaveBodyV2 = codec::from_bytes(&raw)?;
            v3_to_v4(v2_to_v3(v2)?)
        }
        3 => {
            let v3: SaveBodyV3 = codec::from_bytes(&raw)?;
            v3_to_v4(v3)
        }
        FORMAT_VERSION => Ok(codec::from_bytes(&raw)?),
        other => Err(PersistError::UnsupportedVersion(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canonical bytes of an empty component store.
    fn empty_store() -> Vec<u8> {
        codec::to_bytes(&Vec::<(u32, u8)>::new()).unwrap()
    }

    /// Every component name the chain appends after a v2-era store list.
    fn all_appended() -> Vec<&'static str> {
        V3_ADDED_COMPONENTS
            .iter()
            .chain(V4_ADDED_COMPONENTS.iter())
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

        // The migrated blob restores strictly into the grown registration.
        let mut grown = core_events::Events::new(8);
        grown.register::<Churn>().unwrap();
        grown.register::<Alarm>().unwrap();
        #[derive(Debug, serde::Serialize, serde::Deserialize)]
        struct Died(u8);
        impl core_events::Event for Died {
            const NAME: &'static str = "people.person_died";
        }
        grown.register::<Died>().unwrap();
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
        assert_eq!(
            migrated.events,
            core_events::extend_registration_bytes(&original_events, &V3_ADDED_EVENTS).unwrap(),
            "events blob must differ by exactly the registration extension"
        );
        assert_ne!(migrated.events, original_events);
    }

    /// ADR 0004 §10 content-continuity proof for the committed Phase 2
    /// fixture: v3→v4 passes every field through verbatim except the six
    /// appended (empty) world/AI stores; the events blob is untouched.
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
        assert_eq!(
            migrated.components.len(),
            original_components.len() + V4_ADDED_COMPONENTS.len()
        );
        for (name, bytes) in &migrated.components[original_components.len()..] {
            assert!(V4_ADDED_COMPONENTS.contains(&name.as_str()));
            assert_eq!(*bytes, empty_store());
        }
        assert_eq!(
            migrated.events, original_events,
            "v3→v4 adds no events; the blob must pass through untouched"
        );
    }
}

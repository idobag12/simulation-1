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

/// Migrates a decompressed save body from `version` to the current
/// [`SaveBody`], chaining pure steps (`v1 → v2 → v3`).
pub(crate) fn migrate_to_current(version: u32, raw: Vec<u8>) -> Result<SaveBody, PersistError> {
    match version {
        1 => {
            let v1: SaveBodyV1 = codec::from_bytes(&raw)?;
            v2_to_v3(v1_to_v2(v1))
        }
        2 => {
            let v2: SaveBodyV2 = codec::from_bytes(&raw)?;
            v2_to_v3(v2)
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

    #[test]
    fn v1_bodies_chain_migrate_to_v3() {
        let v1 = SaveBodyV1 {
            seed: Seed::new(9),
            tick: Ticks::new(77),
            entities: vec![1, 2],
            rng: vec![3],
            components: vec![("a".into(), vec![4])],
        };
        let raw = codec::to_bytes(&v1).unwrap();
        let v3 = migrate_to_current(1, raw).unwrap();
        assert_eq!(v3.seed, Seed::new(9));
        assert_eq!(v3.tick, Ticks::new(77));
        assert_eq!(v3.entities, vec![1, 2]);
        assert_eq!(v3.rng, vec![3]);
        // Original components pass through verbatim, then the v3 appended
        // stores, all empty.
        assert_eq!(v3.components[0], ("a".to_owned(), vec![4]));
        assert_eq!(v3.components.len(), 1 + V3_ADDED_COMPONENTS.len());
        for (i, name) in V3_ADDED_COMPONENTS.iter().enumerate() {
            assert_eq!(v3.components[1 + i], ((*name).to_owned(), empty_store()));
        }
        // The v1 empty-events marker survives the whole chain.
        assert!(v3.events.is_empty());
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
        assert_eq!(v3.components.len(), 1 + V3_ADDED_COMPONENTS.len());

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
        // Original stores verbatim, then the appended (empty) v3 stores.
        assert_eq!(&migrated.components[..v1.components.len()], &v1.components);
        assert_eq!(
            migrated.components.len(),
            v1.components.len() + V3_ADDED_COMPONENTS.len()
        );
        for (name, bytes) in &migrated.components[v1.components.len()..] {
            assert!(V3_ADDED_COMPONENTS.contains(&name.as_str()));
            assert_eq!(*bytes, empty_store());
        }
        assert!(migrated.events.is_empty());
    }
}

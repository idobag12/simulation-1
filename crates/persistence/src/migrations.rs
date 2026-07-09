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

/// Migrates a decompressed save body from `version` to the current
/// [`SaveBody`], chaining pure steps.
pub(crate) fn migrate_to_current(version: u32, raw: Vec<u8>) -> Result<SaveBody, PersistError> {
    match version {
        1 => {
            let v1: SaveBodyV1 = codec::from_bytes(&raw)?;
            Ok(v1_to_v2(v1))
        }
        FORMAT_VERSION => Ok(codec::from_bytes(&raw)?),
        other => Err(PersistError::UnsupportedVersion(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_bodies_migrate_to_v2_with_no_event_state() {
        let v1 = SaveBodyV1 {
            seed: Seed::new(9),
            tick: Ticks::new(77),
            entities: vec![1, 2],
            rng: vec![3],
            components: vec![("a".into(), vec![4])],
        };
        let raw = codec::to_bytes(&v1).unwrap();
        let v2 = migrate_to_current(1, raw).unwrap();
        assert_eq!(v2.seed, Seed::new(9));
        assert_eq!(v2.tick, Ticks::new(77));
        assert_eq!(v2.entities, vec![1, 2]);
        assert_eq!(v2.rng, vec![3]);
        assert_eq!(v2.components, vec![("a".to_owned(), vec![4])]);
        assert!(v2.events.is_empty());
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
    /// fixture's entity/RNG/component bytes pass through the v1→v2
    /// migration byte-for-byte — the migration adds event state and touches
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
        assert_eq!(migrated.components, v1.components);
        assert!(migrated.events.is_empty());
    }
}

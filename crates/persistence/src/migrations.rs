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

/// Format v4 body (Phase 3), FROZEN. Structurally identical to v5 — the
/// v4→v5 difference is the registration set, not the body shape.
type SaveBodyV4 = SaveBody;

// Registration growth from v4 to v5 (Phase 4, ADR 0007 §9). Historical
// facts of the format, frozen here forever.
const V5_ADDED_COMPONENTS: [&str; 7] = [
    "econ.wallet",
    "goods.inventory",
    "econ.retail_offer",
    "econ.firm",
    "econ.firm_books",
    "econ.counters",
    "econ.production",
];
const V5_ADDED_EVENTS: [&str; 2] = ["econ.goods_purchased", "econ.price_changed"];

/// Pure step v4 → v5 (ADR 0007 §9): the registration grew by the economy
/// components and events, all appended after the v4 set. A v4 world
/// carried none of them, so each new store is empty and the event-name
/// list extends losslessly. (A migrated town therefore has no wallets,
/// firms, or ledger — it honestly has no economy; the auditor reports
/// "nothing to audit" and the economy systems no-op over empty stores.)
fn v4_to_v5(mut v4: SaveBodyV4) -> Result<SaveBody, PersistError> {
    let empty_store = codec::to_bytes(&Vec::<(u32, u8)>::new())?;
    for name in V5_ADDED_COMPONENTS {
        v4.components.push((name.to_owned(), empty_store.clone()));
    }
    if !v4.events.is_empty() {
        v4.events = core_events::extend_registration_bytes(&v4.events, &V5_ADDED_EVENTS)
            .map_err(EcsError::from)?;
    }
    Ok(v4)
}

/// Format v5 body (Phase 4), FROZEN. Structurally identical to v6 — the
/// v5→v6 difference is the registration set, not the body shape.
type SaveBodyV5 = SaveBody;

// Registration growth from v5 to v6 (Phase 5, ADR 0008 §8). Historical
// facts of the format, frozen here forever.
const V6_ADDED_COMPONENTS: [&str; 3] =
    ["econ.employment", "econ.labor_stats", "people.working_age"];
const V6_ADDED_EVENTS: [&str; 2] = ["econ.hired", "econ.fired"];

/// Pure step v5 → v6 (ADR 0008 §8): the registration grew by the labor
/// components and events, all appended after the v5 set. A v5 world
/// carried none of them, so each new store is empty and the event-name
/// list extends losslessly. (A migrated town therefore has no jobs and
/// nobody marked working-age — the labor systems hire nobody until the
/// day-rate promotion stamps the marker from real ages: migrations never
/// invent state; the world catches up honestly within its first day.)
fn v5_to_v6(mut v5: SaveBodyV5) -> Result<SaveBody, PersistError> {
    let empty_store = codec::to_bytes(&Vec::<(u32, u8)>::new())?;
    for name in V6_ADDED_COMPONENTS {
        v5.components.push((name.to_owned(), empty_store.clone()));
    }
    if !v5.events.is_empty() {
        v5.events = core_events::extend_registration_bytes(&v5.events, &V6_ADDED_EVENTS)
            .map_err(EcsError::from)?;
    }
    Ok(v5)
}

/// Format v6 body (Phase 5), FROZEN. Structurally identical to v7 — the
/// v6→v7 difference is the registration set, not the body shape.
type SaveBodyV6 = SaveBody;

// Registration growth from v6 to v7 (Phase 6, ADR 0009 §6). Historical
// facts of the format, frozen here forever.
const V7_ADDED_COMPONENTS: [&str; 6] = [
    "econ.bank_book",
    "econ.treasury_book",
    "world.ownership",
    "world.tenancy",
    "econ.borrower_status",
    "econ.housing_book",
];
const V7_ADDED_EVENTS: [&str; 6] = [
    "econ.loan_granted",
    "econ.loan_defaulted",
    "econ.tenancy_started",
    "econ.home_sold",
    "econ.home_built",
    "econ.tax_collected",
];

/// Pure step v6 → v7 (ADR 0009 §6): the registration grew by the money
/// components and events, all appended after the v6 set. A v6 world
/// carried none of them, so each new store is empty and the event-name
/// list extends losslessly. (A migrated town therefore has no bank,
/// treasury, or ownership — a bank needs seeded equity, which migrations
/// must not invent, so those systems no-op; documented limitation.)
fn v6_to_v7(mut v6: SaveBodyV6) -> Result<SaveBody, PersistError> {
    let empty_store = codec::to_bytes(&Vec::<(u32, u8)>::new())?;
    for name in V7_ADDED_COMPONENTS {
        v6.components.push((name.to_owned(), empty_store.clone()));
    }
    if !v6.events.is_empty() {
        v6.events = core_events::extend_registration_bytes(&v6.events, &V7_ADDED_EVENTS)
            .map_err(EcsError::from)?;
    }
    Ok(v6)
}

/// Format v7 body, FROZEN. Structurally identical to v8 — v7→v8 only
/// appends registrations — so the alias documents the version boundary.
type SaveBodyV7 = SaveBodyV8;

// Registration growth from v7 to v8 (Phase 7, ADR 0010 §6). Historical
// facts of the format, frozen here forever.
const V8_ADDED_COMPONENTS: [&str; 4] = [
    "people.skills",
    "social.relationships",
    "social.beliefs",
    "people.school_age",
];
const V8_ADDED_EVENTS: [&str; 3] = ["social.married", "people.born", "people.school_attended"];

/// Pure step v7 → v8 (ADR 0010 §6): the registration grew by the social
/// components and lifecycle events, all appended after the v7 set. A v7
/// world carried none of them, so each new store is empty and the
/// event-name list extends losslessly. Migrated citizens have no
/// skills, edges, or beliefs — every Phase 7 system builds state from
/// lived events, inventing nothing.
fn v7_to_v8(mut v7: SaveBodyV7) -> Result<SaveBodyV8, PersistError> {
    let empty_store = codec::to_bytes(&Vec::<(u32, u8)>::new())?;
    for name in V8_ADDED_COMPONENTS {
        v7.components.push((name.to_owned(), empty_store.clone()));
    }
    if !v7.events.is_empty() {
        v7.events = core_events::extend_registration_bytes(&v7.events, &V8_ADDED_EVENTS)
            .map_err(EcsError::from)?;
    }
    Ok(v7)
}

/// Format v8 body, FROZEN. Structurally identical to v9 — v8→v9 only
/// appends registrations — so the alias documents the version boundary.
type SaveBodyV8 = SaveBody;

// Registration growth from v8 to v9 (Phase 8, ADR 0011 §6). Historical
// facts of the format, frozen here forever.
const V9_ADDED_COMPONENTS: [&str; 3] = ["lod.tier", "lod.day_model", "lod.spotlight"];
const V9_ADDED_EVENTS: [&str; 1] = ["lod.tier_changed"];

/// Pure step v8 → v9 (ADR 0011 §6): the registration grew by the LOD
/// components and the tier-change event, all appended after the v8 set.
/// A v8 world carried no tier rows, so each new store is empty — every
/// migrated citizen runs Tier A (the pre-v9 status quo) until the first
/// day boundary's assignment stamps them; nothing is invented.
fn v8_to_v9(mut v8: SaveBodyV8) -> Result<SaveBody, PersistError> {
    let empty_store = codec::to_bytes(&Vec::<(u32, u8)>::new())?;
    for name in V9_ADDED_COMPONENTS {
        v8.components.push((name.to_owned(), empty_store.clone()));
    }
    if !v8.events.is_empty() {
        v8.events = core_events::extend_registration_bytes(&v8.events, &V9_ADDED_EVENTS)
            .map_err(EcsError::from)?;
    }
    Ok(v8)
}

/// Migrates a decompressed save body from `version` to the current
/// [`SaveBody`], chaining pure steps (`v1 → v2 → … → v9`).
pub(crate) fn migrate_to_current(version: u32, raw: Vec<u8>) -> Result<SaveBody, PersistError> {
    match version {
        1 => {
            let v1: SaveBodyV1 = codec::from_bytes(&raw)?;
            v8_to_v9(v7_to_v8(v6_to_v7(v5_to_v6(v4_to_v5(v3_to_v4(
                v2_to_v3(v1_to_v2(v1))?,
            )?)?)?)?)?)
        }
        2 => {
            let v2: SaveBodyV2 = codec::from_bytes(&raw)?;
            v8_to_v9(v7_to_v8(v6_to_v7(v5_to_v6(v4_to_v5(v3_to_v4(
                v2_to_v3(v2)?,
            )?)?)?)?)?)
        }
        3 => {
            let v3: SaveBodyV3 = codec::from_bytes(&raw)?;
            v8_to_v9(v7_to_v8(v6_to_v7(v5_to_v6(v4_to_v5(v3_to_v4(v3)?)?)?)?)?)
        }
        4 => {
            let v4: SaveBodyV4 = codec::from_bytes(&raw)?;
            v8_to_v9(v7_to_v8(v6_to_v7(v5_to_v6(v4_to_v5(v4)?)?)?)?)
        }
        5 => {
            let v5: SaveBodyV5 = codec::from_bytes(&raw)?;
            v8_to_v9(v7_to_v8(v6_to_v7(v5_to_v6(v5)?)?)?)
        }
        6 => {
            let v6: SaveBodyV6 = codec::from_bytes(&raw)?;
            v8_to_v9(v7_to_v8(v6_to_v7(v6)?)?)
        }
        7 => {
            let v7: SaveBodyV7 = codec::from_bytes(&raw)?;
            v8_to_v9(v7_to_v8(v7)?)
        }
        8 => {
            let v8: SaveBodyV8 = codec::from_bytes(&raw)?;
            v8_to_v9(v8)
        }
        FORMAT_VERSION => Ok(codec::from_bytes(&raw)?),
        other => Err(PersistError::UnsupportedVersion(other)),
    }
}

#[cfg(test)]
#[path = "migrations_tests.rs"]
mod tests;

//! Data-defined location kinds (`data/locations.ron`, SPEC §8;
//! ADR 0006 §2). These structs ARE the RON schema; `data_defs` loads and
//! validates them (ADR 0005 §6 amendment).

use serde::Deserialize;

/// One need a location kind satisfies, and how fast.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SatisfierDef {
    /// A need id from `data/balance/needs.ron` (validated to exist).
    pub need_id: String,
    /// Per-million need units gained per tick spent performing here.
    pub per_tick: i64,
}

/// One location kind, in the fixed order that becomes `Location::kind`
/// (data order = the stable integer id, SPEC §8).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationKindDef {
    /// Stable identifier, e.g. `"tavern"`.
    pub id: String,
    /// Home kinds are instantiated one per household at genesis and are
    /// afforded only to their residents; public kinds are open to all.
    pub is_home: bool,
    /// Public instances created at genesis (ignored for home kinds).
    pub count: u32,
    /// Which needs this kind satisfies, at what per-tick rate.
    pub satisfies: Vec<SatisfierDef>,
}

/// `data/locations.ron`.
///
/// Invariants after validation: exactly one home kind; every satisfier's
/// need id exists; at least one kind satisfies the rest need (a town
/// whose citizens cannot sleep is a data error, ADR 0006 §7).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationsConfig {
    /// Ordered kind list (`Location::kind` indexes it).
    pub kinds: Vec<LocationKindDef>,
}

impl LocationsConfig {
    /// Index of the (validated: unique) home kind.
    pub fn home_kind(&self) -> Option<u32> {
        self.kinds
            .iter()
            .position(|kind| kind.is_home)
            .map(|index| index as u32)
    }
}

/// One district on the town map (Phase 9, ADR 0012 §1).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DistrictDef {
    /// Stable identifier, e.g. `"old_town"`.
    pub id: String,
}

/// `data/map.ron` — the whole spatial model: districts and the
/// symmetric travel-time matrix (the matrix IS the path; ADR 0012 §1).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapConfig {
    /// Ordered district definitions (data order = the persisted index).
    pub districts: Vec<DistrictDef>,
    /// `travel_ticks[from][to]`, districts × districts; validated
    /// square, symmetric, every entry ≥ 1 (the diagonal is the
    /// intra-district trip — no teleports) and ≤ one day.
    pub travel_ticks: Vec<Vec<u32>>,
    /// What one commute tick costs a housing bidder, mills
    /// (ADR 0012 §3 — converts distance into money for the clearing).
    pub commute_mills_per_tick: i64,
}

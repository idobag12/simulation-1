//! Data-defined goods (`data/goods.ron`, SPEC §8; ADR 0007 §7). These
//! structs ARE the RON schema; `data_defs` loads and validates them.

use serde::Deserialize;

/// One good, in the fixed order that becomes its stable integer id
/// (data order, SPEC §8): inventory vectors, counters, and recipe
/// references all index this list.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoodDef {
    /// Stable identifier, e.g. `"grain"`.
    pub id: String,
    /// Daily spoilage in per-mille of held stock (0 = non-perishable).
    pub spoil_per_mille: i64,
}

/// `data/goods.ron`.
///
/// Invariants after validation: ids unique and non-empty;
/// `0 <= spoil_per_mille <= 1000`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoodsConfig {
    /// Ordered good list (data order = stable id).
    pub goods: Vec<GoodDef>,
}

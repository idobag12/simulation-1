//! AI tunables: the RON schema for `data/balance/ai.ron` (SPEC §8) and
//! the resolved cross-config tables the systems run on.

use serde::Deserialize;

/// Sleep-schedule tunables (ADR 0006 §5).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SleepDef {
    /// Base bedtime hour (0..24).
    pub base_start_hour: u8,
    /// Base wake hour (0..24; may be earlier than start — window crosses
    /// midnight).
    pub base_end_hour: u8,
    /// Maximum minutes the shift trait moves the window earlier.
    pub max_shift_minutes: u16,
    /// Trait id (from `traits.ron`) that shifts the window (early risers).
    pub shift_trait_id: String,
    /// Need id (from `needs.ron`) that sleeping satisfies.
    pub rest_need_id: String,
    /// Score bias (micro units) added to rest-at-own-home candidates
    /// during the citizen's sleep window.
    pub home_bias_micro: i64,
}

/// Purchase-scoring tunables (Phase 4, ADR 0007 §6): the marginal
/// utility of wealth converts a posted price into a score cost —
/// `mu = (mu_scale_micro / 1e6) / (1 + wallet / half_wealth_mills)` —
/// so the same price weighs more on a thin wallet (SPEC §11's money
/// cost).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PurchaseDef {
    /// Score micro-units one mill costs a penniless citizen.
    pub mu_scale_micro: i64,
    /// Wealth (mills) at which money matters half as much.
    pub half_wealth_mills: i64,
}

/// Maps a need to the personality trait that amplifies its utility
/// (SPEC §11: traits are weights in scoring).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeedTraitWeight {
    /// A need id from `needs.ron`.
    pub need_id: String,
    /// A trait id from `traits.ron`.
    pub trait_id: String,
    /// Blend strength, per-mille: factor = 1 + (weight/1000)·(trait/1000).
    pub weight_per_mille: u32,
}

/// `data/balance/ai.ron`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiConfig {
    /// Flat travel time between any two locations (no map until Phase 9).
    pub travel_ticks: u32,
    /// Urgency nonlinearity: urgency = deficit^exponent (≥ 1; SPEC §11's
    /// "critical hunger dominates").
    pub urgency_exponent: u32,
    /// Cost of one tick spent traveling or performing, in score micro
    /// units (the time term of SPEC §11's cost side).
    pub time_cost_micro_per_tick: i64,
    /// Longest single performance; an action ends here even if the need
    /// is not yet full.
    pub max_perform_ticks: u32,
    /// How long an Idle decision lasts before the citizen reconsiders.
    pub idle_ticks: u32,
    /// Hour of day when citizens compile the next day's plan (ADR 0006 §5).
    pub plan_compile_hour: u8,
    /// Sleep-schedule tunables.
    pub sleep: SleepDef,
    /// Purchase-scoring tunables (Phase 4).
    pub purchase: PurchaseDef,
    /// Need→trait scoring weights.
    pub need_trait_weights: Vec<NeedTraitWeight>,
}

/// The resolved, index-based tables the AI systems run on — built by
/// `data_defs::resolve_ai` after validation (string ids resolved to data
/// order indices; `sim_ai` never sees other sim crates' config types,
/// SPEC §4).
///
/// Invariant: indices are valid for the loaded data (needs/traits/location
/// kinds in data order); construction is validation's responsibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiTables {
    /// See [`AiConfig::travel_ticks`].
    pub travel_ticks: u32,
    /// See [`AiConfig::urgency_exponent`].
    pub urgency_exponent: u32,
    /// See [`AiConfig::time_cost_micro_per_tick`].
    pub time_cost_micro_per_tick: i64,
    /// See [`AiConfig::max_perform_ticks`].
    pub max_perform_ticks: u32,
    /// See [`AiConfig::idle_ticks`].
    pub idle_ticks: u32,
    /// See [`AiConfig::plan_compile_hour`].
    pub plan_compile_hour: u8,
    /// Sleep window base start, minutes of day.
    pub sleep_start_minute: u16,
    /// Sleep window base end, minutes of day.
    pub sleep_end_minute: u16,
    /// Maximum earlier-shift in minutes.
    pub sleep_max_shift_minutes: u16,
    /// Trait index (data order) shifting the sleep window.
    pub sleep_shift_trait: u32,
    /// Need index (data order) sleeping satisfies.
    pub rest_need: u32,
    /// Sleep-at-home score bias, micro units.
    pub sleep_home_bias_micro: i64,
    /// Per location kind (data order): is it a home?
    pub kind_is_home: Vec<bool>,
    /// Per location kind (data order): `(need index, per-tick gain)` in
    /// satisfier data order.
    pub kind_satisfiers: Vec<Vec<(u32, i64)>>,
    /// Per need index: the trait that amplifies it `(trait index, weight
    /// per-mille)`, if any.
    pub need_trait: Vec<Option<(u32, u32)>>,
    /// See [`PurchaseDef::mu_scale_micro`].
    pub mu_scale_micro: i64,
    /// See [`PurchaseDef::half_wealth_mills`].
    pub half_wealth_mills: i64,
}

impl AiTables {
    /// The per-tick gain of `kind` for `need_index`, if that kind
    /// satisfies it.
    pub fn satisfier_rate(&self, kind: u32, need_index: u32) -> Option<i64> {
        self.kind_satisfiers
            .get(kind as usize)?
            .iter()
            .find(|(need, _)| *need == need_index)
            .map(|(_, rate)| *rate)
    }
}

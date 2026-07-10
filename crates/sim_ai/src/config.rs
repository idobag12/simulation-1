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
    /// Work shift start, minutes of day (Phase 5, from
    /// `data/balance/labor.ron` via `data_defs::resolve_ai`).
    pub work_start_minute: u16,
    /// Work shift end, minutes of day.
    pub work_end_minute: u16,
    /// Score bias for the Work candidate during the shift (micro units;
    /// obligations bias, they don't dictate — ADR 0008 §2).
    pub work_bias_micro: i64,
    /// Length of one work stint, ticks.
    pub work_ticks: u32,
    /// Need (data order) working satisfies.
    pub work_need: u32,
    /// Per-tick per-million gain of that need while working.
    pub work_need_per_tick: i64,
    /// Sales tax split out of every retail purchase, per-mille
    /// (Phase 6, from `data/balance/taxes.ron`).
    pub sales_tax_per_mille: i64,
    /// School window start, minutes of day (Phase 7, from
    /// `data/skills.ron` via `data_defs::resolve_ai`).
    pub school_start_minute: u16,
    /// School window end, minutes of day.
    pub school_end_minute: u16,
    /// Score bias for the AttendSchool candidate inside the window.
    pub school_bias_micro: i64,
    /// One attendance stint, ticks.
    pub school_attend_ticks: u32,
    /// The school's location kind (data order).
    pub school_location_kind: u32,
    /// The skill (data order) attendance raises.
    pub school_taught_skill: u32,
    /// Per-mille mastery per completed attendance.
    pub school_gain_per_mille: u16,
    /// The social graph's resolved tunables (Phase 7, ADR 0010 §§2–3).
    pub social: SocialTables,
    /// Total skills in data order (the `Skills.levels` length).
    pub skill_count: u32,
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

/// `data/balance/social.ron` (Phase 7, ADR 0010 §§2–3): the relationship
/// graph's drifts and caps, the marriage threshold, and gossip.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocialConfig {
    /// The need (from `needs.ron`) whose satisfiers are the leisure
    /// venues where bonds drift.
    pub drift_need_id: String,
    /// The trait (from `traits.ron`) whose product screens romance.
    pub spark_trait_id: String,
    /// Bounded edges per citizen; the weakest non-kin edge evicts.
    pub edge_cap: u32,
    /// Friend-edge growth per shared leisure meeting, per-mille.
    pub friend_drift_per_meeting_per_mille: i32,
    /// Romance-edge growth per meeting (both single adults), per-mille.
    pub romance_drift_per_meeting_per_mille: i32,
    /// Non-kin edges decay this much per day; zero edges drop.
    pub decay_per_day_per_mille: i32,
    /// Romance sparks only when `soc_a × soc_b / 1000` clears this.
    pub romance_min_sociability_product_per_mille: i32,
    /// A romance edge at or above this marries (read by `sim_people`).
    pub marriage_threshold_per_mille: i32,
    /// Social-need gain multiplier per present friend:
    /// `1 + weight/1000 × strength/1000`.
    pub social_bond_weight_per_mille: i64,
    /// Bounded believed-price rows per citizen.
    pub belief_cap: u32,
}

/// The resolved social tables (string ids → indices), carried in
/// [`AiTables`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocialTables {
    /// See [`SocialConfig::edge_cap`].
    pub edge_cap: u32,
    /// See [`SocialConfig::friend_drift_per_meeting_per_mille`].
    pub friend_drift_per_meeting_per_mille: i32,
    /// See [`SocialConfig::romance_drift_per_meeting_per_mille`].
    pub romance_drift_per_meeting_per_mille: i32,
    /// See [`SocialConfig::decay_per_day_per_mille`].
    pub decay_per_day_per_mille: i32,
    /// See [`SocialConfig::romance_min_sociability_product_per_mille`].
    pub romance_min_sociability_product_per_mille: i32,
    /// See [`SocialConfig::marriage_threshold_per_mille`].
    pub marriage_threshold_per_mille: i32,
    /// See [`SocialConfig::social_bond_weight_per_mille`].
    pub social_bond_weight_per_mille: i64,
    /// See [`SocialConfig::belief_cap`].
    pub belief_cap: u32,
    /// The drift need (data order index).
    pub drift_need: u32,
    /// The romance-screen trait (data order index).
    pub spark_trait: u32,
}

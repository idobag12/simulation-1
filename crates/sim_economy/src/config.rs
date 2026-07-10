//! Economy tunables: the RON schemas for `data/recipes.ron`,
//! `data/firms.ron`, and `data/balance/economy.ron` (SPEC §8;
//! ADR 0007 §7), and the resolved index-based tables the systems run on
//! (built by `data_defs::resolve_economy` after validation — `sim_economy`
//! never sees other sim crates' config types, SPEC §4).

use core_types::Money;
use serde::Deserialize;

/// A `(good, quantity)` pair in a recipe or a seeded inventory.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoodQty {
    /// A good id from `data/goods.ron`.
    pub good_id: String,
    /// Units (≥ 1 after validation).
    pub quantity: i64,
}

/// One recipe, in data order (`Firm::recipe` indexes this list).
/// Harvest recipes (farm, well, forest) have no inputs — they are the
/// explicit modeled goods sources (ADR 0007 §1).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeDef {
    /// Stable identifier, e.g. `"bake_bread"`.
    pub id: String,
    /// Consumed per batch (may be empty: a harvest recipe).
    pub inputs: Vec<GoodQty>,
    /// Produced per completed batch. `None` is allowed only for
    /// home-building recipes (Phase 6, ADR 0009 §5) — the home entity is
    /// the output.
    pub output: Option<GoodQty>,
    /// Batch duration in hours (≥ 1).
    pub batch_hours: u32,
    /// A completed batch yields a new home instead of goods (Phase 6).
    #[serde(default)]
    pub builds_home: bool,
    /// The skill this recipe trains and rewards (Phase 7, ADR 0010 §1);
    /// `None` = unskilled work.
    #[serde(default)]
    pub skill_id: Option<String>,
}

/// `data/recipes.ron`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipesConfig {
    /// Ordered recipe list.
    pub recipes: Vec<RecipeDef>,
}

/// `data/balance/labor.ron` (Phase 5, ADR 0008 §7): the shift, the
/// labor-force threshold, and the market's reservation/bid tunables.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaborConfig {
    /// Shift start hour (0..24).
    pub shift_start_hour: u8,
    /// Shift end hour (0..24; must be after start — no overnight shifts
    /// this phase).
    pub shift_end_hour: u8,
    /// Minimum age (world years) to enter the labor force.
    pub min_working_age_years: u32,
    /// Reservation-wage base, mills/day.
    pub reservation_base_mills: i64,
    /// How much wealth raises the reservation, per-mille of base at
    /// saturation: ask += base × per_mille/1000 × wallet/(wallet+half).
    pub reservation_wealth_per_mille: i64,
    /// Wealth (mills) at which the raise reaches half strength.
    pub reservation_half_wealth_mills: i64,
    /// Trait (from `traits.ron`) that lowers the reservation.
    pub reservation_trait_id: String,
    /// Discount at full trait, per-mille of base.
    pub reservation_trait_discount_per_mille: i64,
    /// The share of a worker's daily marginal product a firm bids,
    /// per-mille.
    pub bid_fraction_per_mille: i64,
    /// Score bias for the Work candidate during the shift, micro units.
    pub work_bias_micro: i64,
    /// Length of one work stint, ticks.
    pub work_ticks: u32,
    /// Need id (from `needs.ron`) working satisfies.
    pub work_need_id: String,
    /// Per-tick per-million gain of that need while working.
    pub work_need_per_tick: i64,
}

/// A firm kind's retail block (ADR 0007 §6): present on the kinds whose
/// firms sell to citizens. The firm entity carries `Location` (of
/// `location_kind_id`) and a `RetailOffer` built from this.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetailDef {
    /// The need (from `data/balance/needs.ron`) one unit satisfies.
    pub need_id: String,
    /// Need satisfaction per unit, per-million.
    pub gain_per_unit: i64,
    /// Ticks the buyer spends consuming a unit.
    pub use_ticks: u32,
}

/// One firm kind, in data order (`Firm::kind` indexes this list).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirmDef {
    /// Stable identifier, e.g. `"bakery"`.
    pub id: String,
    /// Instances created at genesis (≥ 1).
    pub count: u32,
    /// The recipe (from `data/recipes.ron`) this kind runs.
    pub recipe_id: String,
    /// Cash seeded per instance, in mills (recorded as issuance).
    pub initial_cash_mills: i64,
    /// Stock seeded per instance (recorded in `produced`).
    pub initial_inventory: Vec<GoodQty>,
    /// Opening posted price for the output good, in mills (≥ 1).
    pub initial_price_mills: i64,
    /// The location kind (from `data/locations.ron`) every instance
    /// appears as (Phase 5, ADR 0008 §1: every firm is a place).
    pub location_kind_id: String,
    /// Worker slots per instance (≥ 1).
    pub positions: u32,
    /// Workers that must be present for a batch to start
    /// (1 ≤ min_workers ≤ positions).
    pub min_workers: u32,
    /// Present iff this kind retails to citizens.
    pub retail: Option<RetailDef>,
}

/// `data/firms.ron`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirmsConfig {
    /// Ordered firm-kind list.
    pub kinds: Vec<FirmDef>,
}

/// `data/balance/economy.ron` — the pricing controller and procurement
/// tunables (SPEC §12: cost-plus, inventory controller, bounded per-day
/// movement; ADR 0007 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EconomyConfig {
    /// Margin over unit cost, per-mille (300 = cost × 1.3 floor).
    pub markup_per_mille: i64,
    /// Fixed non-input cost per batch, in mills (enters the cost-plus
    /// floor; the modeled stand-in for rent/wear until Phases 5–6).
    pub overhead_mills_per_batch: i64,
    /// Daily posted-price movement, per-mille of the current price (the
    /// controller's bounded step; at least 1 mill).
    pub controller_step_per_mille: i64,
    /// Inventory target, in batches of output: stock above → cut price,
    /// below → raise (also the procurement fill target for inputs).
    pub inventory_target_batches: i64,
    /// Absolute posted-price floor, mills.
    pub min_price_mills: i64,
    /// Absolute posted-price ceiling, mills.
    pub max_price_mills: i64,
}

/// One resolved recipe: string ids replaced by good indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeTable {
    /// `(good index, quantity)` consumed per batch.
    pub inputs: Vec<(u32, i64)>,
    /// `(good index, units)` produced per batch; `None` = builds a home.
    pub output: Option<(u32, i64)>,
    /// Batch duration in hours.
    pub batch_hours: u32,
    /// A completed batch yields a new home (Phase 6, ADR 0009 §5).
    pub builds_home: bool,
    /// The skill (data order) this recipe trains and rewards, if any.
    pub skill: Option<u32>,
}

/// One resolved firm kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirmKindTable {
    /// Instances at genesis.
    pub count: u32,
    /// Recipe index.
    pub recipe: u32,
    /// Seeded cash per instance.
    pub initial_cash: Money,
    /// Seeded stock per instance, dense per-good (data order).
    pub initial_inventory: Vec<i64>,
    /// Opening posted price.
    pub initial_price: Money,
    /// Location kind every instance appears as (Phase 5).
    pub location_kind: u32,
    /// Worker slots per instance.
    pub positions: u32,
    /// Workers required present to start a batch.
    pub min_workers: u32,
    /// Retail block, resolved: `(need index, gain, use_ticks)` — the
    /// place is the firm's own `location_kind` (Phase 5: every firm is a
    /// place).
    pub retail: Option<(u32, i64, u32)>,
}

/// Resolved labor tunables (Phase 5): string ids replaced by indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaborTables {
    /// Shift start hour.
    pub shift_start_hour: u8,
    /// Shift end hour.
    pub shift_end_hour: u8,
    /// Labor-force age threshold, world years.
    pub min_working_age_years: u32,
    /// Reservation base, mills/day.
    pub reservation_base_mills: i64,
    /// Wealth raise at saturation, per-mille of base.
    pub reservation_wealth_per_mille: i64,
    /// Half-strength wealth, mills.
    pub reservation_half_wealth_mills: i64,
    /// Trait index (data order) discounting the reservation.
    pub reservation_trait: u32,
    /// Full-trait discount, per-mille of base.
    pub reservation_trait_discount_per_mille: i64,
    /// Bid share of marginal product, per-mille.
    pub bid_fraction_per_mille: i64,
}

/// The resolved, index-based tables the economy systems and genesis run
/// on. Construction is `data_defs::resolve_economy`'s responsibility;
/// indices are valid for the loaded data by validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EconTables {
    /// Number of goods (inventory/counter vector length).
    pub goods: usize,
    /// Per good (data order): daily spoilage per-mille.
    pub spoil_per_mille: Vec<i64>,
    /// Recipes in data order.
    pub recipes: Vec<RecipeTable>,
    /// Firm kinds in data order.
    pub firm_kinds: Vec<FirmKindTable>,
    /// Pricing/procurement tunables.
    pub economy: EconomyConfig,
    /// Labor-market tunables (Phase 5).
    pub labor: LaborTables,
    /// Banking/housing/taxes tunables (Phase 6, ADR 0009).
    pub money: crate::config_money::MoneyTables,
    /// Labor bids scale by `1 + weight/1000 × skill/1000` of the
    /// recipe's skill (Phase 7, ADR 0010 §1).
    pub labor_skill_weight_per_mille: i64,
    /// The skill (data order) the public employer's slots reward.
    pub public_skill: u32,
    /// Per-mille mastery gained per paid day (learning by doing).
    pub doing_gain_per_shift_per_mille: u16,
}

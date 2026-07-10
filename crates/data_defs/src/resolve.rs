//! Cross-config resolution into `sim_ai::AiTables` (split from `lib.rs`
//! for the SPEC §3 module-size rule).

use crate::DataDefs;

/// Resolves the loaded, validated configs into the index-based tables the
/// AI systems run on (`sim_ai::AiTables`; ADR 0006 §1 — `sim_ai` never
/// sees other sim crates' config types).
///
/// Invariant: only call with a `DataDefs` produced by [`load`] (or an
/// equally validated value); resolution relies on validation having
/// checked every cross-reference.
pub fn resolve_ai(defs: &DataDefs) -> sim_ai::AiTables {
    let need_index = |id: &str| -> u32 {
        defs.people
            .needs
            .needs
            .iter()
            .position(|n| n.id == id)
            .unwrap_or(0) as u32 // validation guarantees a hit; 0 is unreachable
    };
    let trait_index = |id: &str| -> u32 {
        defs.people
            .traits
            .traits
            .iter()
            .position(|t| t.id == id)
            .unwrap_or(0) as u32 // validation guarantees a hit
    };

    let mut need_trait: Vec<Option<(u32, u32)>> = vec![None; defs.people.needs.needs.len()];
    for weight in &defs.ai.need_trait_weights {
        let slot = need_index(&weight.need_id) as usize;
        if let Some(entry) = need_trait.get_mut(slot) {
            *entry = Some((trait_index(&weight.trait_id), weight.weight_per_mille));
        }
    }

    sim_ai::AiTables {
        travel_ticks: defs.ai.travel_ticks,
        urgency_exponent: defs.ai.urgency_exponent,
        time_cost_micro_per_tick: defs.ai.time_cost_micro_per_tick,
        max_perform_ticks: defs.ai.max_perform_ticks,
        idle_ticks: defs.ai.idle_ticks,
        plan_compile_hour: defs.ai.plan_compile_hour,
        sleep_start_minute: u16::from(defs.ai.sleep.base_start_hour) * 60,
        sleep_end_minute: u16::from(defs.ai.sleep.base_end_hour) * 60,
        sleep_max_shift_minutes: defs.ai.sleep.max_shift_minutes,
        sleep_shift_trait: trait_index(&defs.ai.sleep.shift_trait_id),
        rest_need: need_index(&defs.ai.sleep.rest_need_id),
        sleep_home_bias_micro: defs.ai.sleep.home_bias_micro,
        kind_is_home: defs.locations.kinds.iter().map(|k| k.is_home).collect(),
        kind_satisfiers: defs
            .locations
            .kinds
            .iter()
            .map(|kind| {
                kind.satisfies
                    .iter()
                    .map(|s| (need_index(&s.need_id), s.per_tick))
                    .collect()
            })
            .collect(),
        need_trait,
        mu_scale_micro: defs.ai.purchase.mu_scale_micro,
        half_wealth_mills: defs.ai.purchase.half_wealth_mills,
        work_start_minute: u16::from(defs.labor.shift_start_hour) * 60,
        work_end_minute: u16::from(defs.labor.shift_end_hour) * 60,
        work_bias_micro: defs.labor.work_bias_micro,
        work_ticks: defs.labor.work_ticks,
        work_need: need_index(&defs.labor.work_need_id),
        work_need_per_tick: defs.labor.work_need_per_tick,
    }
}

/// Resolves the loaded, validated configs into the index-based tables the
/// economy systems and genesis run on (`sim_economy::EconTables`;
/// ADR 0007 §7 — `sim_economy`/`sim_goods` never see other sim crates'
/// config types).
///
/// Invariant: only call with a `DataDefs` produced by [`crate::load`];
/// resolution relies on validation having checked every cross-reference.
pub fn resolve_economy(defs: &DataDefs) -> sim_economy::EconTables {
    let trait_index = |id: &str| -> u32 {
        defs.people
            .traits
            .traits
            .iter()
            .position(|t| t.id == id)
            .unwrap_or(0) as u32 // validation guarantees a hit
    };
    let good_index = |id: &str| -> u32 {
        defs.goods
            .goods
            .iter()
            .position(|g| g.id == id)
            .unwrap_or(0) as u32 // validation guarantees a hit
    };
    let need_index = |id: &str| -> u32 {
        defs.people
            .needs
            .needs
            .iter()
            .position(|n| n.id == id)
            .unwrap_or(0) as u32 // validation guarantees a hit
    };
    let location_kind_index = |id: &str| -> u32 {
        defs.locations
            .kinds
            .iter()
            .position(|k| k.id == id)
            .unwrap_or(0) as u32 // validation guarantees a hit
    };
    let recipe_index = |id: &str| -> u32 {
        defs.recipes
            .recipes
            .iter()
            .position(|r| r.id == id)
            .unwrap_or(0) as u32 // validation guarantees a hit
    };

    let goods = defs.goods.goods.len();
    sim_economy::EconTables {
        goods,
        spoil_per_mille: defs.goods.goods.iter().map(|g| g.spoil_per_mille).collect(),
        recipes: defs
            .recipes
            .recipes
            .iter()
            .map(|recipe| sim_economy::config::RecipeTable {
                inputs: recipe
                    .inputs
                    .iter()
                    .map(|input| (good_index(&input.good_id), input.quantity))
                    .collect(),
                output_good: good_index(&recipe.output.good_id),
                output_quantity: recipe.output.quantity,
                batch_hours: recipe.batch_hours,
            })
            .collect(),
        firm_kinds: defs
            .firms
            .kinds
            .iter()
            .map(|firm| {
                let mut initial_inventory = vec![0i64; goods];
                for entry in &firm.initial_inventory {
                    if let Some(slot) =
                        initial_inventory.get_mut(good_index(&entry.good_id) as usize)
                    {
                        *slot += entry.quantity;
                    }
                }
                sim_economy::config::FirmKindTable {
                    count: firm.count,
                    recipe: recipe_index(&firm.recipe_id),
                    initial_cash: core_types::Money::from_mills(firm.initial_cash_mills),
                    initial_inventory,
                    initial_price: core_types::Money::from_mills(firm.initial_price_mills),
                    location_kind: location_kind_index(&firm.location_kind_id),
                    positions: firm.positions,
                    min_workers: firm.min_workers,
                    retail: firm.retail.as_ref().map(|retail| {
                        (
                            need_index(&retail.need_id),
                            retail.gain_per_unit,
                            retail.use_ticks,
                        )
                    }),
                }
            })
            .collect(),
        economy: defs.economy,
        labor: sim_economy::config::LaborTables {
            shift_start_hour: defs.labor.shift_start_hour,
            shift_end_hour: defs.labor.shift_end_hour,
            min_working_age_years: defs.labor.min_working_age_years,
            reservation_base_mills: defs.labor.reservation_base_mills,
            reservation_wealth_per_mille: defs.labor.reservation_wealth_per_mille,
            reservation_half_wealth_mills: defs.labor.reservation_half_wealth_mills,
            reservation_trait: trait_index(&defs.labor.reservation_trait_id),
            reservation_trait_discount_per_mille: defs.labor.reservation_trait_discount_per_mille,
            bid_fraction_per_mille: defs.labor.bid_fraction_per_mille,
        },
    }
}

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
        sales_tax_per_mille: defs.taxes.sales_per_mille,
        school_start_minute: u16::from(defs.skills.school.start_hour) * 60,
        school_end_minute: u16::from(defs.skills.school.end_hour) * 60,
        school_bias_micro: defs.skills.school.attend_bias_micro,
        school_attend_ticks: defs.skills.school.attend_ticks,
        school_location_kind: defs
            .locations
            .kinds
            .iter()
            .position(|kind| kind.id == defs.skills.school.location_kind_id)
            .unwrap_or(0) as u32, // validation guarantees a hit
        school_taught_skill: defs
            .skills
            .skills
            .iter()
            .position(|skill| skill.id == defs.skills.school.taught_skill_id)
            .unwrap_or(0) as u32,
        school_gain_per_mille: defs.skills.school.gain_per_attendance_per_mille,
        social: sim_ai::config::SocialTables {
            edge_cap: defs.social.edge_cap,
            friend_drift_per_meeting_per_mille: defs.social.friend_drift_per_meeting_per_mille,
            romance_drift_per_meeting_per_mille: defs.social.romance_drift_per_meeting_per_mille,
            decay_per_day_per_mille: defs.social.decay_per_day_per_mille,
            romance_min_sociability_product_per_mille: defs
                .social
                .romance_min_sociability_product_per_mille,
            marriage_threshold_per_mille: defs.social.marriage_threshold_per_mille,
            social_bond_weight_per_mille: defs.social.social_bond_weight_per_mille,
            belief_cap: defs.social.belief_cap,
            drift_need: defs
                .people
                .needs
                .needs
                .iter()
                .position(|need| need.id == defs.social.drift_need_id)
                .unwrap_or(0) as u32,
            spark_trait: defs
                .people
                .traits
                .traits
                .iter()
                .position(|t| t.id == defs.social.spark_trait_id)
                .unwrap_or(0) as u32,
        },
        skill_count: defs.skills.skills.len() as u32,
        lod: sim_ai::config::LodTables {
            tier_a_cap: defs.lod.tier_a_cap,
            tier_b_cap: defs.lod.tier_b_cap,
            highlight_days: defs.lod.highlight_days,
            leisure_hours_per_day: defs.lod.leisure_hours_per_day,
        },
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
                output: recipe
                    .output
                    .as_ref()
                    .map(|output| (good_index(&output.good_id), output.quantity)),
                batch_hours: recipe.batch_hours,
                builds_home: recipe.builds_home,
                skill: recipe.skill_id.as_ref().map(|id| {
                    defs.skills
                        .skills
                        .iter()
                        .position(|skill| skill.id == *id)
                        .unwrap_or(0) as u32 // validation guarantees a hit
                }),
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
        money: sim_economy::MoneyTables {
            bank: defs.bank,
            housing: defs.housing,
            income_per_mille: defs.taxes.income_per_mille,
            sales_per_mille: defs.taxes.sales_per_mille,
            treasury_seed: core_types::Money::from_mills(defs.taxes.treasury_seed_mills),
            public_positions: defs.taxes.public_positions,
            public_wage_bid_mills: defs.taxes.public_wage_bid_mills,
            public_location_kind: location_kind_index(&defs.taxes.public_location_kind_id),
            home_location_kind: defs.locations.home_kind().unwrap_or(0),
        },
        labor_skill_weight_per_mille: defs.skills.labor_skill_weight_per_mille,
        doing_gain_per_shift_per_mille: defs.skills.doing_gain_per_shift_per_mille,
        skill_count: defs.skills.skills.len() as u32,
        public_skill: defs
            .skills
            .skills
            .iter()
            .position(|skill| skill.id == defs.skills.public_skill_id)
            .unwrap_or(0) as u32,
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

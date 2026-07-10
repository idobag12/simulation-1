//! The construction economics shared by production (the start gate) and
//! the bank (materials credit) — ADR 0009 §5. Split file for the SPEC §3
//! module-size rule.

use core_ecs::sim_interface::{BankBook, Wallet};
use core_ecs::{EcsError, Entity, World};

use crate::components::Firm;
use crate::config::{EconTables, RecipeTable};
use crate::transact::{add, mul};

/// The cost of one batch's inputs at the cheapest posted price per good
/// across producing firms (what the builder's procurement would pay).
pub(crate) fn materials_cost(
    world: &World,
    tables: &EconTables,
    recipe: &RecipeTable,
) -> Result<i64, EcsError> {
    let mut materials: i64 = 0;
    for (good, quantity) in &recipe.inputs {
        let mut best: Option<i64> = None;
        for (_, firm) in world.iter::<Firm>()? {
            let Some(other) = tables.recipes.get(firm.recipe as usize) else {
                continue;
            };
            if other.output.map(|(output, _)| output) == Some(*good) {
                let posted = firm.posted_price.mills();
                if best.is_none_or(|current| posted < current) {
                    best = Some(posted);
                }
            }
        }
        materials = add(
            materials,
            mul(best.unwrap_or(0), *quantity, "construction materials")?,
            "construction cost",
        )?;
    }
    Ok(materials)
}

/// The construction start gate (ADR 0009 §5): the data home price must
/// cover the materials (at the cheapest posted prices) plus the
/// financing surcharge a cash-short builder would pay, marked up by the
/// data margin. A higher policy rate raises the surcharge and
/// suppresses starts.
pub(crate) fn construction_pays(
    world: &World,
    tables: &EconTables,
    builder: Entity,
    recipe: &RecipeTable,
) -> Result<bool, EcsError> {
    let housing = &tables.money.housing;
    let materials = materials_cost(world, tables, recipe)?;
    let cash = world
        .get::<Wallet>(builder)?
        .map(|wallet| wallet.cash.mills())
        .unwrap_or(0);
    let financing = if cash < materials {
        let rate = world
            .iter::<BankBook>()?
            .next()
            .map(|(_, book)| book.policy_rate_per_million_daily)
            .unwrap_or(0)
            + tables.money.bank.risk_premium_per_million_daily;
        mul(
            mul(materials, rate, "financing rate")?,
            tables.money.bank.repay_term_days,
            "financing term",
        )? / 1_000_000
    } else {
        0
    };
    let hurdle = mul(
        add(materials, financing, "construction hurdle")?,
        1000 + housing.construction_margin_per_mille,
        "construction margin",
    )? / 1000;
    // Against the MEASURED market price of homes — the last purchase
    // clearing's average, or the data floor before any sale (ADR 0009
    // §5: prices measured, never set).
    let anchor = crate::housing_market::home_price_anchor(world, housing.home_price_mills)?;
    Ok(anchor >= hurdle)
}

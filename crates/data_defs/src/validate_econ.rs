//! Startup validation for the Phase 4 economy data (ADR 0007 §7): goods,
//! recipes, firms, and the market tunables, including every cross-file
//! reference (recipes↔goods, firms↔recipes↔location kinds↔needs). Split
//! file for the SPEC §3 module-size rule.

use std::path::Path;

use crate::DataError;
use crate::validate::verr;

/// Validates the economy configs and their cross-references. Rules
/// (ADR 0007 §7): ids unique; every referenced id exists; every good has
/// a producer; retail goods are produced by their firm's recipe; exactly
/// one firm kind per retail location kind; retail location kinds have
/// count 0 (their instances come from firm genesis); non-retail location
/// kinds still may not be dead data (the check `validate_locations`
/// delegated here).
pub(crate) fn validate_economy(
    data_root: &Path,
    goods: &sim_goods::config::GoodsConfig,
    recipes: &sim_economy::config::RecipesConfig,
    firms: &sim_economy::config::FirmsConfig,
    economy: &sim_economy::config::EconomyConfig,
    people: &sim_people::config::PeopleConfig,
    locations: &sim_world::config::LocationsConfig,
) -> Result<(), DataError> {
    validate_goods(data_root, goods)?;
    validate_recipes(data_root, goods, recipes)?;
    validate_firms(data_root, goods, recipes, firms, people, locations)?;
    validate_market(data_root, economy)?;
    validate_location_liveness(data_root, firms, locations)
}

fn validate_goods(
    data_root: &Path,
    goods: &sim_goods::config::GoodsConfig,
) -> Result<(), DataError> {
    let file = "goods.ron";
    if goods.goods.is_empty() {
        return Err(verr(data_root, file, "at least one good required".into()));
    }
    let mut seen: Vec<&str> = Vec::new();
    for good in &goods.goods {
        if seen.contains(&good.id.as_str()) {
            return Err(verr(
                data_root,
                file,
                format!("duplicate good id `{}`", good.id),
            ));
        }
        seen.push(&good.id);
        if !(0..=1000).contains(&good.spoil_per_mille) {
            return Err(verr(
                data_root,
                file,
                format!("good `{}` spoil_per_mille must be within 0..=1000", good.id),
            ));
        }
    }
    Ok(())
}

fn validate_recipes(
    data_root: &Path,
    goods: &sim_goods::config::GoodsConfig,
    recipes: &sim_economy::config::RecipesConfig,
) -> Result<(), DataError> {
    let file = "recipes.ron";
    let good_exists = |id: &str| goods.goods.iter().any(|g| g.id == id);
    if recipes.recipes.is_empty() {
        return Err(verr(data_root, file, "at least one recipe required".into()));
    }
    let mut seen: Vec<&str> = Vec::new();
    for recipe in &recipes.recipes {
        if seen.contains(&recipe.id.as_str()) {
            return Err(verr(
                data_root,
                file,
                format!("duplicate recipe id `{}`", recipe.id),
            ));
        }
        seen.push(&recipe.id);
        if recipe.batch_hours == 0 {
            return Err(verr(
                data_root,
                file,
                format!("recipe `{}` batch_hours must be >= 1", recipe.id),
            ));
        }
        let entries = recipe.inputs.iter().chain(std::iter::once(&recipe.output));
        for entry in entries {
            if !good_exists(&entry.good_id) {
                return Err(verr(
                    data_root,
                    file,
                    format!(
                        "recipe `{}` references unknown good `{}`",
                        recipe.id, entry.good_id
                    ),
                ));
            }
            if entry.quantity < 1 {
                return Err(verr(
                    data_root,
                    file,
                    format!(
                        "recipe `{}` quantity for `{}` must be >= 1",
                        recipe.id, entry.good_id
                    ),
                ));
            }
        }
        if recipe
            .inputs
            .iter()
            .any(|input| input.good_id == recipe.output.good_id)
        {
            return Err(verr(
                data_root,
                file,
                format!("recipe `{}` outputs one of its own inputs", recipe.id),
            ));
        }
        // Duplicate input goods would defeat the batch-start stock check
        // (each entry is tested against the same stock independently) and
        // double-spend an inventory below zero (ADR 0007 §8b).
        let mut input_goods: Vec<&str> = Vec::new();
        for input in &recipe.inputs {
            if input_goods.contains(&input.good_id.as_str()) {
                return Err(verr(
                    data_root,
                    file,
                    format!(
                        "recipe `{}` lists input good `{}` more than once",
                        recipe.id, input.good_id
                    ),
                ));
            }
            input_goods.push(&input.good_id);
        }
    }
    Ok(())
}

fn validate_firms(
    data_root: &Path,
    goods: &sim_goods::config::GoodsConfig,
    recipes: &sim_economy::config::RecipesConfig,
    firms: &sim_economy::config::FirmsConfig,
    people: &sim_people::config::PeopleConfig,
    locations: &sim_world::config::LocationsConfig,
) -> Result<(), DataError> {
    let file = "firms.ron";
    if firms.kinds.is_empty() {
        return Err(verr(
            data_root,
            file,
            "at least one firm kind required".into(),
        ));
    }
    let mut seen: Vec<&str> = Vec::new();
    let mut claimed_location_kinds: Vec<&str> = Vec::new();
    for firm in &firms.kinds {
        if seen.contains(&firm.id.as_str()) {
            return Err(verr(
                data_root,
                file,
                format!("duplicate firm kind id `{}`", firm.id),
            ));
        }
        seen.push(&firm.id);
        if firm.count == 0 {
            return Err(verr(
                data_root,
                file,
                format!("firm kind `{}` has count 0 (dead data)", firm.id),
            ));
        }
        if firm.initial_cash_mills < 0 || firm.initial_price_mills < 1 {
            return Err(verr(
                data_root,
                file,
                format!(
                    "firm kind `{}` needs initial_cash_mills >= 0 and initial_price_mills >= 1",
                    firm.id
                ),
            ));
        }
        // The retail offer always sells the recipe's output good — that
        // cross-reference holds by construction (genesis builds the offer
        // from the recipe), so only the recipe itself needs to exist.
        if !recipes.recipes.iter().any(|r| r.id == firm.recipe_id) {
            return Err(verr(
                data_root,
                file,
                format!(
                    "firm kind `{}` references unknown recipe `{}`",
                    firm.id, firm.recipe_id
                ),
            ));
        }
        for entry in &firm.initial_inventory {
            if !goods.goods.iter().any(|g| g.id == entry.good_id) {
                return Err(verr(
                    data_root,
                    file,
                    format!(
                        "firm kind `{}` seeds unknown good `{}`",
                        firm.id, entry.good_id
                    ),
                ));
            }
            if entry.quantity < 1 {
                return Err(verr(
                    data_root,
                    file,
                    format!(
                        "firm kind `{}` seeded quantity for `{}` must be >= 1",
                        firm.id, entry.good_id
                    ),
                ));
            }
        }
        // Every firm is a place (ADR 0008 §1): the kind's location kind
        // must exist, be public with count 0 (firm genesis creates the
        // instances), and be claimed by exactly one firm kind.
        let Some(location_kind) = locations
            .kinds
            .iter()
            .find(|k| k.id == firm.location_kind_id)
        else {
            return Err(verr(
                data_root,
                file,
                format!(
                    "firm kind `{}` names unknown location kind `{}`",
                    firm.id, firm.location_kind_id
                ),
            ));
        };
        if location_kind.is_home || location_kind.count != 0 {
            return Err(verr(
                data_root,
                file,
                format!(
                    "firm location kind `{}` must be public with count 0 \
                     (firm genesis creates its instances)",
                    firm.location_kind_id
                ),
            ));
        }
        if claimed_location_kinds.contains(&firm.location_kind_id.as_str()) {
            return Err(verr(
                data_root,
                file,
                format!(
                    "location kind `{}` is claimed by more than one firm kind",
                    firm.location_kind_id
                ),
            ));
        }
        claimed_location_kinds.push(&firm.location_kind_id);
        if firm.positions == 0 || firm.min_workers == 0 || firm.min_workers > firm.positions {
            return Err(verr(
                data_root,
                file,
                format!(
                    "firm kind `{}` needs 1 <= min_workers <= positions",
                    firm.id
                ),
            ));
        }
        if let Some(retail) = &firm.retail {
            if !people.needs.needs.iter().any(|n| n.id == retail.need_id) {
                return Err(verr(
                    data_root,
                    file,
                    format!(
                        "firm kind `{}` retail references unknown need `{}`",
                        firm.id, retail.need_id
                    ),
                ));
            }
            if retail.gain_per_unit < 1 || retail.use_ticks == 0 {
                return Err(verr(
                    data_root,
                    file,
                    format!(
                        "firm kind `{}` retail needs gain_per_unit >= 1 and use_ticks >= 1",
                        firm.id
                    ),
                ));
            }
        }
    }
    // Every good has a producer (ADR 0007 §7): some firm kind's recipe
    // outputs it — otherwise its consumers starve by construction.
    for good in &goods.goods {
        let produced = firms.kinds.iter().any(|firm| {
            recipes
                .recipes
                .iter()
                .any(|r| r.id == firm.recipe_id && r.output.good_id == good.id)
        });
        if !produced {
            return Err(verr(
                data_root,
                file,
                format!("no firm kind produces good `{}`", good.id),
            ));
        }
    }
    Ok(())
}

fn validate_market(
    data_root: &Path,
    economy: &sim_economy::config::EconomyConfig,
) -> Result<(), DataError> {
    let file = "balance/economy.ron";
    if economy.markup_per_mille < 0 {
        return Err(verr(
            data_root,
            file,
            "markup_per_mille must be >= 0".into(),
        ));
    }
    if economy.overhead_mills_per_batch < 0 {
        return Err(verr(
            data_root,
            file,
            "overhead_mills_per_batch must be >= 0".into(),
        ));
    }
    if !(1..=1000).contains(&economy.controller_step_per_mille) {
        return Err(verr(
            data_root,
            file,
            "controller_step_per_mille must be within 1..=1000".into(),
        ));
    }
    if economy.inventory_target_batches < 1 {
        return Err(verr(
            data_root,
            file,
            "inventory_target_batches must be >= 1".into(),
        ));
    }
    if economy.min_price_mills < 1 || economy.min_price_mills > economy.max_price_mills {
        return Err(verr(
            data_root,
            file,
            "price bounds must satisfy 1 <= min <= max".into(),
        ));
    }
    Ok(())
}

/// The dead-data check `validate_locations` delegated here: every
/// location kind must either satisfy a need and get instantiated, or be
/// claimed by a retail firm kind.
fn validate_location_liveness(
    data_root: &Path,
    firms: &sim_economy::config::FirmsConfig,
    locations: &sim_world::config::LocationsConfig,
) -> Result<(), DataError> {
    let file = "locations.ron";
    for kind in &locations.kinds {
        let claimed = firms
            .kinds
            .iter()
            .any(|firm| firm.location_kind_id == kind.id);
        if claimed {
            continue;
        }
        if !kind.is_home && kind.count == 0 {
            return Err(verr(
                data_root,
                file,
                format!(
                    "public kind `{}` has count 0 and no firm claims it (dead data)",
                    kind.id
                ),
            ));
        }
        if kind.satisfies.is_empty() {
            return Err(verr(
                data_root,
                file,
                format!(
                    "kind `{}` satisfies nothing and no firm claims it (dead data)",
                    kind.id
                ),
            ));
        }
    }
    Ok(())
}

/// Validates `data/balance/labor.ron` (ADR 0008 §7), including its
/// cross-references into needs and traits.
pub(crate) fn validate_labor(
    data_root: &Path,
    labor: &sim_economy::config::LaborConfig,
    people: &sim_people::config::PeopleConfig,
) -> Result<(), DataError> {
    let file = "balance/labor.ron";
    let e = |message: String| verr(data_root, file, message);

    if labor.shift_start_hour >= 24 || labor.shift_end_hour > 24 {
        return Err(e("shift hours must be within the day".into()));
    }
    if labor.shift_start_hour >= labor.shift_end_hour {
        return Err(e(
            "shift_start_hour must be before shift_end_hour (no overnight shifts)".into(),
        ));
    }
    if labor.reservation_base_mills < 1 {
        return Err(e("reservation_base_mills must be >= 1".into()));
    }
    for (name, value) in [
        (
            "reservation_wealth_per_mille",
            labor.reservation_wealth_per_mille,
        ),
        (
            "reservation_trait_discount_per_mille",
            labor.reservation_trait_discount_per_mille,
        ),
    ] {
        if !(0..=1000).contains(&value) {
            return Err(e(format!("{name} must be within 0..=1000")));
        }
    }
    if labor.reservation_half_wealth_mills < 1 {
        return Err(e("reservation_half_wealth_mills must be >= 1".into()));
    }
    if !(1..=1000).contains(&labor.bid_fraction_per_mille) {
        return Err(e("bid_fraction_per_mille must be within 1..=1000".into()));
    }
    if labor.work_bias_micro < 0 {
        return Err(e("work_bias_micro must be >= 0".into()));
    }
    if labor.work_ticks == 0 {
        return Err(e("work_ticks must be >= 1".into()));
    }
    if labor.work_need_per_tick < 0 {
        return Err(e("work_need_per_tick must be >= 0".into()));
    }
    if !people
        .traits
        .traits
        .iter()
        .any(|t| t.id == labor.reservation_trait_id)
    {
        return Err(e(format!(
            "reservation_trait_id `{}` is not a defined trait",
            labor.reservation_trait_id
        )));
    }
    if !people
        .needs
        .needs
        .iter()
        .any(|n| n.id == labor.work_need_id)
    {
        return Err(e(format!(
            "work_need_id `{}` is not a defined need",
            labor.work_need_id
        )));
    }
    Ok(())
}

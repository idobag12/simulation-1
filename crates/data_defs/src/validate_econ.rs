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
    let mut retail_location_kinds: Vec<&str> = Vec::new();
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
        if let Some(retail) = &firm.retail {
            if !locations
                .kinds
                .iter()
                .any(|k| k.id == retail.location_kind_id)
            {
                return Err(verr(
                    data_root,
                    file,
                    format!(
                        "firm kind `{}` retails at unknown location kind `{}`",
                        firm.id, retail.location_kind_id
                    ),
                ));
            }
            if retail_location_kinds.contains(&retail.location_kind_id.as_str()) {
                return Err(verr(
                    data_root,
                    file,
                    format!(
                        "location kind `{}` is claimed by more than one retail firm kind",
                        retail.location_kind_id
                    ),
                ));
            }
            retail_location_kinds.push(&retail.location_kind_id);
            if let Some(kind) = locations
                .kinds
                .iter()
                .find(|k| k.id == retail.location_kind_id)
                && (kind.is_home || kind.count != 0)
            {
                return Err(verr(
                    data_root,
                    file,
                    format!(
                        "retail location kind `{}` must be public with count 0 \
                         (firm genesis creates its instances)",
                        retail.location_kind_id
                    ),
                ));
            }
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
        let retail = firms.kinds.iter().any(|firm| {
            firm.retail
                .as_ref()
                .is_some_and(|r| r.location_kind_id == kind.id)
        });
        if retail {
            continue;
        }
        if !kind.is_home && kind.count == 0 {
            return Err(verr(
                data_root,
                file,
                format!(
                    "public kind `{}` has count 0 and no retail firm (dead data)",
                    kind.id
                ),
            ));
        }
        if kind.satisfies.is_empty() {
            return Err(verr(
                data_root,
                file,
                format!(
                    "kind `{}` satisfies nothing and no retail firm claims it (dead data)",
                    kind.id
                ),
            ));
        }
    }
    Ok(())
}

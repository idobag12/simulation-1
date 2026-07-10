//! Inspector v1: headless queries over a loaded world (SPEC §13,
//! SPEC §15 Phase 2). Pure formatting over world state — no mutation.

use core_ecs::{Entity, World};
use data_defs::DataDefs;
use sim_time::Simulation;

use crate::runner;

/// Renders a full dump of the entity at `index`: a citizen (identity, age,
/// needs, personality, household) or a household (member roster). Errors
/// if no live entity occupies the index.
pub fn inspect_entity(sim: &Simulation, defs: &DataDefs, index: u32) -> Result<String, String> {
    let world = sim.world();
    let entity = world
        .iter_entities()
        .find(|e| e.index() == index)
        .ok_or(format!("no live entity at index {index}"))?;

    if let Some(identity) = world.get::<sim_people::Identity>(entity).map_err(err)? {
        return citizen_report(sim, defs, entity, identity.clone());
    }
    if let Some(household) = world.get::<sim_people::Household>(entity).map_err(err)? {
        let mut out = format!(
            "household #{index}: {} member(s)\n",
            household.members.len()
        );
        for member in &household.members {
            let name = world
                .get::<sim_people::Identity>(*member)
                .map_err(err)?
                .map(|i| format!("{} {}", i.given_name, i.family_name))
                .unwrap_or_else(|| "<unknown>".into());
            out.push_str(&format!("  - #{} {}\n", member.index(), name));
        }
        return Ok(out);
    }
    Ok(format!(
        "entity #{index} is live but carries no people components (fixture or other)"
    ))
}

fn err(e: core_ecs::EcsError) -> String {
    e.to_string()
}

fn citizen_report(
    sim: &Simulation,
    defs: &DataDefs,
    entity: Entity,
    identity: sim_people::Identity,
) -> Result<String, String> {
    let world = sim.world();
    let ticks_per_year = runner::ticks_per_year(defs);
    let age_years = identity.age_years(sim.tick(), ticks_per_year);
    let age_days = identity.age_days_into_year(sim.tick(), ticks_per_year);

    let mut out = format!(
        "citizen #{}: {} {} ({:?}, age {age_years}y {age_days}d)\n",
        entity.index(),
        identity.given_name,
        identity.family_name,
        identity.sex,
    );

    if let Some(needs) = world.get::<sim_people::Needs>(entity).map_err(err)? {
        out.push_str("needs (per-million):\n");
        for (def, level) in defs.people.needs.needs.iter().zip(&needs.levels) {
            out.push_str(&format!("  {:<10} {}\n", def.id, level.raw()));
        }
    }
    if let Some(personality) = world.get::<sim_people::Personality>(entity).map_err(err)? {
        out.push_str("personality (per-mille):\n");
        for (def, weight) in defs.people.traits.traits.iter().zip(&personality.weights) {
            out.push_str(&format!("  {:<16} {weight}\n", def.id));
        }
    }
    if let Some(wallet) = world
        .get::<core_ecs::sim_interface::Wallet>(entity)
        .map_err(err)?
    {
        out.push_str(&format!("wallet: {}\n", wallet.cash));
    }
    if let Some(employment) = world
        .get::<core_ecs::sim_interface::Employment>(entity)
        .map_err(err)?
    {
        out.push_str(&format!(
            "job: {} at {} per day\n",
            location_label(world, defs, employment.employer)?,
            employment.wage_per_day
        ));
    }
    if let Some(member) = world
        .get::<sim_people::HouseholdMember>(entity)
        .map_err(err)?
    {
        let household = member.household;
        out.push_str(&format!("household: #{}\n", household.index()));
        if let Some(h) = world.get::<sim_people::Household>(household).map_err(err)? {
            for other in h.members.iter().filter(|m| **m != entity) {
                let name = world
                    .get::<sim_people::Identity>(*other)
                    .map_err(err)?
                    .map(|i| format!("{} {}", i.given_name, i.family_name))
                    .unwrap_or_else(|| "<unknown>".into());
                out.push_str(&format!("  with #{} {}\n", other.index(), name));
            }
        }
    }
    out.push_str(&activity_report(world, defs, entity)?);
    Ok(out)
}

/// Resolves a location entity to `"kind #index"` for display.
fn location_label(world: &World, defs: &DataDefs, location: Entity) -> Result<String, String> {
    let kind_name = world
        .get::<core_ecs::sim_interface::Location>(location)
        .map_err(err)?
        .and_then(|l| defs.locations.kinds.get(l.kind as usize))
        .map(|k| k.id.clone())
        .unwrap_or_else(|| "<not a location>".into());
    Ok(format!("{kind_name} #{}", location.index()))
}

fn need_name(defs: &DataDefs, need_index: u32) -> &str {
    defs.people
        .needs
        .needs
        .get(need_index as usize)
        .map(|n| n.id.as_str())
        .unwrap_or("<unknown need>")
}

/// The AI section of a citizen dump: position, current action, and the
/// full scored candidate list of the last decision (SPEC §11/§13 — every
/// Tier A decision inspectable from a save).
fn activity_report(world: &World, defs: &DataDefs, entity: Entity) -> Result<String, String> {
    let mut out = String::new();
    if let Some(position) = world
        .get::<core_ecs::sim_interface::Position>(entity)
        .map_err(err)?
    {
        out.push_str(&format!(
            "at: {}\n",
            location_label(world, defs, position.at)?
        ));
    }
    if let Some(action) = world.get::<sim_ai::CurrentAction>(entity).map_err(err)? {
        let line = match action {
            sim_ai::CurrentAction::Travel {
                target,
                need_index,
                remaining,
            } => format!(
                "doing: traveling to {} for {} ({remaining} ticks left)\n",
                location_label(world, defs, *target)?,
                need_name(defs, *need_index),
            ),
            sim_ai::CurrentAction::Perform {
                at,
                need_index,
                remaining,
            } => format!(
                "doing: satisfying {} at {} (up to {remaining} more ticks)\n",
                need_name(defs, *need_index),
                location_label(world, defs, *at)?,
            ),
            sim_ai::CurrentAction::Idle { remaining } => {
                format!("doing: idling ({remaining} ticks left)\n")
            }
            sim_ai::CurrentAction::BuyTravel { target, remaining } => format!(
                "doing: heading to {} to buy ({remaining} ticks left)\n",
                location_label(world, defs, *target)?,
            ),
            sim_ai::CurrentAction::BuyPending { at } => {
                format!("doing: buying at {}\n", location_label(world, defs, *at)?,)
            }
            sim_ai::CurrentAction::Consume {
                at,
                need_index,
                remaining,
            } => format!(
                "doing: consuming a purchase for {} at {} ({remaining} ticks left)\n",
                need_name(defs, *need_index),
                location_label(world, defs, *at)?,
            ),
            sim_ai::CurrentAction::WorkTravel { target, remaining } => format!(
                "doing: commuting to {} ({remaining} ticks left)\n",
                location_label(world, defs, *target)?,
            ),
            sim_ai::CurrentAction::Work { at, remaining } => format!(
                "doing: working at {} ({remaining} ticks left in the stint)\n",
                location_label(world, defs, *at)?,
            ),
        };
        out.push_str(&line);
    }
    if let Some(decision) = world.get::<sim_ai::LastDecision>(entity).map_err(err)? {
        out.push_str(&format!(
            "last decision (tick {}), {} candidates:\n",
            decision.tick,
            decision.candidates.len()
        ));
        for (index, candidate) in decision.candidates.iter().enumerate() {
            let marker = if index as u32 == decision.chosen {
                ">"
            } else {
                " "
            };
            let what = match candidate.action {
                sim_ai::CandidateAction::Satisfy {
                    location,
                    need_index,
                } => format!(
                    "{} at {}",
                    need_name(defs, need_index),
                    location_label(world, defs, location)?
                ),
                sim_ai::CandidateAction::Idle => "idle".into(),
                sim_ai::CandidateAction::Buy {
                    location,
                    need_index,
                } => format!(
                    "buy for {} at {}",
                    need_name(defs, need_index),
                    location_label(world, defs, location)?
                ),
                sim_ai::CandidateAction::Work { location } => {
                    format!("work at {}", location_label(world, defs, location)?)
                }
            };
            out.push_str(&format!(
                " {marker} {what}: {} micro\n",
                candidate.score_micro
            ));
        }
    }
    Ok(out)
}

/// Renders the town's demographic summary: population, sex counts, age
/// histogram by decade, household-size histogram — the observables the
/// Phase 2 demographic bands are asserted against (ADR 0005 §7).
pub fn demography(sim: &Simulation, defs: &DataDefs) -> Result<String, String> {
    let world = sim.world();
    let ticks_per_year = runner::ticks_per_year(defs);
    let now = sim.tick();

    let mut population = 0u32;
    let mut male = 0u32;
    let mut ages_by_decade: Vec<u32> = Vec::new();
    for (_, identity) in world.iter::<sim_people::Identity>().map_err(err)? {
        population += 1;
        if identity.sex == sim_people::Sex::Male {
            male += 1;
        }
        let decade = (identity.age_years(now, ticks_per_year) / 10) as usize;
        if ages_by_decade.len() <= decade {
            ages_by_decade.resize(decade + 1, 0);
        }
        ages_by_decade[decade] += 1;
    }

    let mut household_sizes: Vec<u32> = Vec::new();
    let mut households = 0u32;
    for (_, household) in world.iter::<sim_people::Household>().map_err(err)? {
        households += 1;
        let size = household.members.len();
        if household_sizes.len() <= size {
            household_sizes.resize(size + 1, 0);
        }
        household_sizes[size] += 1;
    }

    let mut out = format!(
        "tick {now}: population {population} ({male} male / {} female), {households} households\n",
        population - male
    );
    out.push_str("age decades:\n");
    for (decade, count) in ages_by_decade.iter().enumerate() {
        out.push_str(&format!(
            "  {:>3}-{:>3}: {count}\n",
            decade * 10,
            decade * 10 + 9
        ));
    }
    out.push_str("household sizes:\n");
    for (size, count) in household_sizes.iter().enumerate().skip(1) {
        if *count > 0 {
            out.push_str(&format!("  {size}: {count}\n"));
        }
    }
    Ok(out)
}

/// Counts live citizens (the `Identity` store population).
pub fn population(world: &World) -> Result<u32, core_ecs::EcsError> {
    Ok(world.iter::<sim_people::Identity>()?.count() as u32)
}

/// Renders the town's economy: every firm's posted price, stock, cash,
/// and books; the conservation counters; and the live audit verdict
/// (SPEC §13 — the Phase 4 observables).
pub fn economy(sim: &Simulation, defs: &DataDefs) -> Result<String, String> {
    use core_ecs::sim_interface::{EconCounters, FirmBooks, Inventory, Wallet};

    let world = sim.world();
    let good_name = |good: usize| {
        defs.goods
            .goods
            .get(good)
            .map(|g| g.id.as_str())
            .unwrap_or("<unknown good>")
    };

    let mut out = format!("tick {}: economy\n", sim.tick());
    let mut firm_count = 0;
    for (entity, firm) in world.iter::<sim_economy::Firm>().map_err(err)? {
        firm_count += 1;
        let kind = defs
            .firms
            .kinds
            .get(firm.kind as usize)
            .map(|k| k.id.as_str())
            .unwrap_or("<unknown kind>");
        let output = defs
            .recipes
            .recipes
            .get(firm.recipe as usize)
            .map(|r| r.output.good_id.as_str())
            .unwrap_or("<unknown>");
        out.push_str(&format!(
            "firm #{} {kind}: posts {output} at {}\n",
            entity.index(),
            firm.posted_price
        ));
        if let Some(wallet) = world.get::<Wallet>(entity).map_err(err)? {
            out.push_str(&format!("  cash {}", wallet.cash));
        }
        if let Some(books) = world.get::<FirmBooks>(entity).map_err(err)? {
            out.push_str(&format!(
                " | revenue {} expenses {}",
                books.revenue, books.expenses
            ));
        }
        out.push('\n');
        if let Some(inventory) = world.get::<Inventory>(entity).map_err(err)? {
            out.push_str("  stock:");
            for (good, quantity) in inventory.quantities.iter().enumerate() {
                if *quantity > 0 {
                    out.push_str(&format!(" {} {}", good_name(good), quantity));
                }
            }
            out.push('\n');
        }
    }
    if firm_count == 0 {
        out.push_str("no firms (pre-economy world)\n");
    }

    if let Some((_, counters)) = world.iter::<EconCounters>().map_err(err)?.next() {
        out.push_str(&format!("issued: {}\n", counters.issued));
        out.push_str("counters (produced/citizens/production/spoiled):\n");
        for good in 0..counters.produced.len() {
            out.push_str(&format!(
                "  {:<10} {} / {} / {} / {}\n",
                good_name(good),
                counters.produced.get(good).copied().unwrap_or(0),
                counters
                    .consumed_by_citizens
                    .get(good)
                    .copied()
                    .unwrap_or(0),
                counters
                    .consumed_in_production
                    .get(good)
                    .copied()
                    .unwrap_or(0),
                counters.spoiled.get(good).copied().unwrap_or(0),
            ));
        }
    }
    if let Some((_, stats)) = world
        .iter::<core_ecs::sim_interface::LaborStats>()
        .map_err(err)?
        .next()
    {
        out.push_str(&format!(
            "labor: {} working-age, {} employed, {} sought, {} unmatched \
             (lifetime {} hires / {} firings)\n",
            stats.working_age,
            stats.employed,
            stats.seeking,
            stats.unmatched,
            stats.hires,
            stats.firings,
        ));
    }
    match debug_tools::audit_economy(world) {
        Ok(true) => out.push_str("audit: PASS\n"),
        Ok(false) => out.push_str("audit: no economy to audit\n"),
        Err(e) => out.push_str(&format!("audit: FAIL — {e}\n")),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{self, WorldSpec};
    use core_types::Seed;

    fn defs() -> DataDefs {
        let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data"));
        data_defs::load(root).expect("live data loads")
    }

    /// Content assertions for the inspector outputs (Phase 2 review):
    /// every section ADR 0005 §7 promises actually appears, with real
    /// values.
    #[test]
    fn inspector_outputs_contain_the_promised_sections() {
        let defs = defs();
        let spec = WorldSpec::town(Seed::new(51), 30);
        let (sim, _) = runner::build_simulation(&spec, &defs).expect("build");

        // Entity 0 is the first genesis household; its first member is a
        // citizen.
        let household_dump = inspect_entity(&sim, &defs, 0).expect("household dump");
        assert!(
            household_dump.starts_with("household #0:"),
            "{household_dump}"
        );

        let citizen_dump = inspect_entity(&sim, &defs, 1).expect("citizen dump");
        assert!(citizen_dump.starts_with("citizen #1:"), "{citizen_dump}");
        for section in [
            "age ", // "age {years}y {days}d" (ADR 0005 §7: years/days)
            "y ",
            "needs (per-million):",
            "hunger",
            "personality (per-mille):",
            "industriousness",
            "household: #0",
        ] {
            assert!(
                citizen_dump.contains(section),
                "missing `{section}` in:\n{citizen_dump}"
            );
        }

        let summary = demography(&sim, &defs).expect("demography");
        assert!(summary.contains("population 30"), "{summary}");
        assert!(summary.contains("age decades:"), "{summary}");
        assert!(summary.contains("household sizes:"), "{summary}");

        let missing = inspect_entity(&sim, &defs, 9_999);
        assert!(missing.is_err(), "dead index must error, got {missing:?}");
    }
}

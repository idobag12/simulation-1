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

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

pub(crate) fn err(e: core_ecs::EcsError) -> String {
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
    // Phase 6 money state (SPEC §13): most wealth sits in the vault.
    if let Some((_, book)) = world
        .iter::<core_ecs::sim_interface::BankBook>()
        .map_err(err)?
        .next()
    {
        if let Some((_, balance)) = book.deposits.iter().find(|(owner, _)| *owner == entity) {
            out.push_str(&format!("deposits: {balance}\n"));
        }
        for loan in book.loans.iter().filter(|loan| loan.borrower == entity) {
            out.push_str(&format!(
                "loan: owes {} at {} per day\n",
                loan.principal, loan.day_payment
            ));
        }
    }
    if let Some(tenancy) = world
        .get::<core_ecs::sim_interface::Tenancy>(entity)
        .map_err(err)?
    {
        out.push_str(&format!(
            "rents: home #{} at {} per day\n",
            tenancy.home.index(),
            tenancy.rent_per_day
        ));
    }
    for (home, ownership) in world
        .iter::<core_ecs::sim_interface::Ownership>()
        .map_err(err)?
    {
        if ownership.owner == entity {
            out.push_str(&format!("owns: home #{}\n", home.index()));
        }
    }
    if let Some(status) = world
        .get::<core_ecs::sim_interface::BorrowerStatus>(entity)
        .map_err(err)?
    {
        out.push_str(&format!(
            "credit: uncreditworthy until day {}\n",
            status.uncreditworthy_until_day
        ));
    }
    // Phase 8 (SPEC §13): the simulation tier (missing row = embodied).
    let tier = match world
        .get::<core_ecs::sim_interface::LodTier>(entity)
        .map_err(err)?
    {
        Some(row) => match row.tier {
            core_ecs::sim_interface::Tier::A => "A (embodied)",
            core_ecs::sim_interface::Tier::B => "B (scheduled)",
            core_ecs::sim_interface::Tier::C => "C (statistical)",
        },
        None => "A (embodied; unassigned)",
    };
    out.push_str(&format!("tier: {tier}\n"));
    // Phase 7 social state (SPEC §13): skills, bonds, beliefs.
    if let Some(skills) = world
        .get::<core_ecs::sim_interface::Skills>(entity)
        .map_err(err)?
        && skills.levels.iter().any(|level| *level > 0)
    {
        out.push_str("skills (per-mille):\n");
        for (def, level) in defs.skills.skills.iter().zip(&skills.levels) {
            out.push_str(&format!("  {:<10} {level}\n", def.id));
        }
    }
    if let Some(relationships) = world
        .get::<core_ecs::sim_interface::Relationships>(entity)
        .map_err(err)?
    {
        for edge in &relationships.edges {
            let name = world
                .get::<sim_people::Identity>(edge.other)
                .map_err(err)?
                .map(|identity| format!("{} {}", identity.given_name, identity.family_name))
                .unwrap_or_else(|| format!("#{}", edge.other.index()));
            out.push_str(&format!(
                "bond: {:?} {} ({} per-mille)\n",
                edge.kind, name, edge.strength_per_mille
            ));
        }
    }
    if let Some(beliefs) = world
        .get::<core_ecs::sim_interface::Beliefs>(entity)
        .map_err(err)?
    {
        for (shop, price) in &beliefs.prices {
            out.push_str(&format!(
                "believes: {} charges {}\n",
                location_label(world, defs, *shop)?,
                price
            ));
        }
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
            sim_ai::CurrentAction::SchoolTravel { target, remaining } => format!(
                "doing: heading to {} for school ({remaining} ticks left)\n",
                location_label(world, defs, *target)?,
            ),
            sim_ai::CurrentAction::Attend { at, remaining } => format!(
                "doing: attending school at {} ({remaining} ticks left)\n",
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
                sim_ai::CandidateAction::AttendSchool { location } => {
                    format!("school at {}", location_label(world, defs, location)?)
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

// The economy/bank/treasury report lives in `inspect_econ.rs`
// (SPEC §3 module-size rule).
#[path = "inspect_econ.rs"]
mod econ_report;
pub use econ_report::economy;

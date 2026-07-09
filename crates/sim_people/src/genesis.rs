//! Deterministic initial-population generation (ADR 0005 §5).
//!
//! Invariants:
//! - Bit-identical towns from equal (seed, config, citizen count): every
//!   draw comes from the dedicated `people.genesis` stream in a fixed
//!   order; iteration is construction order.
//! - Every citizen ends up in exactly one household; `Household.members`
//!   lists are sorted by entity index.

use core_ecs::{EcsError, Entity, World};
use core_rng::RngCore;

use crate::components::{Household, HouseholdMember, Identity, NeedLevel, Needs, Personality, Sex};
use crate::config::PeopleConfig;

/// RNG stream for all genesis draws.
pub const GENESIS_STREAM: &str = "people.genesis";

/// Uniform draw in `[min, max]` (inclusive) from `value`. Modulo bias is
/// acceptable for genesis sampling (documented; ranges are tiny relative
/// to u64).
fn in_range(value: u64, min: u64, max: u64) -> u64 {
    min + value % (max - min + 1)
}

/// Spawns `count` citizens grouped into households, with data-driven ages,
/// sexes, names, traits, and initial needs. Call once at world assembly,
/// before any ticks.
///
/// `ticks_per_year` comes from the calendar (age → birth_tick conversion).
///
/// Household sizes are drawn from `[household_min, household_max]`, except
/// that the FINAL household absorbs however many citizens remain and may
/// therefore be smaller than `household_min` (documented on
/// `DemographicsConfig`; e.g. 10 citizens with sizes 4,4 leave a tail of 2).
pub fn populate(
    world: &mut World,
    config: &PeopleConfig,
    count: u32,
    ticks_per_year: u64,
) -> Result<(), EcsError> {
    let mut remaining = count;
    while remaining > 0 {
        let size_draw = world.rng(GENESIS_STREAM).next_u64();
        let size = in_range(
            size_draw,
            u64::from(config.demographics.household_min),
            u64::from(config.demographics.household_max),
        )
        .min(u64::from(remaining)) as u32;

        let family_draw = world.rng(GENESIS_STREAM).next_u64();
        let family_name = pick(&config.family.names, family_draw);

        let household_entity = world.spawn();
        let mut members: Vec<Entity> = Vec::with_capacity(size as usize);
        for _ in 0..size {
            let citizen = spawn_citizen(world, config, &family_name, ticks_per_year)?;
            world.insert(
                citizen,
                HouseholdMember {
                    household: household_entity,
                },
            )?;
            members.push(citizen);
        }
        members.sort_by_key(|e| e.index());
        world.insert(household_entity, Household { members })?;
        remaining -= size;
    }
    Ok(())
}

fn pick(list: &[String], draw: u64) -> String {
    // Validation guarantees non-empty lists; an empty list here would be a
    // data_defs bug, surfaced as an empty name rather than a panic.
    list.get((draw % list.len().max(1) as u64) as usize)
        .cloned()
        .unwrap_or_default()
}

fn spawn_citizen(
    world: &mut World,
    config: &PeopleConfig,
    family_name: &str,
    ticks_per_year: u64,
) -> Result<Entity, EcsError> {
    // Sex.
    let sex_draw = world.rng(GENESIS_STREAM).next_u64();
    let sex = if sex_draw % 1000 < u64::from(config.demographics.male_per_mille) {
        Sex::Male
    } else {
        Sex::Female
    };

    // Given name from the sex-matched list.
    let name_draw = world.rng(GENESIS_STREAM).next_u64();
    let given_name = match sex {
        Sex::Female => pick(&config.given_female.names, name_draw),
        Sex::Male => pick(&config.given_male.names, name_draw),
    };

    // Age: weighted band, then uniform within the band, then a uniform
    // day-offset within the year so birthdays spread across the calendar.
    let band_draw = world.rng(GENESIS_STREAM).next_u64();
    let band = pick_age_band(config, band_draw);
    let age_draw = world.rng(GENESIS_STREAM).next_u64();
    let age_years = in_range(age_draw, u64::from(band.0), u64::from(band.1));
    let offset_draw = world.rng(GENESIS_STREAM).next_u64();
    let birth_offset_ticks = offset_draw % ticks_per_year;
    let birth_tick = -((age_years * ticks_per_year + birth_offset_ticks) as i64);

    // Traits: uniform per-mille in each data range, data order.
    let mut weights = Vec::with_capacity(config.traits.traits.len());
    for def in &config.traits.traits {
        let draw = world.rng(GENESIS_STREAM).next_u64();
        weights.push(in_range(draw, def.min as u64, def.max as u64) as i16);
    }

    // Needs: uniform per-million in each data range, data order.
    let mut levels = Vec::with_capacity(config.needs.needs.len());
    for def in &config.needs.needs {
        let draw = world.rng(GENESIS_STREAM).next_u64();
        levels.push(NeedLevel::new_clamped(in_range(
            draw,
            def.initial_min as u64,
            def.initial_max as u64,
        ) as i64));
    }

    let citizen = world.spawn();
    world.insert(
        citizen,
        Identity {
            given_name,
            family_name: family_name.to_owned(),
            sex,
            birth_tick,
        },
    )?;
    world.insert(citizen, Needs { levels })?;
    world.insert(citizen, Personality { weights })?;
    Ok(citizen)
}

/// Weighted age-band choice: draw in [0, total_weight), walk bands in data
/// order. Returns (min_age, max_age).
fn pick_age_band(config: &PeopleConfig, draw: u64) -> (u32, u32) {
    let total: u64 = config
        .demographics
        .age_bands
        .iter()
        .map(|b| u64::from(b.weight_per_mille))
        .sum();
    let mut point = draw % total.max(1);
    for band in &config.demographics.age_bands {
        let weight = u64::from(band.weight_per_mille);
        if point < weight {
            return (band.min_age_years, band.max_age_years);
        }
        point -= weight;
    }
    // Validation guarantees weights sum to 1000 (> 0); unreachable, but
    // total coverage without panicking: fall back to the last band.
    config
        .demographics
        .age_bands
        .last()
        .map(|b| (b.min_age_years, b.max_age_years))
        .unwrap_or((0, 0))
}

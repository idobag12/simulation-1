//! Deterministic location genesis (ADR 0006 §2): public places from data
//! counts, one home per household. Orchestrated by the application, which
//! passes plain entity lists — `sim_world` never reads another sim
//! crate's components (SPEC §4).

use core_ecs::sim_interface::{Location, Ownership, Position, Residence, WorkingAge};
use core_ecs::{EcsError, Entity, World};

use crate::config::LocationsConfig;

/// Spawns every public (non-home) location: `count` instances per kind,
/// kind order then instance order (deterministic entity indices).
pub fn create_public_locations(
    world: &mut World,
    config: &LocationsConfig,
) -> Result<(), EcsError> {
    for (kind_index, kind) in config.kinds.iter().enumerate() {
        if kind.is_home {
            continue;
        }
        for _ in 0..kind.count {
            let entity = world.spawn();
            world.insert(
                entity,
                Location {
                    kind: kind_index as u32,
                },
            )?;
        }
    }
    Ok(())
}

/// Creates one home per household (member lists supplied by the
/// application, household order preserved), homes every member there, and
/// returns the home entities in household order.
///
/// Invariant: afterwards every listed citizen has `Position` and
/// `Residence` pointing at their household's home.
pub fn place_households(
    world: &mut World,
    config: &LocationsConfig,
    households: &[Vec<Entity>],
) -> Result<Vec<Entity>, EcsError> {
    let home_kind = config.home_kind().ok_or(EcsError::InternalCorruption(
        "locations config has no home kind (data_defs validation must reject this)",
    ))?;
    let mut homes = Vec::with_capacity(households.len());
    for members in households {
        let home = world.spawn();
        world.insert(home, Location { kind: home_kind })?;
        // Ownership (Phase 6, ADR 0009 §3): the household's first
        // working-age member owns the home; a household of minors falls
        // to its first member (deterministic — member lists are sorted).
        let mut owner = members.first().copied();
        for member in members {
            if world.get::<WorkingAge>(*member)?.is_some() {
                owner = Some(*member);
                break;
            }
        }
        if let Some(owner) = owner {
            world.insert(home, Ownership { owner })?;
        }
        for member in members {
            world.insert(*member, Position { at: home })?;
            world.insert(*member, Residence { home })?;
        }
        homes.push(home);
    }
    Ok(homes)
}

//! Deterministic location genesis (ADR 0006 §2): public places from data
//! counts, one home per household. Orchestrated by the application, which
//! passes plain entity lists — `sim_world` never reads another sim
//! crate's components (SPEC §4).

use core_ecs::sim_interface::{Location, Position, Residence};
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
        for member in members {
            world.insert(*member, Position { at: home })?;
            world.insert(*member, Residence { home })?;
        }
        homes.push(home);
    }
    Ok(homes)
}

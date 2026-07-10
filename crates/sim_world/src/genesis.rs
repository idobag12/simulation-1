//! Deterministic location genesis (ADR 0006 §2): public places from data
//! counts, one home per household. Orchestrated by the application, which
//! passes plain entity lists — `sim_world` never reads another sim
//! crate's components (SPEC §4).

use core_ecs::sim_interface::{Location, Ownership, Position, Residence, Sited, WorkingAge};
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

/// Sites every location on the map (Phase 9, ADR 0012 §1): round-robin
/// by entity order WITHIN each kind, so homes and venues spread across
/// every district and the travel MATRIX — not the placement rule — is
/// what differentiates locations. Zero districts = no map: no rows.
pub fn site_locations(world: &mut World, districts: u32) -> Result<(), EcsError> {
    if districts == 0 {
        return Ok(());
    }
    let located: Vec<(Entity, u32)> = world
        .iter::<Location>()?
        .map(|(entity, location)| (entity, location.kind))
        .collect();
    let mut next_per_kind: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    for (entity, kind) in located {
        let counter = next_per_kind.entry(kind).or_insert(0);
        world.insert(
            entity,
            Sited {
                district: *counter % districts,
            },
        )?;
        *counter += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::Seed;

    /// ADR 0012 §1: siting is round-robin BY KIND, so every kind spreads
    /// across every district and the matrix, not placement, is what
    /// differentiates locations. Zero districts sites nothing.
    #[test]
    fn siting_round_robins_within_each_kind() {
        let mut world = World::new(Seed::new(61), 32);
        world.register::<Location>().expect("register");
        world.register::<Sited>().expect("register");
        let mut spawned = Vec::new();
        for kind in [0u32, 0, 0, 1, 1] {
            let entity = world.spawn();
            world.insert(entity, Location { kind }).expect("insert");
            spawned.push(entity);
        }
        site_locations(&mut world, 2).expect("site");
        let district = |world: &World, entity: Entity| {
            world
                .get::<Sited>(entity)
                .expect("query")
                .map(|sited| sited.district)
        };
        assert_eq!(district(&world, spawned[0]), Some(0));
        assert_eq!(district(&world, spawned[1]), Some(1));
        assert_eq!(district(&world, spawned[2]), Some(0));
        assert_eq!(district(&world, spawned[3]), Some(0), "each kind restarts");
        assert_eq!(district(&world, spawned[4]), Some(1));

        let mut unmapped = World::new(Seed::new(62), 32);
        unmapped.register::<Location>().expect("register");
        unmapped.register::<Sited>().expect("register");
        let lone = unmapped.spawn();
        unmapped.insert(lone, Location { kind: 0 }).expect("insert");
        site_locations(&mut unmapped, 0).expect("site");
        assert_eq!(
            unmapped.get::<Sited>(lone).expect("query"),
            None,
            "no districts, no rows"
        );
    }
}

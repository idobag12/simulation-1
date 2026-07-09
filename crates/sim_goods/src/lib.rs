//! Goods (SPEC §12, §15 Phase 4; design in ADR 0007): the data-defined
//! good list and spoilage — the explicit sink that destroys perishable
//! stock and records every destroyed unit in the conservation ledger.
//!
//! Invariants owned by this crate:
//! - Spoilage is exact integer arithmetic: `floor(qty × per_mille / 1000)`
//!   daily, no remainder carried (ADR 0007 §5).
//! - Every destroyed unit is counted in `EconCounters::spoiled` in the
//!   same call that removes it — [`spoil_stock`] is the ONLY destruction
//!   path, used by the system and by seeded-shock interventions alike
//!   (ADR 0007 §8), so the goods audit identity can never drift.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod config;

use core_ecs::sim_interface::{EconCounters, Inventory};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};

pub use config::{GoodDef, GoodsConfig};

/// Destroys `quantity` units of `good` from `holder`'s inventory through
/// the modeled spoilage sink: stock and [`EconCounters::spoiled`] update
/// together (never one without the other). Destroys at most the held
/// stock; returns the quantity actually destroyed.
pub fn spoil_stock(
    world: &mut World,
    holder: Entity,
    good: u32,
    quantity: i64,
) -> Result<i64, EcsError> {
    if quantity <= 0 {
        return Ok(0);
    }
    let destroyed = match world.get_mut::<Inventory>(holder)? {
        Some(inventory) => match inventory.quantities.get_mut(good as usize) {
            Some(stock) => {
                let destroyed = quantity.min(*stock);
                *stock -= destroyed;
                destroyed
            }
            None => 0,
        },
        None => 0,
    };
    if destroyed > 0 {
        let counters = counters_entity(world)?.ok_or_else(|| {
            EcsError::InvariantViolation(
                "goods destroyed in a world without an EconCounters ledger".to_owned(),
            )
        })?;
        if let Some(counters) = world.get_mut::<EconCounters>(counters)?
            && let Some(spoiled) = counters.spoiled.get_mut(good as usize)
        {
            *spoiled += destroyed;
        }
    }
    Ok(destroyed)
}

/// The world's conservation-ledger entity, if the world has an economy
/// (worlds migrated from pre-economy saves honestly have none).
pub fn counters_entity(world: &World) -> Result<Option<Entity>, EcsError> {
    Ok(world
        .iter::<EconCounters>()?
        .next()
        .map(|(entity, _)| entity))
}

/// Day-rate system (ADR 0007 §5): every inventory loses
/// `floor(qty × spoil_per_mille / 1000)` units of each perishable good,
/// through the counted sink.
pub struct SpoilageSystem {
    /// Per good (data order): daily loss in per-mille of held stock.
    spoil_per_mille: Vec<i64>,
}

impl SpoilageSystem {
    /// Builds from the validated goods config (rate order = good order).
    pub fn new(spoil_per_mille: Vec<i64>) -> Self {
        SpoilageSystem { spoil_per_mille }
    }
}

impl System for SpoilageSystem {
    fn name(&self) -> &'static str {
        "goods.spoilage"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        // Pass 1 (immutable): compute every loss in entity order.
        let mut losses: Vec<(Entity, u32, i64)> = Vec::new();
        for (entity, inventory) in world.iter::<Inventory>()? {
            for (good, stock) in inventory.quantities.iter().enumerate() {
                let per_mille = self.spoil_per_mille.get(good).copied().unwrap_or(0);
                let loss = stock * per_mille / 1000;
                if loss > 0 {
                    losses.push((entity, good as u32, loss));
                }
            }
        }
        // Pass 2: destroy through the counted sink, same order.
        for (entity, good, loss) in losses {
            spoil_stock(world, entity, good, loss)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::Seed;

    fn world_with_ledger(goods: usize) -> (World, Entity) {
        let mut world = World::new(Seed::new(1), 16);
        world.register::<Inventory>().expect("register");
        world.register::<EconCounters>().expect("register");
        let ledger = world.spawn();
        world
            .insert(ledger, EconCounters::new(goods))
            .expect("insert");
        (world, ledger)
    }

    #[test]
    fn spoilage_is_exact_floor_arithmetic_and_counted() {
        let (mut world, ledger) = world_with_ledger(2);
        let firm = world.spawn();
        world
            .insert(
                firm,
                Inventory {
                    quantities: vec![999, 10],
                },
            )
            .expect("insert");

        // 150 per-mille of 999 = 149.85 → floor 149; of 10 = 1.5 → 1.
        let mut system = SpoilageSystem::new(vec![150, 150]);
        let ctx = core_ecs::TickContext {
            tick: core_types::Ticks::ZERO,
            time: core_types::CalendarTime::START,
        };
        let mut cmd = CommandBuffer::new();
        system.run(&mut world, &ctx, &mut cmd).expect("run");

        let inventory = world.get::<Inventory>(firm).expect("get").expect("some");
        assert_eq!(inventory.quantities, vec![850, 9]);
        let counters = world
            .get::<EconCounters>(ledger)
            .expect("get")
            .expect("some");
        assert_eq!(counters.spoiled, vec![149, 1]);
    }

    #[test]
    fn spoil_stock_is_bounded_by_held_stock() {
        let (mut world, ledger) = world_with_ledger(1);
        let firm = world.spawn();
        world
            .insert(
                firm,
                Inventory {
                    quantities: vec![5],
                },
            )
            .expect("insert");
        let destroyed = spoil_stock(&mut world, firm, 0, 100).expect("spoil");
        assert_eq!(destroyed, 5);
        assert_eq!(
            world
                .get::<Inventory>(firm)
                .expect("get")
                .expect("some")
                .quantities,
            vec![0]
        );
        assert_eq!(
            world
                .get::<EconCounters>(ledger)
                .expect("get")
                .expect("some")
                .spoiled,
            vec![5]
        );
        // Nothing left: destroying more is a counted no-op, not a negative.
        assert_eq!(spoil_stock(&mut world, firm, 0, 1).expect("spoil"), 0);
    }
}

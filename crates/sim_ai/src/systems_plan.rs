//! `PlanSystem` (split from `systems.rs` for the SPEC §3 module-size
//! rule).

use core_ecs::sim_interface::Personality;
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use core_types::calendar::MINUTES_PER_HOUR;

use crate::components::DailyPlan;
use crate::config::AiTables;

/// Minutes in a day, u16 (definitional; 24 × 60).
const DAY_MINUTES: u16 = (24 * MINUTES_PER_HOUR) as u16;

/// Hour-rate system: at the data-defined compile hour, every citizen
/// (re)compiles tomorrow's plan — the sleep window, shifted earlier by
/// their shift trait (early risers; ADR 0006 §5).
///
/// Invariant: pure integer arithmetic; same personality ⇒ same plan.
pub struct PlanSystem {
    tables: AiTables,
}

impl PlanSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: AiTables) -> Self {
        PlanSystem { tables }
    }

    fn plan_for(&self, personality: &Personality) -> DailyPlan {
        let trait_per_mille = personality
            .weights
            .get(self.tables.sleep_shift_trait as usize)
            .copied()
            .unwrap_or(0)
            .max(0) as u64;
        let shift =
            (trait_per_mille * u64::from(self.tables.sleep_max_shift_minutes) / 1000) as u16;
        let start = (self.tables.sleep_start_minute + DAY_MINUTES - shift) % DAY_MINUTES;
        let end = (self.tables.sleep_end_minute + DAY_MINUTES - shift) % DAY_MINUTES;
        DailyPlan {
            sleep_start_minute: start,
            sleep_end_minute: end,
        }
    }
}

impl System for PlanSystem {
    fn name(&self) -> &'static str {
        "ai.plan"
    }

    fn run(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        if ctx.time.hour != self.tables.plan_compile_hour {
            return Ok(());
        }
        // Pass 1 (immutable): compile plans in entity order. Tier C
        // citizens carry no plan — the day model IS their day (Phase 8,
        // ADR 0011 §2); Tier B executes the plan's sleep window.
        let mut plans: Vec<(Entity, DailyPlan)> = Vec::new();
        for (entity, personality) in world.iter::<Personality>()? {
            if matches!(
                world
                    .get::<core_ecs::sim_interface::LodTier>(entity)?
                    .map(|row| row.tier),
                Some(core_ecs::sim_interface::Tier::C)
            ) {
                continue;
            }
            plans.push((entity, self.plan_for(personality)));
        }
        // Pass 2: write.
        for (entity, plan) in plans {
            world.insert(entity, plan)?;
        }
        Ok(())
    }
}

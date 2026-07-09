//! The Phase 2 people systems: needs decay (hour rate) and mortality
//! (day rate). Explicit order and rates are wired by the application's
//! schedule builder (SPEC §6).

use core_ecs::{CommandBuffer, EcsError, System, TickContext, World};
use core_rng::RngCore;

use crate::components::{Household, HouseholdMember, Identity, Needs};
use crate::config::MortalityConfig;
use crate::events::{DeathCause, PersonDied};

/// RNG stream for mortality draws (one per citizen per day).
pub const MORTALITY_STREAM: &str = "people.mortality";

// Mortality draws are exact integer comparisons against a uniform draw in
// [0, 1e9) — the per-billion unit of the data-defined curve (ADR 0005 §4),
// an algorithmic constant, not a tunable.
const PER_BILLION: u64 = 1_000_000_000;

/// Hour-rate system: every citizen's needs decay by the data-defined
/// per-hour amounts, clamped at 0 (ADR 0005 §4). No satisfaction sources
/// exist until Phase 3 — "no AI yet" means people simply get hungrier.
///
/// Invariants: holds only immutable data-derived config; iterates in
/// entity-index order; exact integer arithmetic.
pub struct NeedsDecaySystem {
    decay_per_hour: Vec<i64>,
}

impl NeedsDecaySystem {
    /// Builds from the validated needs config (decay order = need order).
    pub fn new(decay_per_hour: Vec<i64>) -> Self {
        NeedsDecaySystem { decay_per_hour }
    }
}

impl System for NeedsDecaySystem {
    fn name(&self) -> &'static str {
        "people.needs_decay"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        for (_, needs) in world.iter_mut::<Needs>()? {
            for (level, decay) in needs.levels.iter_mut().zip(&self.decay_per_hour) {
                *level = level.saturating_sub(*decay).max(0);
            }
        }
        Ok(())
    }
}

/// Day-rate system: each citizen faces their age band's daily death
/// chance (deterministic curve from data + one draw from
/// [`MORTALITY_STREAM`] per citizen per day, ADR 0005 §4).
///
/// On death: emits [`PersonDied`], removes the citizen from their
/// household (despawning a household that empties), and despawns the
/// citizen — all structural changes through the command buffer (SPEC §6).
pub struct MortalitySystem {
    curve: MortalityConfig,
    ticks_per_year: u64,
}

impl MortalitySystem {
    /// Builds from the validated mortality curve and the calendar's year
    /// length in ticks.
    pub fn new(curve: MortalityConfig, ticks_per_year: u64) -> Self {
        MortalitySystem {
            curve,
            ticks_per_year,
        }
    }
}

impl System for MortalitySystem {
    fn name(&self) -> &'static str {
        "people.mortality"
    }

    fn run(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
        cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        // Pass 1 (immutable): collect ages in entity-index order.
        let candidates: Vec<(core_ecs::Entity, u32)> = world
            .iter::<Identity>()?
            .map(|(entity, identity)| {
                (
                    entity,
                    identity.age_years(ctx.tick.raw(), self.ticks_per_year),
                )
            })
            .collect();

        // Pass 2: one draw per citizen, same order (the draw count and
        // order are functions of deterministic state).
        for (entity, age_years) in candidates {
            let chance = u64::from(self.curve.per_day_chance(age_years));
            let draw = world.rng(MORTALITY_STREAM).next_u64() % PER_BILLION;
            if draw < chance {
                world.emit(&PersonDied {
                    person: entity,
                    cause: DeathCause::OldAge,
                })?;
                let member = world.get::<HouseholdMember>(entity)?.copied();
                if let Some(member) = member {
                    let household = member.household;
                    cmd.run(move |w| {
                        if let Some(h) = w.get_mut::<Household>(household)? {
                            h.members.retain(|m| *m != entity);
                            if h.members.is_empty() {
                                w.despawn(household)?;
                            }
                        }
                        Ok(())
                    });
                }
                cmd.despawn(entity);
            }
        }
        Ok(())
    }
}

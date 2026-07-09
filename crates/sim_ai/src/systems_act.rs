//! `ActSystem`: advances every `CurrentAction` with exact integer effects
//! (split from `systems.rs` for the SPEC §3 module-size rule).

use core_ecs::sim_interface::{Location, Needs, Position};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};

use crate::components::CurrentAction;
use crate::config::AiTables;

use super::NEED_MAX;

/// One entity's resolved transition for this tick (two-pass, SPEC §6).
enum Transition {
    Continue(CurrentAction),
    Arrive {
        at: Entity,
        next: CurrentAction,
    },
    GainAndContinue {
        need_index: u32,
        amount: i64,
        next: Option<CurrentAction>,
    },
    Finish,
}

/// Tick-rate system (after `DecideSystem`): advances every
/// `CurrentAction`. Travel counts down and sets `Position` on arrival;
/// performance applies the location's exact per-tick need gain and ends
/// when the need fills or the duration elapses; idling counts down.
pub struct ActSystem {
    tables: AiTables,
}

impl ActSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: AiTables) -> Self {
        ActSystem { tables }
    }

    fn transition(
        &self,
        world: &World,
        entity: Entity,
        action: CurrentAction,
    ) -> Result<Transition, EcsError> {
        Ok(match action {
            CurrentAction::Idle { remaining } => {
                if remaining > 1 {
                    Transition::Continue(CurrentAction::Idle {
                        remaining: remaining - 1,
                    })
                } else {
                    Transition::Finish
                }
            }
            CurrentAction::Travel {
                target,
                need_index,
                remaining,
            } => {
                if remaining > 1 {
                    Transition::Continue(CurrentAction::Travel {
                        target,
                        need_index,
                        remaining: remaining - 1,
                    })
                } else {
                    Transition::Arrive {
                        at: target,
                        next: CurrentAction::Perform {
                            at: target,
                            need_index,
                            remaining: self.tables.max_perform_ticks,
                        },
                    }
                }
            }
            CurrentAction::Perform {
                at,
                need_index,
                remaining,
            } => {
                let kind = world
                    .get::<Location>(at)?
                    .map(|location| location.kind)
                    .ok_or(EcsError::InternalCorruption(
                        "performing at an entity that is not a location",
                    ))?;
                let rate = self.tables.satisfier_rate(kind, need_index).unwrap_or(0);
                let level = world
                    .get::<Needs>(entity)?
                    .and_then(|needs| needs.levels.get(need_index as usize).copied())
                    .map(|level| level.raw())
                    .unwrap_or(0);
                let will_fill = level + rate >= NEED_MAX;
                let next = if will_fill || remaining <= 1 || rate <= 0 {
                    None
                } else {
                    Some(CurrentAction::Perform {
                        at,
                        need_index,
                        remaining: remaining - 1,
                    })
                };
                Transition::GainAndContinue {
                    need_index,
                    amount: rate,
                    next,
                }
            }
        })
    }
}

impl System for ActSystem {
    fn name(&self) -> &'static str {
        "ai.act"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        // Pass 1 (immutable): resolve every actor's transition, entity order.
        let mut transitions: Vec<(Entity, Transition)> = Vec::new();
        {
            let actions: Vec<(Entity, CurrentAction)> = world
                .iter::<CurrentAction>()?
                .map(|(entity, action)| (entity, *action))
                .collect();
            for (entity, action) in actions {
                transitions.push((entity, self.transition(world, entity, action)?));
            }
        }

        // Pass 2: apply, same order.
        for (entity, transition) in transitions {
            match transition {
                Transition::Continue(next) => {
                    world.insert(entity, next)?;
                }
                Transition::Arrive { at, next } => {
                    world.insert(entity, Position { at })?;
                    world.insert(entity, next)?;
                }
                Transition::GainAndContinue {
                    need_index,
                    amount,
                    next,
                } => {
                    if let Some(needs) = world.get_mut::<Needs>(entity)?
                        && let Some(level) = needs.levels.get_mut(need_index as usize)
                    {
                        *level = level.gain(amount);
                    }
                    match next {
                        Some(action) => {
                            world.insert(entity, action)?;
                        }
                        None => {
                            world.remove::<CurrentAction>(entity)?;
                        }
                    }
                }
                Transition::Finish => {
                    world.remove::<CurrentAction>(entity)?;
                }
            }
        }
        Ok(())
    }
}

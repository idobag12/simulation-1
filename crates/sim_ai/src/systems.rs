//! The Tier A decision loop (ADR 0006 §§3–5): `PlanSystem` (hour rate)
//! compiles sleep windows, `DecideSystem` (tick rate) scores and picks
//! actions, `ActSystem` (tick rate, after decide) advances them with
//! exact integer effects.

use core_ecs::sim_interface::{Location, NeedLevel, Needs, Personality, Position, Residence};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use core_types::Ticks;
use core_types::calendar::MINUTES_PER_HOUR;

use crate::components::{CandidateAction, CurrentAction, DailyPlan, LastDecision, ScoredCandidate};
use crate::config::AiTables;

// Scale constants. Unit definitions (per-million need scale, micro score
// scale), not tunables.
pub(crate) const NEED_MAX: i64 = NeedLevel::MAX.raw();
const MICRO: f64 = 1_000_000.0;
/// Minutes in a day, u16 (definitional; 24 × 60).
const DAY_MINUTES: u16 = (24 * MINUTES_PER_HOUR) as u16;

/// Everything scoring needs about one deciding citizen, snapshotted so
/// the scoring pass borrows nothing (two-pass pattern; SPEC §6 forbids
/// mid-iteration mutation).
struct Decider {
    entity: Entity,
    needs: Vec<i64>,
    traits: Vec<i16>,
    at: Option<Entity>,
    home: Option<Entity>,
    asleep_window: bool,
}

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
        // Pass 1 (immutable): compile plans in entity order.
        let plans: Vec<(Entity, DailyPlan)> = world
            .iter::<Personality>()?
            .map(|(entity, personality)| (entity, self.plan_for(personality)))
            .collect();
        // Pass 2: write.
        for (entity, plan) in plans {
            world.insert(entity, plan)?;
        }
        Ok(())
    }
}

/// Tick-rate system: every citizen without a `CurrentAction` enumerates
/// candidates, scores them (floats confined here, SPEC §11), and commits
/// to the argmax — recording the full scored list as `LastDecision`.
pub struct DecideSystem {
    tables: AiTables,
}

impl DecideSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: AiTables) -> Self {
        DecideSystem { tables }
    }

    /// Scores one candidate. Float discipline: `+ − × ÷` only (ADR 0006 §4).
    #[allow(
        clippy::too_many_arguments,
        reason = "pure scoring kernel; a params struct would only rename the locals"
    )]
    fn score(
        &self,
        level: i64,
        rate: i64,
        traveling: bool,
        traits: &[i16],
        need_index: u32,
        is_own_home_rest: bool,
        asleep_window: bool,
    ) -> f64 {
        let deficit = (NEED_MAX - level).max(0);
        // Recoverable amount is bounded by the longest performance.
        let recoverable =
            deficit.min(rate.saturating_mul(i64::from(self.tables.max_perform_ticks)));
        if recoverable <= 0 {
            return f64::MIN; // nothing to gain: never chosen over Idle (0.0)
        }
        let perform_ticks = recoverable.div_euclid(rate) + i64::from(recoverable % rate != 0);
        let travel_ticks = if traveling {
            i64::from(self.tables.travel_ticks)
        } else {
            0
        };

        let gain = recoverable as f64 / MICRO;
        let deficit_frac = deficit as f64 / MICRO;
        let mut urgency = 1.0f64;
        for _ in 0..self.tables.urgency_exponent {
            urgency *= deficit_frac;
        }
        let trait_factor = match self
            .tables
            .need_trait
            .get(need_index as usize)
            .copied()
            .flatten()
        {
            Some((trait_index, weight_per_mille)) => {
                let trait_value = traits
                    .get(trait_index as usize)
                    .copied()
                    .unwrap_or(0)
                    .max(0) as f64
                    / 1000.0;
                1.0 + (f64::from(weight_per_mille) / 1000.0) * trait_value
            }
            None => 1.0,
        };
        let time_cost = (travel_ticks + perform_ticks) as f64
            * self.tables.time_cost_micro_per_tick as f64
            / MICRO;
        let sleep_bias = if is_own_home_rest && asleep_window {
            self.tables.sleep_home_bias_micro as f64 / MICRO
        } else {
            0.0
        };

        gain * urgency * trait_factor - time_cost + sleep_bias
    }

    /// Enumerates and scores a decider's candidates; returns the dump and
    /// the chosen action. Enumeration order (= tie-break order, SPEC §11):
    /// own home (satisfier data order), public locations (entity order ×
    /// satisfier data order), Idle last.
    fn decide(
        &self,
        decider: &Decider,
        publics: &[(Entity, u32)],
        home_kind: Option<u32>,
    ) -> (LastDecision, CurrentAction) {
        let mut candidates: Vec<ScoredCandidate> = Vec::new();
        let mut scores: Vec<f64> = Vec::new();

        let mut push = |location: Entity, kind: u32, decider: &Decider, this: &Self| {
            let Some(kind_satisfiers) = this.tables.kind_satisfiers.get(kind as usize) else {
                return;
            };
            for (need_index, rate) in kind_satisfiers {
                let level = decider
                    .needs
                    .get(*need_index as usize)
                    .copied()
                    .unwrap_or(NEED_MAX);
                let traveling = decider.at != Some(location);
                let own_home_rest =
                    Some(location) == decider.home && *need_index == this.tables.rest_need;
                let score = this.score(
                    level,
                    *rate,
                    traveling,
                    &decider.traits,
                    *need_index,
                    own_home_rest,
                    decider.asleep_window,
                );
                candidates.push(ScoredCandidate {
                    action: CandidateAction::Satisfy {
                        location,
                        need_index: *need_index,
                    },
                    score_micro: quantize(score),
                });
                scores.push(score);
            }
        };

        if let (Some(home), Some(kind)) = (decider.home, home_kind) {
            push(home, kind, decider, self);
        }
        for (location, kind) in publics {
            push(*location, *kind, decider, self);
        }
        candidates.push(ScoredCandidate {
            action: CandidateAction::Idle,
            score_micro: 0,
        });
        scores.push(0.0);

        // Argmax with first-wins tie-breaking (enumeration order).
        let mut chosen = 0usize;
        for (index, score) in scores.iter().enumerate() {
            if *score > scores[chosen] {
                chosen = index;
            }
        }

        let action = match candidates[chosen].action {
            CandidateAction::Satisfy {
                location,
                need_index,
            } => {
                if decider.at == Some(location) {
                    CurrentAction::Perform {
                        at: location,
                        need_index,
                        remaining: self.tables.max_perform_ticks,
                    }
                } else {
                    CurrentAction::Travel {
                        target: location,
                        need_index,
                        remaining: self.tables.travel_ticks,
                    }
                }
            }
            CandidateAction::Idle => CurrentAction::Idle {
                remaining: self.tables.idle_ticks,
            },
        };
        (
            LastDecision {
                tick: Ticks::ZERO, // overwritten by caller with ctx.tick
                chosen: chosen as u32,
                candidates,
            },
            action,
        )
    }
}

/// Quantizes a score to micro units for storage (floats never persisted,
/// SPEC §2). `f64::round` and `as` saturation are both deterministic.
fn quantize(score: f64) -> i64 {
    if score == f64::MIN {
        i64::MIN
    } else {
        (score * MICRO).round() as i64
    }
}

impl System for DecideSystem {
    fn name(&self) -> &'static str {
        "ai.decide"
    }

    fn run(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let minute_of_day =
            u16::from(ctx.time.hour) * (MINUTES_PER_HOUR as u16) + u16::from(ctx.time.minute);
        let home_kind = self
            .tables
            .kind_is_home
            .iter()
            .position(|is_home| *is_home)
            .map(|index| index as u32);

        // Snapshot public locations (entity order).
        let publics: Vec<(Entity, u32)> = world
            .iter::<Location>()?
            .filter(|(_, location)| {
                !self
                    .tables
                    .kind_is_home
                    .get(location.kind as usize)
                    .copied()
                    .unwrap_or(false)
            })
            .map(|(entity, location)| (entity, location.kind))
            .collect();

        // Pass 1 (immutable): snapshot citizens that need a decision.
        let mut deciders: Vec<Decider> = Vec::new();
        for (entity, needs) in world.iter::<Needs>()? {
            if world.get::<CurrentAction>(entity)?.is_some() {
                continue;
            }
            let traits = world
                .get::<Personality>(entity)?
                .map(|p| p.weights.clone())
                .unwrap_or_default();
            let asleep_window = world
                .get::<DailyPlan>(entity)?
                .is_some_and(|plan| plan.in_sleep_window(minute_of_day));
            deciders.push(Decider {
                entity,
                needs: needs.levels.iter().map(|l| l.raw()).collect(),
                traits,
                at: world.get::<Position>(entity)?.map(|p| p.at),
                home: world.get::<Residence>(entity)?.map(|r| r.home),
                asleep_window,
            });
        }

        // Pass 2: decide and write (entity order preserved).
        for decider in deciders {
            let (mut dump, action) = self.decide(&decider, &publics, home_kind);
            dump.tick = ctx.tick;
            world.insert(decider.entity, action)?;
            world.insert(decider.entity, dump)?;
        }
        Ok(())
    }
}

#[path = "systems_act.rs"]
mod act;
pub use act::ActSystem;

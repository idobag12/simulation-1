//! The Phase 8 LOD systems (ADR 0011): the spotlight reader, the daily
//! tier assignment with lossless promotion/demotion, and the Tier B
//! (hourly blocks) and Tier C (daily `DayModel`) integrators. Money is
//! never modeled — wallets, jobs, and tenancies stay live at every
//! tier, which is what makes A↔C cycling conserve money exactly.

use core_ecs::sim_interface::{
    Born, DayModel, Employment, Fired, LoanDefaulted, LodTier, Married, Needs, Position, Residence,
    Spotlight, Tier, TierChanged,
};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use std::cmp::Reverse;

use core_types::calendar::{MINUTES_PER_HOUR, TICKS_PER_DAY};

use crate::components::{CurrentAction, DailyPlan, LastDecision};
use crate::config::AiTables;

/// Tick-rate system: pins citizens touched by a high-signal fact to the
/// embodied tier (ADR 0011 §1). The facts are emitted by day-rate
/// systems, so the readable buffer is empty on almost every tick and
/// this is O(readable events); the pin lives in a PERSISTED component —
/// a system-internal map would not survive save/load.
pub struct SpotlightSystem {
    highlight_days: u32,
}

impl SpotlightSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: &AiTables) -> Self {
        SpotlightSystem {
            highlight_days: tables.lod.highlight_days,
        }
    }
}

impl System for SpotlightSystem {
    fn name(&self) -> &'static str {
        "lod.spotlight"
    }

    fn run(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let mut newsworthy: Vec<Entity> = Vec::new();
        for event in world.events::<Fired>()? {
            newsworthy.push(event.citizen);
        }
        for event in world.events::<LoanDefaulted>()? {
            newsworthy.push(event.borrower);
        }
        for event in world.events::<Married>()? {
            newsworthy.push(event.partner_a);
            newsworthy.push(event.partner_b);
        }
        for event in world.events::<Born>()? {
            newsworthy.push(event.child);
            newsworthy.push(event.parent_a);
            newsworthy.push(event.parent_b);
        }
        if newsworthy.is_empty() {
            return Ok(());
        }
        let until_tick = ctx.tick.raw() + u64::from(self.highlight_days) * TICKS_PER_DAY;
        newsworthy.sort_by_key(|citizen| citizen.index());
        for citizen in newsworthy {
            // Citizens only (Needs is the citizen signal): firms borrow
            // and default too, and a firm must not carry citizen-LOD
            // state into saves and hashes.
            if world.is_alive(citizen) && world.get::<Needs>(citizen)?.is_some() {
                world.insert(citizen, Spotlight { until_tick })?;
            }
        }
        Ok(())
    }
}

/// Day-rate system, FIRST among the day systems (ADR 0011 §1): assigns
/// every citizen's tier deterministically in entity order — spotlighted
/// citizens first, then the stable front of the town, capped by data —
/// and applies the lossless promotion/demotion transitions (§4).
pub struct TierAssignSystem {
    tables: AiTables,
}

impl TierAssignSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: AiTables) -> Self {
        TierAssignSystem { tables }
    }
}

impl System for TierAssignSystem {
    fn name(&self) -> &'static str {
        "lod.tier_assign"
    }

    fn run(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        // Expire yesterday's spotlights.
        let expired: Vec<Entity> = world
            .iter::<Spotlight>()?
            .filter(|(_, pin)| pin.until_tick <= ctx.tick.raw())
            .map(|(citizen, _)| citizen)
            .collect();
        for citizen in expired {
            world.remove::<Spotlight>(citizen)?;
        }
        // Citizens in entity order (Needs is the citizen signal),
        // pinned first — both partitions keep entity order.
        let mut pinned: Vec<Entity> = Vec::new();
        let mut rest: Vec<Entity> = Vec::new();
        for (citizen, _) in world.iter::<Needs>()? {
            if world.get::<Spotlight>(citizen)?.is_some() {
                pinned.push(citizen);
            } else {
                rest.push(citizen);
            }
        }
        let a_cap = self.tables.lod.tier_a_cap as usize;
        let b_cap = self.tables.lod.tier_b_cap as usize;
        let assignments: Vec<(Entity, Tier)> = pinned
            .into_iter()
            .chain(rest)
            .enumerate()
            .map(|(rank, citizen)| {
                let tier = if rank < a_cap {
                    Tier::A
                } else if rank < a_cap + b_cap {
                    Tier::B
                } else {
                    Tier::C
                };
                (citizen, tier)
            })
            .collect();
        for (citizen, tier) in assignments {
            let current = world
                .get::<LodTier>(citizen)?
                .map(|row| row.tier)
                .unwrap_or(Tier::A);
            let stamped = world.get::<LodTier>(citizen)?.is_some();
            if stamped && current == tier {
                continue;
            }
            match (current, tier) {
                (Tier::A, Tier::B) | (Tier::A, Tier::C) => {
                    demote(world, citizen, &self.tables, tier)?;
                }
                (Tier::B, Tier::C) => {
                    world.remove::<DailyPlan>(citizen)?;
                    let model = build_day_model(world, citizen, &self.tables)?;
                    world.insert(citizen, model)?;
                    park_at_home(world, citizen)?;
                }
                (Tier::B, Tier::A) | (Tier::C, Tier::A) => {
                    promote(world, citizen)?;
                }
                (Tier::C, Tier::B) => {
                    world.remove::<DayModel>(citizen)?;
                    park_at_home(world, citizen)?;
                }
                // Same tier: only the first stamp reaches here.
                _ => {}
            }
            world.insert(citizen, LodTier { tier })?;
            if stamped {
                world.emit(&TierChanged {
                    citizen,
                    from: current,
                    to: tier,
                })?;
            }
        }
        Ok(())
    }
}

/// Demotes an embodied citizen (A→B/C, ADR 0011 §4): the embodied rows
/// go, the day model is derived from the data tables and their job, and
/// the citizen parks at home. Wallet, employment, tenancy untouched.
fn demote(world: &mut World, citizen: Entity, tables: &AiTables, to: Tier) -> Result<(), EcsError> {
    world.remove::<CurrentAction>(citizen)?;
    world.remove::<LastDecision>(citizen)?;
    // Only Tier C runs the day model (ADR 0011 §3, amended); Tier B
    // keeps its DailyPlan (it executes the plan's sleep window) and
    // carries no model — persisted state nothing reads would be dead
    // weight in every save and hash.
    if matches!(to, Tier::C) {
        world.remove::<DailyPlan>(citizen)?;
        let model = build_day_model(world, citizen, tables)?;
        world.insert(citizen, model)?;
    }
    park_at_home(world, citizen)?;
    Ok(())
}

/// Promotes a coarse citizen back to embodiment (ADR 0011 §4): the
/// model drops, the LIVE needs row (which the integrators kept) is the
/// materialized state, and the next tick's DecideSystem takes over —
/// the same code path a fresh citizen takes.
fn promote(world: &mut World, citizen: Entity) -> Result<(), EcsError> {
    world.remove::<DayModel>(citizen)?;
    park_at_home(world, citizen)?;
    Ok(())
}

pub(crate) fn park_at_home(world: &mut World, citizen: Entity) -> Result<(), EcsError> {
    match world.get::<Residence>(citizen)?.map(|r| r.home) {
        Some(home) => {
            world.insert(citizen, Position { at: home })?;
        }
        // A roofless coarse citizen is NOWHERE — leaving them standing
        // at their last venue would make an ever-present ghost the
        // social hour keeps meeting (and Tier C skips decay, so those
        // bonds would only grow).
        None => {
            world.remove::<Position>(citizen)?;
        }
    }
    Ok(())
}

/// Derives the statistical day model (ADR 0011 §3) from the data
/// satisfier tables and the citizen's job: home rates over the base
/// sleep window, the best public venue's rates over the data leisure
/// block, and the work-need gain over the shift when employed.
pub(crate) fn build_day_model(
    world: &World,
    citizen: Entity,
    tables: &AiTables,
) -> Result<DayModel, EcsError> {
    let need_count = tables.need_trait.len();
    let day_minutes = (core_types::calendar::HOURS_PER_DAY * MINUTES_PER_HOUR) as u16;
    let sleep_minutes = i64::from(
        (tables.sleep_end_minute + day_minutes - tables.sleep_start_minute) % day_minutes,
    );
    let leisure_minutes = i64::from(tables.lod.leisure_hours_per_day) * MINUTES_PER_HOUR as i64;
    // Home gains require a HOME (rough sleepers get nothing from the
    // sleep window — homelessness must not stop mattering at demotion).
    let home_kind = if world.get::<Residence>(citizen)?.is_some() {
        tables.kind_is_home.iter().position(|is_home| *is_home)
    } else {
        None
    };
    // ONE leisure venue for the block (ADR 0011 §3): the kind best
    // satisfying the citizen's most deficient need at derivation. A
    // per-need best-venue sum would credit the same minutes once per
    // need — a citizen present everywhere at once, over-satisfied days,
    // and structurally deflated Tier C retail demand.
    let most_deficient: Option<u32> = world.get::<Needs>(citizen)?.and_then(|needs| {
        needs
            .levels
            .iter()
            .enumerate()
            .min_by_key(|(index, level)| (level.raw(), *index))
            .map(|(index, _)| index as u32)
    });
    let leisure_kind: Option<u32> = most_deficient.and_then(|need| {
        (0..tables.kind_satisfiers.len() as u32)
            .filter(|kind| {
                !tables
                    .kind_is_home
                    .get(*kind as usize)
                    .copied()
                    .unwrap_or(false)
            })
            .filter(|kind| tables.satisfier_rate(*kind, need).is_some())
            .max_by_key(|kind| {
                (
                    tables.satisfier_rate(*kind, need).unwrap_or(0),
                    Reverse(*kind),
                )
            })
    });
    let mut passive_gain_per_day = Vec::with_capacity(need_count);
    for need in 0..need_count as u32 {
        let home_rate = home_kind
            .and_then(|kind| tables.satisfier_rate(kind as u32, need))
            .unwrap_or(0);
        let venue_rate = leisure_kind
            .and_then(|kind| tables.satisfier_rate(kind, need))
            .unwrap_or(0);
        let mut gain = home_rate * sleep_minutes + venue_rate * leisure_minutes;
        if need == tables.work_need && world.get::<Employment>(citizen)?.is_some() {
            gain += tables.work_need_per_tick
                * i64::from(tables.work_end_minute - tables.work_start_minute);
        }
        passive_gain_per_day.push(gain);
    }
    Ok(DayModel {
        passive_gain_per_day,
    })
}

#[path = "systems_lod_tiers.rs"]
mod tiers;
pub use tiers::{TierBSystem, TierCSystem};

/// Demotes EVERY citizen to Tier C through the normal demotion path —
/// the catch-up controller's entry (SPEC §5: catch-up is simply
/// "everyone runs Tier C"; ADR 0011 §5). The next normal day boundary's
/// assignment restores tiers.
pub fn demote_all_to_c(world: &mut World, tables: &AiTables) -> Result<(), EcsError> {
    let citizens: Vec<Entity> = world.iter::<Needs>()?.map(|(citizen, _)| citizen).collect();
    for citizen in citizens {
        let current = world
            .get::<LodTier>(citizen)?
            .map(|row| row.tier)
            .unwrap_or(Tier::A);
        match current {
            Tier::A => demote(world, citizen, tables, Tier::C)?,
            Tier::B => {
                world.remove::<DailyPlan>(citizen)?;
                let model = build_day_model(world, citizen, tables)?;
                world.insert(citizen, model)?;
                park_at_home(world, citizen)?;
            }
            Tier::C => continue,
        }
        world.insert(citizen, LodTier { tier: Tier::C })?;
        world.emit(&TierChanged {
            citizen,
            from: current,
            to: Tier::C,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_ecs::sim_interface::NeedLevel;
    use core_types::{CalendarTime, Seed, Ticks};

    pub(crate) fn test_tables() -> AiTables {
        AiTables {
            travel_ticks: 1,
            urgency_exponent: 2,
            time_cost_micro_per_tick: 0,
            max_perform_ticks: 10,
            idle_ticks: 1,
            plan_compile_hour: 0,
            sleep_start_minute: 1320, // 22:00 — the window crosses midnight
            sleep_end_minute: 360,    // 06:00 (8 hours)
            sleep_max_shift_minutes: 0,
            sleep_shift_trait: 0,
            rest_need: 0,
            sleep_home_bias_micro: 0,
            // Kind 0 = home (rest 2000/tick); kind 1 = venue (social
            // 5000/tick); two needs: rest (0), social (1).
            kind_is_home: vec![true, false],
            kind_satisfiers: vec![vec![(0, 2000)], vec![(1, 5000)]],
            need_trait: vec![None, None],
            mu_scale_micro: 0,
            half_wealth_mills: 1,
            work_start_minute: 480,
            work_end_minute: 960,
            work_bias_micro: 0,
            work_ticks: 1,
            work_need: 1,
            work_need_per_tick: 100,
            sales_tax_per_mille: 0,
            school_start_minute: 0,
            school_end_minute: 0,
            school_bias_micro: 0,
            school_attend_ticks: 1,
            school_location_kind: 1,
            school_taught_skill: 0,
            school_gain_per_mille: 0,
            social: crate::config::SocialTables {
                edge_cap: 8,
                friend_drift_per_meeting_per_mille: 30,
                romance_drift_per_meeting_per_mille: 25,
                decay_per_day_per_mille: 5,
                romance_min_sociability_product_per_mille: 90,
                marriage_threshold_per_mille: 700,
                social_bond_weight_per_mille: 400,
                belief_cap: 8,
                drift_need: 1,
                spark_trait: 0,
            },
            skill_count: 1,
            lod: crate::config::LodTables {
                tier_a_cap: 2,
                tier_b_cap: 1,
                highlight_days: 1,
                leisure_hours_per_day: 4,
            },
        }
    }

    fn world() -> World {
        let mut world = World::new(Seed::new(41), 64);
        world.register::<Needs>().expect("register");
        world.register::<Position>().expect("register");
        world.register::<Residence>().expect("register");
        world.register::<Employment>().expect("register");
        world.register::<LodTier>().expect("register");
        world.register::<DayModel>().expect("register");
        world.register::<Spotlight>().expect("register");
        world
            .register::<crate::components::CurrentAction>()
            .expect("register");
        world
            .register::<crate::components::DailyPlan>()
            .expect("register");
        world
            .register::<crate::components::LastDecision>()
            .expect("register");
        world.register_event::<TierChanged>().expect("register");
        world
    }

    fn citizen(world: &mut World) -> Entity {
        let entity = world.spawn();
        world
            .insert(
                entity,
                Needs {
                    levels: vec![NeedLevel::new_clamped(0); 2],
                },
            )
            .expect("insert");
        entity
    }

    fn ctx_at(tick: u64) -> TickContext {
        TickContext {
            tick: Ticks::new(tick),
            time: CalendarTime::START,
        }
    }

    /// ADR 0011 §3: the model derives from the roof, the job, and the
    /// data — a roofless citizen's sleep window yields NOTHING, a
    /// jobless one carries no work term, and the leisure block credits
    /// exactly ONE venue (the most deficient need's best).
    #[test]
    fn the_day_model_derives_from_roof_job_and_data() {
        let mut world = world();
        let housed = citizen(&mut world);
        let home = world.spawn();
        world.insert(housed, Residence { home }).expect("insert");
        let firm = world.spawn();
        world
            .insert(
                housed,
                Employment {
                    employer: firm,
                    wage_per_day: core_types::Money::from_mills(100),
                },
            )
            .expect("insert");
        // Both needs at 0: need 0 (rest) is the most deficient by
        // tie-break, so the leisure venue is chosen for it — kind 1
        // satisfies only social, and NO kind satisfies rest away from
        // home, so the leisure term lands on nothing for rest and
        // nothing for social (one venue, chosen for the deficient need).
        let model = build_day_model(&world, housed, &test_tables()).expect("model");
        // Sleep window = 8h = 480 minutes at rest 2000/tick.
        assert_eq!(model.passive_gain_per_day[0], 2000 * 480);
        // Work need (1): shift 480 minutes × 100/tick; no venue term
        // (the leisure venue was picked for need 0, which no venue
        // serves — the block credits ONE venue, never one per need).
        assert_eq!(model.passive_gain_per_day[1], 100 * 480);

        // Roofless: the sleep term vanishes with the roof.
        let roofless = citizen(&mut world);
        let model = build_day_model(&world, roofless, &test_tables()).expect("model");
        assert_eq!(model.passive_gain_per_day[0], 0);
        // …and with need 0 still deficient and unservable by venues,
        // the social need gains nothing passively either (jobless too).
        assert_eq!(model.passive_gain_per_day[1], 0);

        // A citizen whose deficient need IS venue-served gets the block.
        let social_case = citizen(&mut world);
        world
            .get_mut::<Needs>(social_case)
            .expect("query")
            .expect("needs")
            .levels[0] = NeedLevel::MAX;
        let model = build_day_model(&world, social_case, &test_tables()).expect("model");
        assert_eq!(
            model.passive_gain_per_day[1],
            5000 * i64::from(test_tables().lod.leisure_hours_per_day) * 60
        );
    }

    /// ADR 0011 §1: pins jump the queue; pins beyond the cap WAIT in
    /// entity order (no panic, no eviction of other pins); expiry lands
    /// exactly at `until_tick`.
    #[test]
    fn pins_jump_the_queue_and_expire_exactly() {
        let mut world = world();
        let citizens: Vec<Entity> = (0..5).map(|_| citizen(&mut world)).collect();
        // Pin the LAST three (entity order within pins preserved).
        for pinned in &citizens[2..5] {
            world
                .insert(*pinned, Spotlight { until_tick: 100 })
                .expect("insert");
        }
        let mut assign = TierAssignSystem::new(test_tables());
        assign
            .run(&mut world, &ctx_at(0), &mut CommandBuffer::new())
            .expect("assign");
        let tier = |world: &World, citizen: Entity| {
            world
                .get::<LodTier>(citizen)
                .expect("query")
                .map(|row| row.tier)
        };
        // Caps 2/1: pins fill A first (citizens 2, 3), the third pin
        // (4) WAITS at the front of B, the unpinned front (0) takes the
        // B remainder... and 1 is C.
        assert_eq!(tier(&world, citizens[2]), Some(Tier::A));
        assert_eq!(tier(&world, citizens[3]), Some(Tier::A));
        assert_eq!(tier(&world, citizens[4]), Some(Tier::B));
        assert_eq!(tier(&world, citizens[0]), Some(Tier::C));
        assert_eq!(tier(&world, citizens[1]), Some(Tier::C));

        // Expiry is exact: at tick 99 the pins hold; at 100 they drop
        // and the stable front (entity order) takes over.
        assign
            .run(&mut world, &ctx_at(99), &mut CommandBuffer::new())
            .expect("assign");
        assert_eq!(tier(&world, citizens[2]), Some(Tier::A));
        assign
            .run(&mut world, &ctx_at(100), &mut CommandBuffer::new())
            .expect("assign");
        assert_eq!(
            world.get::<Spotlight>(citizens[2]).expect("query"),
            None,
            "the pin dropped exactly at its tick"
        );
        assert_eq!(tier(&world, citizens[0]), Some(Tier::A));
        assert_eq!(tier(&world, citizens[1]), Some(Tier::A));
        assert_eq!(tier(&world, citizens[2]), Some(Tier::B));
        assert_eq!(tier(&world, citizens[3]), Some(Tier::C));
    }

    /// ADR 0011 §4: demotion to C strips the embodied rows and writes
    /// the model; demotion to B keeps the plan and carries NO model;
    /// promotion drops the model and touches nothing else.
    #[test]
    fn transitions_carry_exactly_the_documented_state() {
        let mut world = world();
        let subject = citizen(&mut world);
        let home = world.spawn();
        world.insert(subject, Residence { home }).expect("insert");
        world
            .insert(
                subject,
                crate::components::DailyPlan {
                    sleep_start_minute: 0,
                    sleep_end_minute: 1,
                },
            )
            .expect("insert");
        world
            .insert(
                subject,
                crate::components::CurrentAction::Idle { remaining: 3 },
            )
            .expect("insert");
        demote(&mut world, subject, &test_tables(), Tier::B).expect("demote");
        assert!(
            world
                .get::<crate::components::CurrentAction>(subject)
                .expect("query")
                .is_none()
        );
        assert!(
            world
                .get::<crate::components::DailyPlan>(subject)
                .expect("query")
                .is_some(),
            "Tier B executes the plan's sleep window — the plan stays"
        );
        assert!(
            world.get::<DayModel>(subject).expect("query").is_none(),
            "only Tier C carries the model"
        );
        demote(&mut world, subject, &test_tables(), Tier::C).expect("demote");
        assert!(
            world
                .get::<crate::components::DailyPlan>(subject)
                .expect("query")
                .is_none()
        );
        assert!(world.get::<DayModel>(subject).expect("query").is_some());
        promote(&mut world, subject).expect("promote");
        assert!(world.get::<DayModel>(subject).expect("query").is_none());
        assert_eq!(
            world.get::<Position>(subject).expect("query").map(|p| p.at),
            Some(home),
            "promotion materializes the citizen at home"
        );
    }
}

//! The Phase 8 LOD systems (ADR 0011): the spotlight reader, the daily
//! tier assignment with lossless promotion/demotion, and the Tier B
//! (hourly blocks) and Tier C (daily `DayModel`) integrators. Money is
//! never modeled — wallets, jobs, and tenancies stay live at every
//! tier, which is what makes A↔C cycling conserve money exactly.

use core_ecs::sim_interface::{
    Beliefs, Born, DayModel, Employment, Fired, LoanDefaulted, Location, LodTier, Married, Needs,
    Position, Residence, RetailOffer, SchoolAge, SchoolAttended, Skills, Spotlight, Tier,
    TierChanged,
};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
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
            if world.is_alive(citizen) {
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
    if matches!(to, Tier::C) {
        world.remove::<DailyPlan>(citizen)?;
    }
    let model = build_day_model(world, citizen, tables)?;
    world.insert(citizen, model)?;
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

fn park_at_home(world: &mut World, citizen: Entity) -> Result<(), EcsError> {
    if let Some(home) = world.get::<Residence>(citizen)?.map(|r| r.home) {
        world.insert(citizen, Position { at: home })?;
    }
    Ok(())
}

/// Derives the statistical day model (ADR 0011 §3) from the data
/// satisfier tables and the citizen's job: home rates over the base
/// sleep window, the best public venue's rates over the data leisure
/// block, and the work-need gain over the shift when employed.
fn build_day_model(
    world: &World,
    citizen: Entity,
    tables: &AiTables,
) -> Result<DayModel, EcsError> {
    let need_count = tables.need_trait.len();
    let day_minutes = (24 * MINUTES_PER_HOUR) as u16;
    let sleep_minutes = i64::from(
        (tables.sleep_end_minute + day_minutes - tables.sleep_start_minute) % day_minutes,
    );
    let leisure_minutes = i64::from(tables.lod.leisure_hours_per_day) * MINUTES_PER_HOUR as i64;
    let home_kind = tables.kind_is_home.iter().position(|is_home| *is_home);
    let mut passive_gain_per_day = Vec::with_capacity(need_count);
    for need in 0..need_count as u32 {
        let home_rate = home_kind
            .and_then(|kind| tables.satisfier_rate(kind as u32, need))
            .unwrap_or(0);
        let venue_rate = (0..tables.kind_satisfiers.len() as u32)
            .filter(|kind| {
                !tables
                    .kind_is_home
                    .get(*kind as usize)
                    .copied()
                    .unwrap_or(false)
            })
            .filter_map(|kind| tables.satisfier_rate(kind, need))
            .max()
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

/// A retail snapshot shared by the coarse integrators: per offer, the
/// seller and the need it satisfies (entity order — deterministic).
fn offer_snapshot(world: &World) -> Result<Vec<(Entity, RetailOffer)>, EcsError> {
    let mut offers = Vec::new();
    for (seller, offer) in world.iter::<RetailOffer>()? {
        offers.push((seller, *offer));
    }
    Ok(offers)
}

/// The cheapest KNOWN seller for `need` — believed price when the buyer
/// has one, posted price otherwise (the same knowledge Tier A's scoring
/// uses; ADR 0011 §2). Ties break on entity order.
fn cheapest_for_need(
    world: &World,
    buyer: Entity,
    need: u32,
    offers: &[(Entity, RetailOffer)],
) -> Result<Option<(Entity, RetailOffer)>, EcsError> {
    let beliefs = world.get::<Beliefs>(buyer)?;
    let mut best: Option<(i64, Entity, RetailOffer)> = None;
    for (seller, offer) in offers {
        if offer.need_index != need || offer.gain_per_unit <= 0 {
            continue;
        }
        let believed = beliefs
            .and_then(|beliefs| {
                beliefs
                    .prices
                    .iter()
                    .find(|(shop, _)| shop == seller)
                    .map(|(_, price)| price.mills())
            })
            .unwrap_or(offer.unit_price.mills());
        if best.is_none_or(|(price, _, _)| believed < price) {
            best = Some((believed, *seller, *offer));
        }
    }
    Ok(best.map(|(_, seller, offer)| (seller, offer)))
}

/// Applies the school day to a coarse pupil: the taught skill rises by
/// the data gain and the fact lands — the same growth an embodied
/// attendance produces (ADR 0011 §2).
fn attend_school_abstractly(
    world: &mut World,
    pupil: Entity,
    tables: &AiTables,
) -> Result<(), EcsError> {
    let mut skills = world
        .get::<Skills>(pupil)?
        .cloned()
        .unwrap_or(Skills { levels: Vec::new() });
    if skills.levels.len() < tables.skill_count as usize {
        skills.levels.resize(tables.skill_count as usize, 0);
    }
    let Some(level) = skills
        .levels
        .get_mut(tables.school_taught_skill as usize)
        .copied()
        .map(|level| level.saturating_add(tables.school_gain_per_mille).min(1000))
    else {
        return Ok(());
    };
    skills.levels[tables.school_taught_skill as usize] = level;
    world.insert(pupil, skills)?;
    world.emit(&SchoolAttended {
        pupil,
        skill: tables.school_taught_skill,
        new_level: level,
    })?;
    Ok(())
}

/// Hour-rate system (ADR 0011 §2): executes each Tier B citizen's day
/// as blocks from the SAME obligations the embodied biases read — the
/// sleep window at home, the shift at the workplace, school in its
/// hours, else leisure at the venue best satisfying the most deficient
/// need. Needs integrate analytically (rate × 60, exact integers), one
/// real purchase may resolve per hour, and Position genuinely moves —
/// Tier B citizens keep appearing in the Phase 7 social hour.
pub struct TierBSystem {
    tables: AiTables,
}

impl TierBSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: AiTables) -> Self {
        TierBSystem { tables }
    }
}

impl System for TierBSystem {
    fn name(&self) -> &'static str {
        "lod.tier_b"
    }

    fn run(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let minute_of_day =
            u16::from(ctx.time.hour) * (MINUTES_PER_HOUR as u16) + u16::from(ctx.time.minute);
        let citizens: Vec<Entity> = world
            .iter::<LodTier>()?
            .filter(|(_, row)| matches!(row.tier, Tier::B))
            .map(|(citizen, _)| citizen)
            .collect();
        if citizens.is_empty() {
            return Ok(());
        }
        // Venue-by-kind snapshot (first location of each kind) and the
        // retail offers, both in entity order.
        let mut venue_of_kind: Vec<Option<Entity>> = vec![None; self.tables.kind_satisfiers.len()];
        for (venue, location) in world.iter::<Location>()? {
            if let Some(slot) = venue_of_kind.get_mut(location.kind as usize)
                && slot.is_none()
            {
                *slot = Some(venue);
            }
        }
        let school = venue_of_kind
            .get(self.tables.school_location_kind as usize)
            .copied()
            .flatten();
        let offers = offer_snapshot(world)?;
        let hour_gain = |rate: i64| rate * MINUTES_PER_HOUR as i64;

        for citizen in citizens {
            // No compiled plan yet (freshly demoted): the base data
            // window, with the same midnight-crossing semantics.
            let asleep = world
                .get::<DailyPlan>(citizen)?
                .map(|plan| plan.in_sleep_window(minute_of_day))
                .unwrap_or_else(|| {
                    DailyPlan {
                        sleep_start_minute: self.tables.sleep_start_minute,
                        sleep_end_minute: self.tables.sleep_end_minute,
                    }
                    .in_sleep_window(minute_of_day)
                });
            let workplace = world
                .get::<Employment>(citizen)?
                .map(|employment| employment.employer);
            let in_work_window = minute_of_day >= self.tables.work_start_minute
                && minute_of_day < self.tables.work_end_minute;
            let in_school_window = minute_of_day >= self.tables.school_start_minute
                && minute_of_day < self.tables.school_end_minute;
            let school_age = world.get::<SchoolAge>(citizen)?.is_some();

            if asleep {
                // The night block: home, home rates.
                park_at_home(world, citizen)?;
                let home_kind = self.tables.kind_is_home.iter().position(|h| *h);
                if let Some(kind) = home_kind {
                    apply_kind_gains(world, citizen, &self.tables, kind as u32, hour_gain)?;
                }
                continue;
            }
            if let (Some(workplace), true) = (workplace, in_work_window) {
                world.insert(citizen, Position { at: workplace })?;
                let gain = hour_gain(self.tables.work_need_per_tick);
                bump_need(world, citizen, self.tables.work_need, gain)?;
                continue;
            }
            if school_age && in_school_window {
                if let Some(school) = school {
                    world.insert(citizen, Position { at: school })?;
                    if minute_of_day == self.tables.school_start_minute {
                        attend_school_abstractly(world, citizen, &self.tables)?;
                    }
                }
                continue;
            }
            // The leisure block: the venue best satisfying the most
            // deficient need; one real purchase may resolve first.
            let needs: Vec<i64> = world
                .get::<Needs>(citizen)?
                .map(|needs| needs.levels.iter().map(|level| level.raw()).collect())
                .unwrap_or_default();
            let Some((need, _)) = needs
                .iter()
                .enumerate()
                .min_by_key(|(index, level)| (**level, *index))
            else {
                continue;
            };
            let need = need as u32;
            if let Some((seller, offer)) = cheapest_for_need(world, citizen, need, &offers)?
                && core_ecs::sim_interface::NeedLevel::MAX.raw() - needs[need as usize]
                    >= offer.gain_per_unit
            {
                crate::systems::purchase_unit(
                    world,
                    citizen,
                    seller,
                    self.tables.sales_tax_per_mille,
                    self.tables.social.belief_cap as usize,
                )?;
                continue;
            }
            let best_kind = (0..self.tables.kind_satisfiers.len() as u32)
                .filter(|kind| {
                    !self
                        .tables
                        .kind_is_home
                        .get(*kind as usize)
                        .copied()
                        .unwrap_or(false)
                })
                .filter(|kind| self.tables.satisfier_rate(*kind, need).is_some())
                .max_by_key(|kind| self.tables.satisfier_rate(*kind, need).unwrap_or(0));
            if let Some(kind) = best_kind
                && let Some(venue) = venue_of_kind.get(kind as usize).copied().flatten()
            {
                world.insert(citizen, Position { at: venue })?;
                apply_kind_gains(world, citizen, &self.tables, kind, hour_gain)?;
            } else {
                park_at_home(world, citizen)?;
            }
        }
        Ok(())
    }
}

/// Applies every satisfier of `kind` to the citizen for one block.
fn apply_kind_gains(
    world: &mut World,
    citizen: Entity,
    tables: &AiTables,
    kind: u32,
    block: impl Fn(i64) -> i64,
) -> Result<(), EcsError> {
    let gains: Vec<(u32, i64)> = tables
        .kind_satisfiers
        .get(kind as usize)
        .map(|satisfiers| {
            satisfiers
                .iter()
                .map(|(need, rate)| (*need, block(*rate)))
                .collect()
        })
        .unwrap_or_default();
    for (need, gain) in gains {
        bump_need(world, citizen, need, gain)?;
    }
    Ok(())
}

fn bump_need(world: &mut World, citizen: Entity, need: u32, gain: i64) -> Result<(), EcsError> {
    if let Some(needs) = world.get_mut::<Needs>(citizen)?
        && let Some(level) = needs.levels.get_mut(need as usize)
    {
        *level = level.gain(gain);
    }
    Ok(())
}

/// Day-rate system (ADR 0011 §§2–3): executes each Tier C citizen's
/// `DayModel` once per day — real purchases for the deficits retail can
/// satisfy (cheapest known shop, unit by unit, through the shared
/// till), the model's passive gains for the rest, the school day for
/// school-age citizens, and the abstract day ends at home. Employment,
/// payroll, rent, and the lifecycle run unchanged around it.
pub struct TierCSystem {
    tables: AiTables,
}

impl TierCSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: AiTables) -> Self {
        TierCSystem { tables }
    }
}

impl System for TierCSystem {
    fn name(&self) -> &'static str {
        "lod.tier_c"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let citizens: Vec<Entity> = world
            .iter::<LodTier>()?
            .filter(|(_, row)| matches!(row.tier, Tier::C))
            .map(|(citizen, _)| citizen)
            .collect();
        if citizens.is_empty() {
            return Ok(());
        }
        let offers = offer_snapshot(world)?;
        let school_exists = world
            .iter::<Location>()?
            .any(|(_, location)| location.kind == self.tables.school_location_kind);
        for citizen in citizens {
            // The model refreshes daily (jobs change; ADR 0011 §3).
            let model = build_day_model(world, citizen, &self.tables)?;
            let need_count = model.passive_gain_per_day.len();
            // Purchases first: retail covers what presence cannot.
            for need in 0..need_count as u32 {
                loop {
                    let level = world
                        .get::<Needs>(citizen)?
                        .and_then(|needs| needs.levels.get(need as usize).copied())
                        .map(|level| level.raw())
                        .unwrap_or(core_ecs::sim_interface::NeedLevel::MAX.raw());
                    let deficit = core_ecs::sim_interface::NeedLevel::MAX.raw() - level;
                    let Some((seller, offer)) = cheapest_for_need(world, citizen, need, &offers)?
                    else {
                        break;
                    };
                    if deficit < offer.gain_per_unit {
                        break;
                    }
                    let bought = crate::systems::purchase_unit(
                        world,
                        citizen,
                        seller,
                        self.tables.sales_tax_per_mille,
                        self.tables.social.belief_cap as usize,
                    )?;
                    if bought.is_none() {
                        break;
                    }
                }
            }
            // Passive presence: the model's analytic day.
            for (need, gain) in model.passive_gain_per_day.iter().enumerate() {
                bump_need(world, citizen, need as u32, *gain)?;
            }
            world.insert(citizen, model)?;
            if school_exists && world.get::<SchoolAge>(citizen)?.is_some() {
                attend_school_abstractly(world, citizen, &self.tables)?;
            }
            park_at_home(world, citizen)?;
        }
        Ok(())
    }
}

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

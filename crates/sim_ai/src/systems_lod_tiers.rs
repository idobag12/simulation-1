//! The Phase 8 coarse integrators (ADR 0011 §2): Tier B's hourly
//! obligation blocks and Tier C's daily `DayModel` execution. Split
//! from `systems_lod.rs` for the SPEC §3 module-size rule.

use core_ecs::sim_interface::{
    Beliefs, Employment, Location, LodTier, Needs, Position, RetailOffer, SchoolAge,
    SchoolAttended, Skills, Tier,
};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use core_types::calendar::MINUTES_PER_HOUR;

use crate::components::DailyPlan;
use crate::config::AiTables;

use super::{build_day_model, park_at_home};

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
    tables: &AiTables,
    buyer: Entity,
    need: u32,
    offers: &[(Entity, RetailOffer)],
) -> Result<Option<(Entity, RetailOffer)>, EcsError> {
    let beliefs = world.get::<Beliefs>(buyer)?;
    // The buyer's district (their current abstract position): coarse
    // shop choice weighs distance like embodied scoring does, at the
    // data money-equivalent (Phase 9, ADR 0012 §3).
    let from = match world.get::<Position>(buyer)?.map(|position| position.at) {
        Some(at) => world
            .get::<core_ecs::sim_interface::Sited>(at)?
            .map(|sited| sited.district),
        None => None,
    };
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
        let to = world
            .get::<core_ecs::sim_interface::Sited>(*seller)?
            .map(|sited| sited.district);
        let effective =
            believed + tables.commute_mills_per_tick * i64::from(tables.travel_between(from, to));
        if best.is_none_or(|(price, _, _)| effective < price) {
            best = Some((effective, *seller, *offer));
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
        // Statistical labor is REAL labor (SPEC §10 "labor supplied"):
        // production gates batch starts on PRESENT workers, so employed
        // Tier C citizens stand at their workplace through the shift —
        // two Position writes a day, at the shift's ends.
        if minute_of_day == self.tables.work_start_minute
            || minute_of_day == self.tables.work_end_minute
        {
            let statistical: Vec<Entity> = world
                .iter::<LodTier>()?
                .filter(|(_, row)| matches!(row.tier, Tier::C))
                .map(|(citizen, _)| citizen)
                .collect();
            for citizen in statistical {
                match world.get::<Employment>(citizen)? {
                    Some(employment) if minute_of_day == self.tables.work_start_minute => {
                        let at = employment.employer;
                        world.insert(citizen, Position { at })?;
                    }
                    Some(_) => park_at_home(world, citizen)?,
                    None => {}
                }
            }
        }
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
                // The night block: home rates — under a real roof only
                // (an evictee sleeping rough gains nothing; demotion
                // must not repeal homelessness).
                park_at_home(world, citizen)?;
                if world
                    .get::<core_ecs::sim_interface::Residence>(citizen)?
                    .is_some()
                    && let Some(kind) = self.tables.kind_is_home.iter().position(|h| *h)
                {
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
            // The purchase candidate is the most deficient need RETAIL
            // can serve (the venue need may have no shop — a hungry
            // citizen still buys bread even when loneliness is deeper).
            let purchase_need = needs
                .iter()
                .enumerate()
                .filter(|(candidate, level)| {
                    offers
                        .iter()
                        .any(|(_, offer)| offer.need_index == *candidate as u32)
                        && **level < core_ecs::sim_interface::NeedLevel::MAX.raw()
                })
                .min_by_key(|(candidate, level)| (**level, *candidate))
                .map(|(candidate, _)| candidate as u32);
            if let Some(purchase_need) = purchase_need
                && let Some((seller, offer)) =
                    cheapest_for_need(world, &self.tables, citizen, purchase_need, &offers)?
                // Round-to-nearest coverage, like the embodied buyer
                // (whose clamped gain happily wastes the tail of a
                // unit): buy when at least half the unit lands.
                && (core_ecs::sim_interface::NeedLevel::MAX.raw()
                    - needs[purchase_need as usize])
                    * 2
                    >= offer.gain_per_unit
                && crate::systems::purchase_unit(
                    world,
                    citizen,
                    seller,
                    self.tables.sales_tax_per_mille,
                    self.tables.social.belief_cap as usize,
                )?
                .is_some()
            {
                // The buyer genuinely stands at the till this hour
                // (ADR 0011 §2, amended: a purchase hour replaces the
                // venue visit). An empty shelf or short wallet falls
                // through to the venue instead — the hour is not wasted.
                world.insert(citizen, Position { at: seller })?;
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
                // The hour includes GETTING there (Phase 9): the trip's
                // ticks come off the block, exactly the minutes an
                // embodied citizen loses to the same walk.
                let from = match world.get::<Position>(citizen)?.map(|position| position.at) {
                    Some(at) => world
                        .get::<core_ecs::sim_interface::Sited>(at)?
                        .map(|sited| sited.district),
                    None => None,
                };
                let to = world
                    .get::<core_ecs::sim_interface::Sited>(venue)?
                    .map(|sited| sited.district);
                let trip =
                    i64::from(self.tables.travel_between(from, to)).min(MINUTES_PER_HOUR as i64);
                world.insert(citizen, Position { at: venue })?;
                apply_kind_gains(world, citizen, &self.tables, kind, |rate| {
                    rate * (MINUTES_PER_HOUR as i64 - trip)
                })?;
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
    /// Catch-up mode (ADR 0011 §5): ALSO integrate citizens with no
    /// tier row — a citizen born during the caught-up span has never
    /// met the (omitted) assignment, and "everyone runs Tier C" must
    /// include them, or nothing feeds a mid-span newborn.
    include_unassigned: bool,
}

impl TierCSystem {
    /// Builds from the resolved tables (the normal schedule: stamped
    /// Tier C rows only — a missing row means Tier A).
    pub fn new(tables: AiTables) -> Self {
        TierCSystem {
            tables,
            include_unassigned: false,
        }
    }

    /// The catch-up schedule's variant: unassigned citizens (newborns
    /// of the span) integrate as Tier C too.
    pub fn for_catchup(tables: AiTables) -> Self {
        TierCSystem {
            tables,
            include_unassigned: true,
        }
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
        let mut citizens: Vec<Entity> = Vec::new();
        for (citizen, _) in world.iter::<Needs>()? {
            match world.get::<LodTier>(citizen)?.map(|row| row.tier) {
                Some(Tier::C) => citizens.push(citizen),
                None if self.include_unassigned => citizens.push(citizen),
                _ => {}
            }
        }
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
                    let Some((seller, offer)) =
                        cheapest_for_need(world, &self.tables, citizen, need, &offers)?
                    else {
                        break;
                    };
                    // Round-to-nearest coverage, like the embodied
                    // buyer whose clamped gain wastes the unit's tail:
                    // the last unit is bought when at least half lands.
                    if deficit * 2 < offer.gain_per_unit {
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

#[cfg(test)]
mod tests {
    use super::*;
    use core_ecs::sim_interface::{
        EconCounters, FirmBooks, GoodsPurchased, Inventory, NeedLevel, Wallet,
    };
    use core_types::{CalendarTime, Money, Seed, Ticks};

    /// ADR 0011 §2: the Tier C purchase loop stops at the half-unit
    /// gate, at an empty shelf, and at a short wallet — and every unit
    /// it does buy moves real money and real stock.
    #[test]
    fn tier_c_purchases_stop_at_the_documented_gates() {
        let mut world = core_ecs::World::new(Seed::new(43), 64);
        world.register::<Needs>().expect("register");
        world.register::<Position>().expect("register");
        world
            .register::<core_ecs::sim_interface::Residence>()
            .expect("register");
        world.register::<Employment>().expect("register");
        world.register::<LodTier>().expect("register");
        world
            .register::<core_ecs::sim_interface::DayModel>()
            .expect("register");
        world.register::<Wallet>().expect("register");
        world.register::<Inventory>().expect("register");
        world.register::<RetailOffer>().expect("register");
        world.register::<FirmBooks>().expect("register");
        world.register::<EconCounters>().expect("register");
        world.register::<Beliefs>().expect("register");
        world.register::<Location>().expect("register");
        world.register::<SchoolAge>().expect("register");
        world.register::<Skills>().expect("register");
        world
            .register::<core_ecs::sim_interface::TreasuryBook>()
            .expect("register");
        world
            .register::<core_ecs::sim_interface::Sited>()
            .expect("register");
        world.register_event::<GoodsPurchased>().expect("register");

        let ledger = world.spawn();
        world
            .insert(
                ledger,
                EconCounters {
                    issued: Money::from_mills(1_000),
                    produced: vec![0],
                    consumed_by_citizens: vec![0],
                    consumed_in_production: vec![0],
                    spoiled: vec![0],
                },
            )
            .expect("insert");
        let shop = world.spawn();
        world
            .insert(
                shop,
                RetailOffer {
                    good: 0,
                    need_index: 0,
                    gain_per_unit: 400_000,
                    use_ticks: 1,
                    unit_price: Money::from_mills(100),
                },
            )
            .expect("insert");
        world
            .insert(
                shop,
                Inventory {
                    quantities: vec![2],
                },
            )
            .expect("insert");
        world
            .insert(shop, Wallet { cash: Money::ZERO })
            .expect("insert");
        world
            .insert(
                shop,
                FirmBooks {
                    initial_cash: Money::ZERO,
                    revenue: Money::ZERO,
                    expenses: Money::ZERO,
                },
            )
            .expect("insert");

        let spawn_c = |world: &mut core_ecs::World, level: i64, cash: i64| {
            let citizen = world.spawn();
            world
                .insert(
                    citizen,
                    Needs {
                        levels: vec![NeedLevel::new_clamped(level)],
                    },
                )
                .expect("insert");
            world
                .insert(
                    citizen,
                    Wallet {
                        cash: Money::from_mills(cash),
                    },
                )
                .expect("insert");
            world
                .insert(citizen, LodTier { tier: Tier::C })
                .expect("insert");
            citizen
        };
        // Hungry and funded: buys until the shelf (stock 2) empties.
        let hungry = spawn_c(&mut world, 0, 500);
        // Nearly full: deficit 150k < half the 400k unit — no purchase.
        let sated = spawn_c(&mut world, 850_000, 500);
        // Hungry but broke: the till refuses.
        let broke = spawn_c(&mut world, 0, 50);

        let tables = {
            let mut tables = super::super::tests::test_tables();
            tables.lod.leisure_hours_per_day = 0; // isolate purchases
            tables
        };
        let ctx = TickContext {
            tick: Ticks::new(0),
            time: CalendarTime::START,
        };
        TierCSystem::new(tables)
            .run(&mut world, &ctx, &mut CommandBuffer::new())
            .expect("run");

        let level = |world: &core_ecs::World, who: Entity| {
            world
                .get::<Needs>(who)
                .expect("query")
                .expect("needs")
                .levels[0]
                .raw()
        };
        let cash = |world: &core_ecs::World, who: Entity| {
            world
                .get::<Wallet>(who)
                .expect("query")
                .expect("wallet")
                .cash
                .mills()
        };
        assert_eq!(
            level(&world, hungry),
            800_000,
            "two units landed (the shelf emptied at stock 2)"
        );
        assert_eq!(cash(&world, hungry), 300, "two units paid for");
        assert_eq!(level(&world, sated), 850_000, "under half a unit: no buy");
        assert_eq!(cash(&world, sated), 500);
        assert_eq!(level(&world, broke), 0, "a short wallet buys nothing");
        assert_eq!(cash(&world, broke), 50);
        assert_eq!(
            world
                .get::<Inventory>(shop)
                .expect("query")
                .expect("shelf")
                .quantities[0],
            0,
            "the shelf sold out to the first buyer in entity order"
        );
    }
}

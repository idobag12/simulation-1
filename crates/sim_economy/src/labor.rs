//! The Phase 5 labor systems (ADR 0008 §§3–5): daily payroll (fires
//! instead of paying when a wage can't be covered) and the daily
//! double-auction clearing that hires, sets wages, and measures
//! unemployment. All integer arithmetic; no RNG.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use core_ecs::sim_interface::{
    Employment, Fired, FiredReason, Hired, LaborStats, Wallet, WorkingAge,
};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use core_types::{ArithmeticError, Money};

use crate::components::Firm;
use crate::config::EconTables;

/// The ledger entity — the economy's presence signal, shared with every
/// other economy system (`EconCounters`, not `LaborStats`: a world
/// migrated from v5 HAS an economy but genesis never gave it a labor
/// ledger). The stats row is created here on first use, so migrated
/// towns catch up honestly on their first day boundary instead of
/// never hiring again (ADR 0008 §8).
fn ledger_entity(world: &mut World) -> Result<Option<Entity>, EcsError> {
    let Some(ledger) = world
        .iter::<core_ecs::sim_interface::EconCounters>()?
        .next()
        .map(|(entity, _)| entity)
    else {
        return Ok(None);
    };
    if world.get::<LaborStats>(ledger)?.is_none() {
        world.insert(ledger, LaborStats::default())?;
    }
    Ok(Some(ledger))
}

fn bump_stats(
    world: &mut World,
    ledger: Entity,
    update: impl FnOnce(&mut LaborStats),
) -> Result<(), EcsError> {
    if let Some(stats) = world.get_mut::<LaborStats>(ledger)? {
        update(stats);
    }
    Ok(())
}

/// Day-rate system (ADR 0008 §4), after the audit and before the market:
/// every firm pays every employee their cleared daily wage — an atomic
/// transfer booked as firm expense. A firm that cannot cover a wage
/// fires that employee instead (reason `Insolvent`); employees process
/// in entity order, so the shortfall lands deterministically. Firms over
/// their position count (data shrank) fire the highest-indexed extras
/// (reason `Redundant`) before anyone is paid.
pub struct PayrollSystem {
    tables: EconTables,
}

impl PayrollSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: EconTables) -> Self {
        PayrollSystem { tables }
    }
}

impl System for PayrollSystem {
    fn name(&self) -> &'static str {
        "econ.payroll"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let Some(ledger) = ledger_entity(world)? else {
            return Ok(()); // no economy at all (migrated pre-v5 world)
        };
        // Pass 1 (immutable): the payroll roster in citizen entity order,
        // plus per-firm headcounts for the redundancy check.
        let roster: Vec<(Entity, Entity, Money)> = world
            .iter::<Employment>()?
            .map(|(citizen, employment)| (citizen, employment.employer, employment.wage_per_day))
            .collect();
        let mut headcount: BTreeMap<u32, u32> = BTreeMap::new();
        for (_, employer, _) in &roster {
            *headcount.entry(employer.index()).or_insert(0) += 1;
        }

        // Redundancy: fire highest-indexed extras down to positions.
        let mut fired: Vec<Entity> = Vec::new();
        for (citizen, employer, _) in roster.iter().rev() {
            let positions = world
                .get::<Firm>(*employer)?
                .and_then(|firm| self.tables.firm_kinds.get(firm.kind as usize))
                .map(|kind| kind.positions)
                .unwrap_or(0);
            let count = headcount.entry(employer.index()).or_insert(0);
            if *count > positions {
                *count -= 1;
                fired.push(*citizen);
                world.remove::<Employment>(*citizen)?;
                world.emit(&Fired {
                    citizen: *citizen,
                    employer: *employer,
                    reason: FiredReason::Redundant,
                })?;
                bump_stats(world, ledger, |stats| stats.firings += 1)?;
            }
        }

        // Wages, citizen entity order: pay or fire.
        for (citizen, employer, wage) in roster {
            if fired.contains(&citizen) {
                continue;
            }
            let cash = world
                .get::<Wallet>(employer)?
                .map(|wallet| wallet.cash)
                .unwrap_or(Money::ZERO);
            if cash >= wage {
                transfer_wage(world, employer, citizen, wage)?;
            } else {
                world.remove::<Employment>(citizen)?;
                world.emit(&Fired {
                    citizen,
                    employer,
                    reason: FiredReason::Insolvent,
                })?;
                bump_stats(world, ledger, |stats| stats.firings += 1)?;
            }
        }
        Ok(())
    }
}

/// The wage transfer (ADR 0008 §4): employer wallet → employee wallet,
/// booked as employer expense — employees carry no books. Every leg is
/// structurally required (ADR 0007 §8b).
fn transfer_wage(
    world: &mut World,
    employer: Entity,
    employee: Entity,
    wage: Money,
) -> Result<(), EcsError> {
    let missing = |leg: &str, entity: Entity| {
        EcsError::InvariantViolation(format!(
            "payroll leg missing: entity #{} has no {leg}",
            entity.index()
        ))
    };
    let wallet = world
        .get_mut::<Wallet>(employer)?
        .ok_or_else(|| missing("employer wallet", employer))?;
    wallet.cash = wallet.cash.try_sub(wage)?;
    let wallet = world
        .get_mut::<Wallet>(employee)?
        .ok_or_else(|| missing("employee wallet", employee))?;
    wallet.cash = wallet.cash.try_add(wage)?;
    let books = world
        .get_mut::<core_ecs::sim_interface::FirmBooks>(employer)?
        .ok_or_else(|| missing("employer books", employer))?;
    books.expenses = books.expenses.try_add(wage)?;
    Ok(())
}

/// Day-rate system (ADR 0008 §3), after payroll: the daily double
/// auction. Firms bid a data-defined share of a worker's expected daily
/// marginal product (clamped to their wallet); unemployed working-age
/// citizens ask their reservation wage (wealth raises it, the
/// data-defined trait lowers it). Bids sort descending, asks ascending
/// (ties: entity index); matches clear at the midpoint. Afterwards the
/// clearing writes [`LaborStats`] — unemployment is measured, never
/// assigned (SPEC §12).
pub struct LaborMarketSystem {
    tables: EconTables,
}

impl LaborMarketSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: EconTables) -> Self {
        LaborMarketSystem { tables }
    }

    /// A citizen's reservation wage (exact integer arithmetic).
    pub(crate) fn reservation(
        &self,
        cash_mills: i64,
        trait_per_mille: i64,
    ) -> Result<i64, EcsError> {
        let labor = &self.tables.labor;
        let base = labor.reservation_base_mills;
        let overflow = |op: &'static str| EcsError::Arithmetic(ArithmeticError::Overflow { op });
        // Wealth raise: base × per_mille/1000 × wallet/(wallet + half).
        let cash = cash_mills.max(0);
        let raise = base
            .checked_mul(labor.reservation_wealth_per_mille)
            .and_then(|x| x.checked_mul(cash))
            .ok_or_else(|| overflow("reservation raise"))?
            / 1000
            / cash
                .checked_add(labor.reservation_half_wealth_mills)
                .ok_or_else(|| overflow("reservation divisor"))?
                .max(1);
        // Trait discount: base × discount/1000 × trait/1000.
        let discount = base
            .checked_mul(labor.reservation_trait_discount_per_mille)
            .and_then(|x| x.checked_mul(trait_per_mille.clamp(0, 1000)))
            .ok_or_else(|| overflow("reservation discount"))?
            / 1_000_000;
        Ok(base
            .checked_add(raise)
            .and_then(|x| x.checked_sub(discount))
            .ok_or_else(|| overflow("reservation total"))?
            .max(1))
    }

    /// A firm's bid for one worker: `bid_fraction` of the expected daily
    /// marginal product (output per worker-day at the posted price),
    /// clamped so the wallet could cover the wages already committed to
    /// incumbents PLUS this day's open slots (ADR 0008 §3).
    fn bid(
        &self,
        kind: &crate::config::FirmKindTable,
        posted_price: Money,
        cash: Money,
        committed_wages: i64,
        open_slots: u32,
    ) -> Result<i64, EcsError> {
        let overflow = |op: &'static str| EcsError::Arithmetic(ArithmeticError::Overflow { op });
        let recipe =
            self.tables
                .recipes
                .get(kind.recipe as usize)
                .ok_or(EcsError::InternalCorruption(
                    "firm kind references a recipe outside the loaded data",
                ))?;
        let shift_hours = i64::from(self.tables.labor.shift_end_hour)
            - i64::from(self.tables.labor.shift_start_hour);
        let batches_per_day = (shift_hours / i64::from(recipe.batch_hours.max(1))).max(0);
        let daily_output = recipe
            .output_quantity
            .checked_mul(batches_per_day)
            .ok_or_else(|| overflow("bid output"))?;
        let marginal = daily_output
            .checked_mul(posted_price.mills())
            .ok_or_else(|| overflow("bid product"))?
            / i64::from(kind.positions.max(1));
        let bid = marginal
            .checked_mul(self.tables.labor.bid_fraction_per_mille)
            .ok_or_else(|| overflow("bid fraction"))?
            / 1000;
        // Affordability: incumbents' committed wages plus every open
        // slot must be payable for a day out of cash on hand.
        let free = cash
            .mills()
            .checked_sub(committed_wages)
            .ok_or_else(|| overflow("bid free cash"))?
            .max(0);
        let affordable = free / i64::from(open_slots.max(1));
        Ok(bid.min(affordable))
    }
}

impl System for LaborMarketSystem {
    fn name(&self) -> &'static str {
        "econ.labor_market"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let Some(ledger) = ledger_entity(world)? else {
            return Ok(()); // no economy at all (migrated pre-v5 world)
        };

        // Pass 1 (immutable): asks — unemployed working-age citizens, in
        // entity order.
        let mut asks: Vec<(Entity, i64)> = Vec::new();
        let mut working_age: u32 = 0;
        for (citizen, _) in world.iter::<WorkingAge>()? {
            working_age += 1;
            if world.get::<Employment>(citizen)?.is_some() {
                continue;
            }
            let cash = world
                .get::<Wallet>(citizen)?
                .map(|wallet| wallet.cash.mills())
                .unwrap_or(0);
            let trait_per_mille = world
                .get::<core_ecs::sim_interface::Personality>(citizen)?
                .and_then(|personality| {
                    personality
                        .weights
                        .get(self.tables.labor.reservation_trait as usize)
                        .copied()
                })
                .unwrap_or(0);
            asks.push((citizen, self.reservation(cash, i64::from(trait_per_mille))?));
        }
        let seeking = asks.len() as u32;
        asks.sort_by_key(|(citizen, ask)| (*ask, citizen.index()));

        // Bids — one per open slot, in firm entity order.
        let mut headcount: BTreeMap<u32, u32> = BTreeMap::new();
        let mut committed: BTreeMap<u32, i64> = BTreeMap::new();
        for (_, employment) in world.iter::<Employment>()? {
            *headcount.entry(employment.employer.index()).or_insert(0) += 1;
            *committed.entry(employment.employer.index()).or_insert(0) +=
                employment.wage_per_day.mills();
        }
        let mut bids: Vec<(Entity, i64)> = Vec::new();
        for (firm_entity, firm) in world.iter::<Firm>()? {
            let Some(kind) = self.tables.firm_kinds.get(firm.kind as usize) else {
                continue;
            };
            let employees = headcount.get(&firm_entity.index()).copied().unwrap_or(0);
            let open = kind.positions.saturating_sub(employees);
            if open == 0 {
                continue;
            }
            let cash = world
                .get::<Wallet>(firm_entity)?
                .map(|wallet| wallet.cash)
                .unwrap_or(Money::ZERO);
            let committed_wages = committed.get(&firm_entity.index()).copied().unwrap_or(0);
            let bid = self.bid(kind, firm.posted_price, cash, committed_wages, open)?;
            if bid < 1 {
                continue;
            }
            for _ in 0..open {
                bids.push((firm_entity, bid));
            }
        }
        bids.sort_by_key(|(firm, bid)| (Reverse(*bid), firm.index()));

        // Clearing: match while the best bid covers the best ask; the
        // wage is the midpoint, rounded down to whole mills.
        let mut hires: Vec<(Entity, Entity, Money)> = Vec::new();
        for ((citizen, ask), (firm, bid)) in asks.iter().zip(&bids) {
            if bid < ask {
                break;
            }
            let midpoint =
                ask.checked_add(*bid)
                    .ok_or(EcsError::Arithmetic(ArithmeticError::Overflow {
                        op: "wage midpoint",
                    }))?
                    / 2;
            hires.push((*citizen, *firm, Money::from_mills(midpoint)));
        }
        let hired = hires.len() as u32;
        for (citizen, employer, wage_per_day) in hires {
            world.insert(
                citizen,
                Employment {
                    employer,
                    wage_per_day,
                },
            )?;
            world.emit(&Hired {
                citizen,
                employer,
                wage_per_day,
            })?;
        }

        // Measurement (ADR 0008 §5): written from what actually happened.
        let employed = world.iter::<Employment>()?.count() as u32;
        bump_stats(world, ledger, |stats| {
            stats.working_age = working_age;
            stats.employed = employed;
            stats.seeking = seeking;
            stats.unmatched = seeking - hired;
            stats.hires += u64::from(hired);
        })?;
        Ok(())
    }
}

//! The daily payroll (ADR 0008 §4; ADR 0009 §4 withholding; ADR 0010
//! §1 learning by doing). Split from `labor.rs` for the SPEC §3
//! module-size rule.

use std::collections::BTreeMap;

use core_ecs::sim_interface::{Employment, Fired, FiredReason, Wallet};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use core_types::{ArithmeticError, Money};

use crate::components::Firm;
use crate::config::EconTables;
use crate::labor::{bump_stats, ledger_entity};

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
        // The treasury is an employer without a `Firm` — its slot count
        // is the data `public_positions` (ADR 0009 §4), not zero.
        let mut fired: Vec<Entity> = Vec::new();
        for (citizen, employer, _) in roster.iter().rev() {
            let positions = match world.get::<Firm>(*employer)? {
                Some(firm) => self
                    .tables
                    .firm_kinds
                    .get(firm.kind as usize)
                    .map(|kind| kind.positions)
                    .unwrap_or(0),
                None if world
                    .get::<core_ecs::sim_interface::TreasuryBook>(*employer)?
                    .is_some() =>
                {
                    self.tables.money.public_positions
                }
                None => 0,
            };
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
                transfer_wage(
                    world,
                    employer,
                    citizen,
                    wage,
                    self.tables.money.income_per_mille,
                )?;
                // Learning by doing (Phase 7, ADR 0010 §1): a paid day
                // raises the employer's recipe skill (the treasury's
                // slots teach the public skill). Lazy rows: migrated
                // citizens learn on their first payday.
                let skill = match world.get::<Firm>(employer)? {
                    Some(firm) => self
                        .tables
                        .recipes
                        .get(firm.recipe as usize)
                        .and_then(|recipe| recipe.skill),
                    None => Some(self.tables.public_skill),
                };
                if let Some(skill) = skill {
                    let gain = self.tables.doing_gain_per_shift_per_mille;
                    if gain > 0 {
                        let mut skills = world
                            .get::<core_ecs::sim_interface::Skills>(citizen)?
                            .cloned()
                            .unwrap_or(core_ecs::sim_interface::Skills { levels: Vec::new() });
                        // One canonical row shape: the full data skill
                        // count (short rows would silently no-op a
                        // later Graduate bump).
                        if skills.levels.len() < self.tables.skill_count as usize {
                            skills.levels.resize(self.tables.skill_count as usize, 0);
                        }
                        if let Some(level) = skills.levels.get_mut(skill as usize) {
                            *level = level.saturating_add(gain).min(1000);
                        }
                        world.insert(citizen, skills)?;
                    }
                }
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

/// The wage transfer (ADR 0008 §4; ADR 0009 §4): employer wallet →
/// employee wallet, with income tax withheld to the treasury in the same
/// atomic call, booked as employer expense (the full wage) and treasury
/// revenue (the tax). Every leg is structurally required (ADR 0007 §8b);
/// worlds without a treasury (migrated) withhold nothing, and the
/// treasury's own payroll is not taxed back into itself.
fn transfer_wage(
    world: &mut World,
    employer: Entity,
    employee: Entity,
    wage: Money,
    income_tax_per_mille: i64,
) -> Result<(), EcsError> {
    let missing = |leg: &str, entity: Entity| {
        EcsError::InvariantViolation(format!(
            "payroll leg missing: entity #{} has no {leg}",
            entity.index()
        ))
    };
    let treasury = world
        .iter::<core_ecs::sim_interface::TreasuryBook>()?
        .next()
        .map(|(entity, _)| entity)
        .filter(|treasury| *treasury != employer);
    let tax = match treasury {
        Some(_) => Money::from_mills(
            wage.mills()
                .checked_mul(income_tax_per_mille)
                .ok_or(EcsError::Arithmetic(ArithmeticError::Overflow {
                    op: "income tax",
                }))?
                / 1000,
        ),
        None => Money::ZERO,
    };
    let net = wage.try_sub(tax)?;
    let wallet = world
        .get_mut::<Wallet>(employer)?
        .ok_or_else(|| missing("employer wallet", employer))?;
    wallet.cash = wallet.cash.try_sub(wage)?;
    let wallet = world
        .get_mut::<Wallet>(employee)?
        .ok_or_else(|| missing("employee wallet", employee))?;
    wallet.cash = wallet.cash.try_add(net)?;
    let books = world
        .get_mut::<core_ecs::sim_interface::FirmBooks>(employer)?
        .ok_or_else(|| missing("employer books", employer))?;
    books.expenses = books.expenses.try_add(wage)?;
    if let Some(treasury) = treasury
        && tax > Money::ZERO
    {
        let wallet = world
            .get_mut::<Wallet>(treasury)?
            .ok_or_else(|| missing("treasury wallet", treasury))?;
        wallet.cash = wallet.cash.try_add(tax)?;
        let books = world
            .get_mut::<core_ecs::sim_interface::FirmBooks>(treasury)?
            .ok_or_else(|| missing("treasury books", treasury))?;
        books.revenue = books.revenue.try_add(tax)?;
        if let Some(book) = world.get_mut::<core_ecs::sim_interface::TreasuryBook>(treasury)? {
            book.income_tax_received = book.income_tax_received.try_add(tax)?;
        }
        world.emit(&core_ecs::sim_interface::TaxCollected {
            payer: employee,
            amount: tax,
            kind: core_ecs::sim_interface::TaxKind::Income,
        })?;
    }
    Ok(())
}

//! The conservation auditor (SPEC §12, ADR 0007 §§4–5): recomputes the
//! money and goods identities from the actual stores — never from the
//! counters' own history — and returns a typed error on any drift, which
//! aborts the tick and halts the run. Also checks every firm's
//! double-entry slice (ADR 0007 §3).

use core_ecs::sim_interface::{EconCounters, FirmBooks, Inventory, Wallet};
use core_ecs::{CommandBuffer, EcsError, System, TickContext, World};
use core_types::Money;

/// Recomputes every Phase 4 audit identity against `world`'s stores.
/// `Ok(false)` = the world has no economy (no `EconCounters` ledger —
/// e.g. migrated from a pre-economy save), so there is honestly nothing
/// to audit; `Ok(true)` = every identity holds; `Err` = drift, with the
/// identity and diff named.
///
/// Identities (ADR 0007 §§3–4):
/// - money:  `Σ wallets == issued`
/// - goods:  per good, `Σ inventories ==
///   produced − consumed_by_citizens − consumed_in_production − spoiled`
/// - ledger: per firm, `cash − initial_cash == revenue − expenses`
pub fn audit_economy(world: &World) -> Result<bool, EcsError> {
    let Some((_, counters)) = world.iter::<EconCounters>()?.next() else {
        return Ok(false);
    };
    let counters = counters.clone();

    // Money: every wallet in the town, citizens and firms alike.
    let mut wallet_sum = Money::ZERO;
    for (_, wallet) in world.iter::<Wallet>()? {
        wallet_sum = wallet_sum.try_add(wallet.cash)?;
    }
    if wallet_sum != counters.issued {
        return Err(EcsError::InvariantViolation(format!(
            "money conservation broke: Σ wallets = {} but issued = {}",
            wallet_sum, counters.issued
        )));
    }

    // Goods: recompute total stock per good from the inventories.
    let goods = counters.produced.len();
    if counters.consumed_by_citizens.len() != goods
        || counters.consumed_in_production.len() != goods
        || counters.spoiled.len() != goods
    {
        return Err(EcsError::InvariantViolation(
            "the conservation ledger's counter vectors disagree on the good count".to_owned(),
        ));
    }
    let mut stock_sum = vec![0i64; goods];
    for (entity, inventory) in world.iter::<Inventory>()? {
        if inventory.quantities.len() != goods {
            return Err(EcsError::InvariantViolation(format!(
                "entity #{} has {} inventory slots but data defines {goods}",
                entity.index(),
                inventory.quantities.len()
            )));
        }
        for (total, held) in stock_sum.iter_mut().zip(&inventory.quantities) {
            if *held < 0 {
                return Err(EcsError::InvariantViolation(format!(
                    "entity #{} holds negative stock",
                    entity.index()
                )));
            }
            *total += held;
        }
    }
    for (good, total) in stock_sum.iter().enumerate() {
        let produced = counters.produced[good];
        let by_citizens = counters.consumed_by_citizens[good];
        let in_production = counters.consumed_in_production[good];
        let spoiled = counters.spoiled[good];
        let expected = produced - by_citizens - in_production - spoiled;
        if *total != expected {
            return Err(EcsError::InvariantViolation(format!(
                "goods conservation broke for good {good}: Σ inventories = {total} but \
                 produced {produced} − citizens {by_citizens} − production {in_production} \
                 − spoiled {spoiled} = {expected}",
            )));
        }
    }

    // Ledgers: the double-entry slice, per firm.
    for (entity, books) in world.iter::<FirmBooks>()? {
        let cash = world
            .get::<Wallet>(entity)?
            .map(|wallet| wallet.cash)
            .unwrap_or(Money::ZERO);
        let cash_delta = cash.try_sub(books.initial_cash)?;
        let booked = books.revenue.try_sub(books.expenses)?;
        if cash_delta != booked {
            return Err(EcsError::InvariantViolation(format!(
                "firm #{}'s books do not balance: cash − initial = {} but \
                 revenue − expenses = {}",
                entity.index(),
                cash_delta,
                booked
            )));
        }
    }
    Ok(true)
}

/// Day-rate system, scheduled FIRST in the day list (ADR 0007 §5): audits
/// yesterday's activity before today's begins. A violation is an error —
/// the tick aborts and the run halts, debug and release.
pub struct AuditSystem;

impl System for AuditSystem {
    fn name(&self) -> &'static str {
        "debug.audit"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        audit_economy(world).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_types::Seed;

    fn econ_world() -> World {
        let mut world = World::new(Seed::new(3), 16);
        world.register::<Wallet>().expect("register");
        world.register::<Inventory>().expect("register");
        world.register::<FirmBooks>().expect("register");
        world.register::<EconCounters>().expect("register");
        world
    }

    #[test]
    fn a_world_without_a_ledger_has_nothing_to_audit() {
        let world = econ_world();
        assert!(!audit_economy(&world).expect("audit"));
    }

    #[test]
    fn balanced_worlds_pass_and_each_drift_kind_is_named() {
        let mut world = econ_world();
        let ledger = world.spawn();
        let mut counters = EconCounters::new(1);
        counters.issued = Money::from_mills(500);
        counters.produced = vec![7];
        counters.spoiled = vec![2];
        world.insert(ledger, counters).expect("insert");
        let firm = world.spawn();
        world
            .insert(
                firm,
                Wallet {
                    cash: Money::from_mills(500),
                },
            )
            .expect("insert");
        world
            .insert(
                firm,
                Inventory {
                    quantities: vec![5],
                },
            )
            .expect("insert");
        world
            .insert(
                firm,
                FirmBooks {
                    initial_cash: Money::from_mills(400),
                    revenue: Money::from_mills(150),
                    expenses: Money::from_mills(50),
                },
            )
            .expect("insert");
        assert!(audit_economy(&world).expect("audit"));

        // Money drift.
        world
            .get_mut::<Wallet>(firm)
            .expect("get")
            .expect("some")
            .cash = Money::from_mills(501);
        let err = audit_economy(&world).expect_err("must drift");
        assert!(err.to_string().contains("money conservation"), "{err}");
        world
            .get_mut::<Wallet>(firm)
            .expect("get")
            .expect("some")
            .cash = Money::from_mills(500);

        // Goods drift.
        world
            .get_mut::<Inventory>(firm)
            .expect("get")
            .expect("some")
            .quantities[0] = 6;
        let err = audit_economy(&world).expect_err("must drift");
        assert!(err.to_string().contains("goods conservation"), "{err}");
        world
            .get_mut::<Inventory>(firm)
            .expect("get")
            .expect("some")
            .quantities[0] = 5;

        // Ledger drift: cash moved without a booking.
        world
            .get_mut::<FirmBooks>(firm)
            .expect("get")
            .expect("some")
            .revenue = Money::from_mills(151);
        let err = audit_economy(&world).expect_err("must drift");
        assert!(err.to_string().contains("books do not balance"), "{err}");
    }
}

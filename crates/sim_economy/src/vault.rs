//! The vault's atomic operations (Phase 6, ADR 0009 §§1–3), shared by
//! the bank system, the housing markets, and estate settlement: grants,
//! deposits, withdrawals, the one day-payment formula, and repossession.
//! Split from `bank.rs` for the SPEC §3 module-size rule.

use core_ecs::sim_interface::{BankBook, FirmBooks, Loan, LoanGranted, Wallet};
use core_ecs::{EcsError, Entity, World};
use core_types::{ArithmeticError, Money};

fn overflow(op: &'static str) -> EcsError {
    EcsError::Arithmetic(ArithmeticError::Overflow { op })
}

/// Grants a loan: vault → borrower cash, a book row, the borrower's
/// inflow ledger, and the fact — one atomic call (ADR 0007 §4
/// discipline). Also used by the purchase market for mortgages (the
/// principal goes to the SELLER there; this variant pays the borrower).
pub(crate) fn grant_loan(
    world: &mut World,
    bank: Entity,
    borrower: Entity,
    principal: Money,
    rate_per_million_daily: i64,
    term_days: i64,
    collateral: Option<Entity>,
) -> Result<(), EcsError> {
    let day_payment = day_payment(principal, rate_per_million_daily, term_days)?;
    if let Some(wallet) = world.get_mut::<Wallet>(bank)? {
        wallet.cash = wallet.cash.try_sub(principal)?;
    }
    if let Some(wallet) = world.get_mut::<Wallet>(borrower)? {
        wallet.cash = wallet.cash.try_add(principal)?;
    }
    if let Some(books) = world.get_mut::<FirmBooks>(borrower)? {
        books.revenue = books.revenue.try_add(principal)?;
    }
    if let Some(book) = world.get_mut::<BankBook>(bank)? {
        book.loans.push(Loan {
            borrower,
            principal,
            rate_per_million_daily,
            day_payment,
            collateral,
        });
        // Lifetime granted-principal row (owner entity-index order):
        // the serviceability screen nets this out of booked revenue.
        match book
            .granted
            .iter_mut()
            .find(|(entity, _)| *entity == borrower)
        {
            Some(row) => row.1 = row.1.try_add(principal)?,
            None => {
                book.granted.push((borrower, principal));
                book.granted.sort_by_key(|(entity, _)| entity.index());
            }
        }
    }
    world.emit(&LoanGranted {
        borrower,
        principal,
        rate_per_million_daily,
    })?;
    Ok(())
}

/// Grants a MORTGAGE (ADR 0009 §3): the principal goes to the SELLER
/// (the buyer never touches it), the loan lands in the buyer's name
/// secured by the home, and the granted-row/day-payment bookkeeping
/// matches `grant_loan` exactly. Callers pre-check the vault's cash.
#[allow(clippy::too_many_arguments)]
pub(crate) fn grant_mortgage(
    world: &mut World,
    bank: Entity,
    borrower: Entity,
    seller: Entity,
    principal: Money,
    rate_per_million_daily: i64,
    term_days: i64,
    home: Entity,
) -> Result<(), EcsError> {
    let day_payment = day_payment(principal, rate_per_million_daily, term_days)?;
    if let Some(wallet) = world.get_mut::<Wallet>(bank)? {
        wallet.cash = wallet.cash.try_sub(principal)?;
    }
    if let Some(wallet) = world.get_mut::<Wallet>(seller)? {
        wallet.cash = wallet.cash.try_add(principal)?;
    }
    if let Some(book) = world.get_mut::<BankBook>(bank)? {
        book.loans.push(Loan {
            borrower,
            principal,
            rate_per_million_daily,
            day_payment,
            collateral: Some(home),
        });
        match book
            .granted
            .iter_mut()
            .find(|(entity, _)| *entity == borrower)
        {
            Some(row) => row.1 = row.1.try_add(principal)?,
            None => {
                book.granted.push((borrower, principal));
                book.granted.sort_by_key(|(entity, _)| entity.index());
            }
        }
    }
    world.emit(&LoanGranted {
        borrower,
        principal,
        rate_per_million_daily,
    })?;
    Ok(())
}

/// The fixed daily payment a loan books: principal/term plus interest at
/// the contract rate — the ONE formula, shared by grants, the
/// serviceability screen, and the mortgage path (checked throughout).
pub(crate) fn day_payment(
    principal: Money,
    rate_per_million_daily: i64,
    term_days: i64,
) -> Result<Money, EcsError> {
    Ok(Money::from_mills(
        (principal.mills() / term_days.max(1))
            .checked_add(
                principal
                    .mills()
                    .checked_mul(rate_per_million_daily)
                    .ok_or_else(|| overflow("payment interest"))?
                    / 1_000_000,
            )
            .ok_or_else(|| overflow("day payment"))?
            .max(1),
    ))
}

/// Lifetime principal ever granted to `borrower` (ZERO if none).
pub(crate) fn granted_principal(
    world: &World,
    bank: Entity,
    borrower: Entity,
) -> Result<Money, EcsError> {
    Ok(world
        .get::<BankBook>(bank)?
        .and_then(|book| {
            book.granted
                .iter()
                .find(|(entity, _)| *entity == borrower)
                .map(|(_, total)| *total)
        })
        .unwrap_or(Money::ZERO))
}

/// Repossession (ADR 0009 §3: the home is the mortgage's collateral):
/// on default the collateral's ownership passes to the bank, and a
/// defaulting occupant loses the Residence (and any Tenancy) pointing
/// at it — the home is then vacant, bank-owned, and re-enters the
/// purchase market at the next clearing. The bank's eventual sale
/// proceeds credit equity (the recovery against the write-off).
pub(crate) fn repossess(world: &mut World, bank: Entity, loan: &Loan) -> Result<(), EcsError> {
    let Some(home) = loan.collateral else {
        return Ok(());
    };
    if !world.is_alive(home) {
        return Ok(());
    }
    // Only repossess what the borrower still owns — an already-escheated
    // or resold home is someone else's.
    if world
        .get::<core_ecs::sim_interface::Ownership>(home)?
        .map(|o| o.owner)
        != Some(loan.borrower)
    {
        return Ok(());
    }
    world.insert(home, core_ecs::sim_interface::Ownership { owner: bank })?;
    if world.is_alive(loan.borrower) {
        if world
            .get::<core_ecs::sim_interface::Residence>(loan.borrower)?
            .is_some_and(|residence| residence.home == home)
        {
            world.remove::<core_ecs::sim_interface::Residence>(loan.borrower)?;
        }
        if world
            .get::<core_ecs::sim_interface::Tenancy>(loan.borrower)?
            .is_some_and(|tenancy| tenancy.home == home)
        {
            world.remove::<core_ecs::sim_interface::Tenancy>(loan.borrower)?;
        }
    }
    Ok(())
}

/// The borrower's current deposit balance.
pub(crate) fn deposit_balance(
    world: &World,
    bank: Entity,
    owner: Entity,
) -> Result<Money, EcsError> {
    Ok(world
        .get::<BankBook>(bank)?
        .and_then(|book| {
            book.deposits
                .iter()
                .find(|(entity, _)| *entity == owner)
                .map(|(_, balance)| *balance)
        })
        .unwrap_or(Money::ZERO))
}

/// Moves cash wallet → vault and credits the owner's row (kept in owner
/// entity-index order).
pub(crate) fn vault_deposit(
    world: &mut World,
    bank: Entity,
    owner: Entity,
    amount: Money,
) -> Result<(), EcsError> {
    if amount <= Money::ZERO {
        return Ok(());
    }
    if let Some(wallet) = world.get_mut::<Wallet>(owner)? {
        wallet.cash = wallet.cash.try_sub(amount)?;
    }
    if let Some(wallet) = world.get_mut::<Wallet>(bank)? {
        wallet.cash = wallet.cash.try_add(amount)?;
    }
    if let Some(book) = world.get_mut::<BankBook>(bank)? {
        match book
            .deposits
            .iter_mut()
            .find(|(entity, _)| *entity == owner)
        {
            Some(row) => row.1 = row.1.try_add(amount)?,
            None => {
                book.deposits.push((owner, amount));
                book.deposits.sort_by_key(|(entity, _)| entity.index());
            }
        }
    }
    Ok(())
}

/// Moves cash vault → wallet and debits the owner's row (which must
/// cover it — callers check).
pub(crate) fn vault_withdraw(
    world: &mut World,
    bank: Entity,
    owner: Entity,
    amount: Money,
) -> Result<(), EcsError> {
    if amount <= Money::ZERO {
        return Ok(());
    }
    if let Some(book) = world.get_mut::<BankBook>(bank)? {
        let row = book
            .deposits
            .iter_mut()
            .find(|(entity, _)| *entity == owner)
            .ok_or_else(|| {
                EcsError::InvariantViolation(format!(
                    "withdrawal without a deposit row for entity #{}",
                    owner.index()
                ))
            })?;
        row.1 = row.1.try_sub(amount)?;
        if row.1 < Money::ZERO {
            return Err(EcsError::InvariantViolation(format!(
                "entity #{}'s deposit row went negative",
                owner.index()
            )));
        }
    }
    if let Some(wallet) = world.get_mut::<Wallet>(bank)? {
        wallet.cash = wallet.cash.try_sub(amount)?;
    }
    if let Some(wallet) = world.get_mut::<Wallet>(owner)? {
        wallet.cash = wallet.cash.try_add(amount)?;
    }
    Ok(())
}

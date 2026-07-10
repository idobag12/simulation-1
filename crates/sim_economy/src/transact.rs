//! Checked arithmetic and the atomic transaction legs shared by the
//! economy systems (split from `systems.rs` for the SPEC §3 module-size
//! rule; discipline per ADR 0007 §§4, 8b).

use core_ecs::sim_interface::{EconCounters, FirmBooks, Inventory, Wallet};
use core_ecs::{EcsError, Entity, World};
use core_types::{ArithmeticError, Money};

/// `ceil(a / b)` for positive operands (exact integer pricing math;
/// checked — SPEC §2).
pub(crate) fn div_ceil(a: i64, b: i64, op: &'static str) -> Result<i64, EcsError> {
    Ok(a.checked_add(b - 1)
        .ok_or(EcsError::Arithmetic(ArithmeticError::Overflow { op }))?
        / b)
}

/// `a × b` with the overflow surfaced as a typed error (SPEC §2: checked
/// arithmetic wherever data-driven values multiply).
pub(crate) fn mul(a: i64, b: i64, op: &'static str) -> Result<i64, EcsError> {
    a.checked_mul(b)
        .ok_or(EcsError::Arithmetic(ArithmeticError::Overflow { op }))
}

/// `a + b`, checked (SPEC §2 — the sums of checked products must not
/// silently wrap either).
pub(crate) fn add(a: i64, b: i64, op: &'static str) -> Result<i64, EcsError> {
    a.checked_add(b)
        .ok_or(EcsError::Arithmetic(ArithmeticError::Overflow { op }))
}

/// A missing structurally-required transaction leg (ADR 0007 §8b): the
/// error names the leg so a half-executed transfer can never be deferred
/// to an unattributed audit halt a day later.
pub(crate) fn missing(leg: &str, entity: Entity) -> EcsError {
    EcsError::InvariantViolation(format!(
        "transaction leg missing: entity #{} has no {leg}",
        entity.index()
    ))
}

/// Moves `total` from `buyer`'s wallet to `seller`'s and books it on both
/// firms' ledgers (buyer expense, seller revenue) — the money half of a
/// firm-to-firm trade. Every leg is structurally required: a missing
/// wallet or book is a typed error, never a silently skipped half of an
/// "atomic" transfer (ADR 0007 §§4, 8b). The caller has already verified
/// the buyer can pay.
pub(crate) fn transfer_money(
    world: &mut World,
    buyer: Entity,
    seller: Entity,
    total: Money,
) -> Result<(), EcsError> {
    let wallet = world
        .get_mut::<Wallet>(buyer)?
        .ok_or_else(|| missing("buyer wallet", buyer))?;
    wallet.cash = wallet.cash.try_sub(total)?;
    let wallet = world
        .get_mut::<Wallet>(seller)?
        .ok_or_else(|| missing("seller wallet", seller))?;
    wallet.cash = wallet.cash.try_add(total)?;
    let books = world
        .get_mut::<FirmBooks>(buyer)?
        .ok_or_else(|| missing("buyer books", buyer))?;
    books.expenses = books.expenses.try_add(total)?;
    let books = world
        .get_mut::<FirmBooks>(seller)?
        .ok_or_else(|| missing("seller books", seller))?;
    books.revenue = books.revenue.try_add(total)?;
    Ok(())
}

/// Adds `delta` to `holder`'s stock of `good` (negative = remove; the
/// caller has already verified stock covers a removal). A missing
/// inventory or good slot is a typed error (ADR 0007 §8b).
pub(crate) fn adjust_stock(
    world: &mut World,
    holder: Entity,
    good: u32,
    delta: i64,
) -> Result<(), EcsError> {
    let inventory = world
        .get_mut::<Inventory>(holder)?
        .ok_or_else(|| missing("inventory", holder))?;
    let stock = inventory
        .quantities
        .get_mut(good as usize)
        .ok_or_else(|| missing("inventory slot for the traded good", holder))?;
    *stock = stock
        .checked_add(delta)
        .ok_or(EcsError::Arithmetic(ArithmeticError::Overflow {
            op: "stock adjust",
        }))?;
    Ok(())
}

/// Adds `delta` to one of the ledger's per-good counter vectors, selected
/// by `select`. A missing ledger or slot is a typed error (ADR 0007 §8b).
pub(crate) fn count(
    world: &mut World,
    ledger: Entity,
    good: u32,
    delta: i64,
    select: fn(&mut EconCounters) -> &mut Vec<i64>,
) -> Result<(), EcsError> {
    let counters = world
        .get_mut::<EconCounters>(ledger)?
        .ok_or_else(|| missing("conservation ledger", ledger))?;
    let entry = select(counters)
        .get_mut(good as usize)
        .ok_or_else(|| missing("counter slot for the good", ledger))?;
    *entry = entry
        .checked_add(delta)
        .ok_or(EcsError::Arithmetic(ArithmeticError::Overflow {
            op: "counter add",
        }))?;
    Ok(())
}

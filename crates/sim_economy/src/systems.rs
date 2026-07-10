//! The Phase 4 economy systems (ADR 0007 §5): production (hour rate),
//! firm-to-firm procurement and posted-price repricing (day rate). Every
//! transfer is atomic — wallets, inventories, books, and counters move in
//! the same call (ADR 0007 §4).

use core_ecs::sim_interface::{
    EconCounters, Employment, FirmBooks, GoodsPurchased, Inventory, Position, PriceChanged,
    RetailOffer, Wallet,
};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use core_types::{ArithmeticError, Money};

use crate::components::{Firm, Production};
use crate::config::EconTables;

/// `ceil(a / b)` for positive operands (exact integer pricing math;
/// checked — SPEC §2).
fn div_ceil(a: i64, b: i64, op: &'static str) -> Result<i64, EcsError> {
    Ok(a.checked_add(b - 1)
        .ok_or(EcsError::Arithmetic(ArithmeticError::Overflow { op }))?
        / b)
}

/// `a × b` with the overflow surfaced as a typed error (SPEC §2: checked
/// arithmetic wherever data-driven values multiply).
fn mul(a: i64, b: i64, op: &'static str) -> Result<i64, EcsError> {
    a.checked_mul(b)
        .ok_or(EcsError::Arithmetic(ArithmeticError::Overflow { op }))
}

/// `a + b`, checked (SPEC §2 — the sums of checked products must not
/// silently wrap either).
fn add(a: i64, b: i64, op: &'static str) -> Result<i64, EcsError> {
    a.checked_add(b)
        .ok_or(EcsError::Arithmetic(ArithmeticError::Overflow { op }))
}

/// A missing structurally-required transaction leg (ADR 0007 §8b): the
/// error names the leg so a half-executed transfer can never be deferred
/// to an unattributed audit halt a day later.
fn missing(leg: &str, entity: Entity) -> EcsError {
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
fn adjust_stock(world: &mut World, holder: Entity, good: u32, delta: i64) -> Result<(), EcsError> {
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
fn count(
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

/// Hour-rate system (ADR 0007 §5): per firm in entity order, either
/// advance/finish the running batch (outputs land, `produced` counts) or
/// start one by consuming inputs from the firm's own stock
/// (`consumed_in_production` counts). A firm that finishes this hour
/// starts its next batch next hour.
///
/// Phase 5 (ADR 0008 §1): starting a batch also requires the kind's
/// `min_workers` employees PRESENT this hour — an employee counts as
/// present when their `Position` is the firm entity. Labor gates starts,
/// not completions; understaffed firms idle honestly.
pub struct ProductionSystem {
    tables: EconTables,
}

impl ProductionSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: EconTables) -> Self {
        ProductionSystem { tables }
    }
}

impl System for ProductionSystem {
    fn name(&self) -> &'static str {
        "econ.production"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let Some(ledger) = world.iter::<EconCounters>()?.next().map(|(e, _)| e) else {
            return Ok(()); // no economy in this world (migrated pre-v5 town)
        };
        // Pass 1 (immutable): snapshot every firm's state in entity
        // order, plus the hour's worker presence (ADR 0008 §1).
        let mut present: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
        for (citizen, employment) in world.iter::<Employment>()? {
            if world.get::<Position>(citizen)?.map(|p| p.at) == Some(employment.employer) {
                *present.entry(employment.employer.index()).or_insert(0) += 1;
            }
        }
        let firms: Vec<(Entity, u32, u32, Option<u32>)> = {
            let mut list = Vec::new();
            for (entity, firm) in world.iter::<Firm>()? {
                let running = world.get::<Production>(entity)?.map(|p| p.remaining_hours);
                list.push((entity, firm.kind, firm.recipe, running));
            }
            list
        };

        // Pass 2: apply, same order.
        for (entity, kind_index, recipe_index, running) in firms {
            let recipe = self
                .tables
                .recipes
                .get(recipe_index as usize)
                .ok_or(EcsError::InternalCorruption(
                    "firm references a recipe outside the loaded data",
                ))?
                .clone();
            match running {
                Some(remaining) => {
                    if remaining > 1 {
                        world.insert(
                            entity,
                            Production {
                                remaining_hours: remaining - 1,
                            },
                        )?;
                    } else {
                        // Batch complete: outputs land, produced counts.
                        world.remove::<Production>(entity)?;
                        adjust_stock(world, entity, recipe.output_good, recipe.output_quantity)?;
                        count(
                            world,
                            ledger,
                            recipe.output_good,
                            recipe.output_quantity,
                            |c| &mut c.produced,
                        )?;
                    }
                }
                None => {
                    // Start a batch if the shift is staffed and every
                    // input is in stock.
                    let min_workers = self
                        .tables
                        .firm_kinds
                        .get(kind_index as usize)
                        .map(|kind| kind.min_workers)
                        .unwrap_or(u32::MAX);
                    if present.get(&entity.index()).copied().unwrap_or(0) < min_workers {
                        continue;
                    }
                    let can_start = {
                        let Some(inventory) = world.get::<Inventory>(entity)? else {
                            continue;
                        };
                        recipe
                            .inputs
                            .iter()
                            .all(|(good, quantity)| inventory.stock(*good) >= *quantity)
                    };
                    if can_start {
                        for (good, quantity) in &recipe.inputs {
                            adjust_stock(world, entity, *good, -quantity)?;
                            count(world, ledger, *good, *quantity, |c| {
                                &mut c.consumed_in_production
                            })?;
                        }
                        world.insert(
                            entity,
                            Production {
                                remaining_hours: recipe.batch_hours,
                            },
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Day-rate system (ADR 0007 §5): each firm, in entity order, restocks
/// its recipe inputs up to the data-defined target (in batches) by buying
/// from the cheapest firm currently posting that good with stock (ties:
/// lowest entity index). Purchases are sequential — later buyers see
/// earlier buyers' effects — and each is one atomic transfer. Phase 4
/// simplification (documented): one seller per input per day; a partial
/// fill waits for tomorrow.
pub struct TradeSystem {
    tables: EconTables,
}

impl TradeSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: EconTables) -> Self {
        TradeSystem { tables }
    }
}

impl System for TradeSystem {
    fn name(&self) -> &'static str {
        "econ.trade"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        // Firm roster snapshot (entity order); stock/cash/prices are read
        // live per purchase so the market is sequential.
        let firms: Vec<(Entity, u32)> = world
            .iter::<Firm>()?
            .map(|(entity, firm)| (entity, firm.recipe))
            .collect();

        for (buyer, recipe_index) in &firms {
            let recipe = self
                .tables
                .recipes
                .get(*recipe_index as usize)
                .ok_or(EcsError::InternalCorruption(
                    "firm references a recipe outside the loaded data",
                ))?
                .clone();
            for (good, per_batch) in &recipe.inputs {
                let target = mul(
                    *per_batch,
                    self.tables.economy.inventory_target_batches,
                    "trade target",
                )?;
                let held = world
                    .get::<Inventory>(*buyer)?
                    .map(|inventory| inventory.stock(*good))
                    .unwrap_or(0);
                let deficit = target - held;
                if deficit <= 0 {
                    continue;
                }

                // Cheapest seller with stock (first wins ties: entity order).
                let mut best: Option<(Entity, Money, i64)> = None;
                for (seller, seller_recipe) in &firms {
                    if seller == buyer {
                        continue;
                    }
                    let Some(seller_recipe) = self.tables.recipes.get(*seller_recipe as usize)
                    else {
                        continue;
                    };
                    if seller_recipe.output_good != *good {
                        continue;
                    }
                    let stock = world
                        .get::<Inventory>(*seller)?
                        .map(|inventory| inventory.stock(*good))
                        .unwrap_or(0);
                    if stock <= 0 {
                        continue;
                    }
                    let price = match world.get::<Firm>(*seller)? {
                        Some(firm) => firm.posted_price,
                        None => continue,
                    };
                    if best.is_none_or(|(_, best_price, _)| price < best_price) {
                        best = Some((*seller, price, stock));
                    }
                }
                let Some((seller, price, stock)) = best else {
                    continue;
                };

                // Quantity: bounded by deficit, stock, and the buyer's cash.
                let cash = world
                    .get::<Wallet>(*buyer)?
                    .map(|wallet| wallet.cash.mills())
                    .unwrap_or(0);
                let affordable = if price.mills() > 0 {
                    cash / price.mills()
                } else {
                    0
                };
                let quantity = deficit.min(stock).min(affordable);
                if quantity < 1 {
                    continue;
                }
                let total = Money::from_mills(mul(quantity, price.mills(), "trade total")?);

                // The atomic transfer: goods, money, books — one call.
                adjust_stock(world, seller, *good, -quantity)?;
                adjust_stock(world, *buyer, *good, quantity)?;
                transfer_money(world, *buyer, seller, total)?;
                world.emit(&GoodsPurchased {
                    buyer: *buyer,
                    seller,
                    good: *good,
                    quantity,
                    total,
                })?;
            }
        }
        Ok(())
    }
}

/// Day-rate system (SPEC §12 posted prices; ADR 0007 §5): every firm
/// reprices its output — a cost-plus floor (inputs at the market's
/// current posted prices + overhead, marked up) under an inventory
/// controller (stock above target → step down, below → step up), with
/// the per-day movement bounded to the data-defined step. Writes the
/// firm's posted price and mirrors it into its `RetailOffer` in the same
/// call (prices have exactly one writer, so the two can never drift).
pub struct PricingSystem {
    tables: EconTables,
}

impl PricingSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: EconTables) -> Self {
        PricingSystem { tables }
    }

    /// The cost-plus floor for `recipe` under `market` prices: never post
    /// below `ceil((inputs + overhead) × (1 + markup) / output_qty)`.
    fn floor_price(
        &self,
        recipe: &crate::config::RecipeTable,
        market: &[i64],
    ) -> Result<i64, EcsError> {
        let mut batch_cost = self.tables.economy.overhead_mills_per_batch;
        for (good, quantity) in &recipe.inputs {
            let unit = market.get(*good as usize).copied().unwrap_or(0);
            batch_cost = add(
                batch_cost,
                mul(unit, *quantity, "pricing input cost")?,
                "pricing batch cost",
            )?;
        }
        let marked_up = div_ceil(
            mul(
                batch_cost,
                1000 + self.tables.economy.markup_per_mille,
                "pricing markup",
            )?,
            1000,
            "pricing markup ceil",
        )?;
        div_ceil(
            marked_up,
            recipe.output_quantity.max(1),
            "pricing unit ceil",
        )
    }
}

impl System for PricingSystem {
    fn name(&self) -> &'static str {
        "econ.pricing"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        // Pass 1 (immutable): yesterday's market — the cheapest posted
        // price per good across producers — plus every firm's state. One
        // snapshot for all firms: repricing is simultaneous, not
        // sequential (a firm's new price is seen tomorrow).
        let mut market: Vec<i64> = vec![0; self.tables.goods];
        let mut firms: Vec<(Entity, u32, i64, i64)> = Vec::new(); // (entity, recipe, posted, output stock)
        for (entity, firm) in world.iter::<Firm>()? {
            let Some(recipe) = self.tables.recipes.get(firm.recipe as usize) else {
                continue;
            };
            let posted = firm.posted_price.mills();
            if let Some(entry) = market.get_mut(recipe.output_good as usize)
                && (*entry == 0 || posted < *entry)
            {
                *entry = posted;
            }
            let stock = world
                .get::<Inventory>(entity)?
                .map(|inventory| inventory.stock(recipe.output_good))
                .unwrap_or(0);
            firms.push((entity, firm.recipe, posted, stock));
        }

        // Pass 2: reprice every firm, entity order.
        for (entity, recipe_index, posted, stock) in firms {
            let Some(recipe) = self.tables.recipes.get(recipe_index as usize).cloned() else {
                continue;
            };
            let floor = self.floor_price(&recipe, &market)?;
            let target = mul(
                recipe.output_quantity,
                self.tables.economy.inventory_target_batches,
                "pricing target",
            )?;
            let step = (mul(
                posted,
                self.tables.economy.controller_step_per_mille,
                "pricing step",
            )? / 1000)
                .max(1);
            let desired = if stock > target {
                posted - step // posted >= min_price >= 1 and step <= posted: no underflow
            } else if stock < target {
                add(posted, step, "pricing raise")?
            } else {
                posted
            };
            let new = desired.max(floor).clamp(
                self.tables.economy.min_price_mills,
                self.tables.economy.max_price_mills,
            );
            if new != posted {
                if let Some(firm) = world.get_mut::<Firm>(entity)? {
                    firm.posted_price = Money::from_mills(new);
                }
                if let Some(offer) = world.get_mut::<RetailOffer>(entity)? {
                    offer.unit_price = Money::from_mills(new);
                }
                world.emit(&PriceChanged {
                    firm: entity,
                    good: recipe.output_good,
                    old: Money::from_mills(posted),
                    new: Money::from_mills(new),
                })?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "systems_tests.rs"]
mod tests;

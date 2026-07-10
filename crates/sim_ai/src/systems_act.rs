//! `ActSystem`: advances every `CurrentAction` with exact integer effects
//! (split from `systems.rs` for the SPEC §3 module-size rule). Purchases
//! execute here, atomically, at performance start (ADR 0007 §6).

use core_ecs::sim_interface::{
    EconCounters, FirmBooks, GoodsPurchased, Inventory, Location, Needs, Position, RetailOffer,
    Wallet,
};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};

use crate::components::CurrentAction;
use crate::config::AiTables;

use super::NEED_MAX;

/// One entity's resolved transition for this tick (two-pass, SPEC §6).
enum Transition {
    Continue(CurrentAction),
    Arrive {
        at: Entity,
        next: CurrentAction,
    },
    GainAndContinue {
        need_index: u32,
        amount: i64,
        next: Option<CurrentAction>,
    },
    Finish,
    /// Attempt the atomic purchase at `at` — resolved in the apply pass
    /// against LIVE stock and cash, sequentially in entity order, so two
    /// buyers in the same tick can never oversell a shelf.
    Purchase {
        at: Entity,
    },
    /// A completed school attendance (Phase 7, ADR 0010 §1): the skill
    /// bump and the fact land in the apply pass.
    Graduate {
        at: Entity,
    },
}

/// Tick-rate system (after `DecideSystem`): advances every
/// `CurrentAction`. Travel counts down and sets `Position` on arrival;
/// performance applies the location's exact per-tick need gain and ends
/// when the need fills or the duration elapses; idling counts down.
pub struct ActSystem {
    tables: AiTables,
}

impl ActSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: AiTables) -> Self {
        ActSystem { tables }
    }

    fn transition(
        &self,
        world: &World,
        entity: Entity,
        action: CurrentAction,
    ) -> Result<Transition, EcsError> {
        Ok(match action {
            CurrentAction::Idle { remaining } => {
                if remaining > 1 {
                    Transition::Continue(CurrentAction::Idle {
                        remaining: remaining - 1,
                    })
                } else {
                    Transition::Finish
                }
            }
            CurrentAction::Travel {
                target,
                need_index,
                remaining,
            } => {
                if remaining > 1 {
                    Transition::Continue(CurrentAction::Travel {
                        target,
                        need_index,
                        remaining: remaining - 1,
                    })
                } else {
                    Transition::Arrive {
                        at: target,
                        next: CurrentAction::Perform {
                            at: target,
                            need_index,
                            remaining: self.tables.max_perform_ticks,
                        },
                    }
                }
            }
            CurrentAction::BuyTravel { target, remaining } => {
                if remaining > 1 {
                    Transition::Continue(CurrentAction::BuyTravel {
                        target,
                        remaining: remaining - 1,
                    })
                } else {
                    Transition::Arrive {
                        at: target,
                        next: CurrentAction::BuyPending { at: target },
                    }
                }
            }
            CurrentAction::BuyPending { at } => Transition::Purchase { at },
            CurrentAction::WorkTravel { target, remaining } => {
                if remaining > 1 {
                    Transition::Continue(CurrentAction::WorkTravel {
                        target,
                        remaining: remaining - 1,
                    })
                } else {
                    Transition::Arrive {
                        at: target,
                        next: CurrentAction::Work {
                            at: target,
                            remaining: self.tables.work_ticks,
                        },
                    }
                }
            }
            CurrentAction::Work { at, remaining } => {
                // Working satisfies the data-defined need a little; the
                // wage comes from payroll, never from the act
                // (ADR 0008 §2).
                let next = if remaining > 1 {
                    Some(CurrentAction::Work {
                        at,
                        remaining: remaining - 1,
                    })
                } else {
                    None
                };
                Transition::GainAndContinue {
                    need_index: self.tables.work_need,
                    amount: self.tables.work_need_per_tick,
                    next,
                }
            }
            CurrentAction::SchoolTravel { target, remaining } => {
                if remaining > 1 {
                    Transition::Continue(CurrentAction::SchoolTravel {
                        target,
                        remaining: remaining - 1,
                    })
                } else {
                    Transition::Arrive {
                        at: target,
                        next: CurrentAction::Attend {
                            at: target,
                            remaining: self.tables.school_attend_ticks,
                        },
                    }
                }
            }
            CurrentAction::Attend { at, remaining } => {
                if remaining > 1 {
                    Transition::Continue(CurrentAction::Attend {
                        at,
                        remaining: remaining - 1,
                    })
                } else {
                    Transition::Graduate { at }
                }
            }
            CurrentAction::Consume {
                at,
                need_index,
                remaining,
            } => {
                if remaining > 1 {
                    Transition::Continue(CurrentAction::Consume {
                        at,
                        need_index,
                        remaining: remaining - 1,
                    })
                } else {
                    Transition::Finish
                }
            }
            CurrentAction::Perform {
                at,
                need_index,
                remaining,
            } => {
                let kind = world
                    .get::<Location>(at)?
                    .map(|location| location.kind)
                    .ok_or(EcsError::InternalCorruption(
                        "performing at an entity that is not a location",
                    ))?;
                let mut rate = self.tables.satisfier_rate(kind, need_index).unwrap_or(0);
                // Bonds feed utility (Phase 7, ADR 0010 §2): satisfying
                // the drift need among present friends gains more —
                // per friend, weight/1000 × strength/1000 of the rate.
                if need_index == self.tables.social.drift_need && rate > 0 {
                    let mut bonus: i64 = 0;
                    if let Some(relationships) =
                        world.get::<core_ecs::sim_interface::Relationships>(entity)?
                    {
                        for edge in &relationships.edges {
                            if matches!(edge.kind, core_ecs::sim_interface::RelKind::Friend)
                                && world.get::<Position>(edge.other)?.map(|p| p.at) == Some(at)
                            {
                                bonus += rate
                                    .saturating_mul(self.tables.social.social_bond_weight_per_mille)
                                    .saturating_mul(i64::from(edge.strength_per_mille))
                                    / 1_000_000;
                            }
                        }
                    }
                    rate = rate.saturating_add(bonus);
                }
                let level = world
                    .get::<Needs>(entity)?
                    .and_then(|needs| needs.levels.get(need_index as usize).copied())
                    .map(|level| level.raw())
                    .unwrap_or(0);
                let will_fill = level + rate >= NEED_MAX;
                let next = if will_fill || remaining <= 1 || rate <= 0 {
                    None
                } else {
                    Some(CurrentAction::Perform {
                        at,
                        need_index,
                        remaining: remaining - 1,
                    })
                };
                Transition::GainAndContinue {
                    need_index,
                    amount: rate,
                    next,
                }
            }
        })
    }
}

impl System for ActSystem {
    fn name(&self) -> &'static str {
        "ai.act"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        // Pass 1 (immutable): resolve every actor's transition, entity order.
        let mut transitions: Vec<(Entity, Transition)> = Vec::new();
        {
            let actions: Vec<(Entity, CurrentAction)> = world
                .iter::<CurrentAction>()?
                .map(|(entity, action)| (entity, *action))
                .collect();
            for (entity, action) in actions {
                transitions.push((entity, self.transition(world, entity, action)?));
            }
        }

        // Pass 2: apply, same order.
        for (entity, transition) in transitions {
            match transition {
                Transition::Continue(next) => {
                    world.insert(entity, next)?;
                }
                Transition::Arrive { at, next } => {
                    world.insert(entity, Position { at })?;
                    world.insert(entity, next)?;
                }
                Transition::GainAndContinue {
                    need_index,
                    amount,
                    next,
                } => {
                    if let Some(needs) = world.get_mut::<Needs>(entity)?
                        && let Some(level) = needs.levels.get_mut(need_index as usize)
                    {
                        *level = level.gain(amount);
                    }
                    match next {
                        Some(action) => {
                            world.insert(entity, action)?;
                        }
                        None => {
                            world.remove::<CurrentAction>(entity)?;
                        }
                    }
                }
                Transition::Finish => {
                    world.remove::<CurrentAction>(entity)?;
                }
                Transition::Purchase { at } => {
                    execute_purchase(
                        world,
                        entity,
                        at,
                        self.tables.sales_tax_per_mille,
                        self.tables.social.belief_cap as usize,
                    )?;
                }
                Transition::Graduate { at } => {
                    // The skill bump (lazy row: migrated citizens learn
                    // on their first attendance) and the fact.
                    world.remove::<CurrentAction>(entity)?;
                    let skill = self.tables.school_taught_skill as usize;
                    let gain = self.tables.school_gain_per_mille;
                    let mut skills = world
                        .get::<core_ecs::sim_interface::Skills>(entity)?
                        .cloned()
                        .unwrap_or(core_ecs::sim_interface::Skills {
                            levels: vec![0; self.tables.skill_count as usize],
                        });
                    if skills.levels.len() < self.tables.skill_count as usize {
                        skills.levels.resize(self.tables.skill_count as usize, 0);
                    }
                    if let Some(level) = skills.levels.get_mut(skill) {
                        *level = level.saturating_add(gain).min(1000);
                    }
                    let new_level = skills.levels.get(skill).copied().unwrap_or(0);
                    world.insert(entity, skills)?;
                    world.emit(&core_ecs::sim_interface::SchoolAttended {
                        pupil: entity,
                        skill: self.tables.school_taught_skill,
                        new_level,
                    })?;
                    let _ = at;
                }
            }
        }
        Ok(())
    }
}

/// Executes one retail purchase atomically (ADR 0007 §§4, 6), or aborts
/// it cleanly: if the offer is gone, the shelf is empty, or the buyer can
/// no longer pay (state moved during travel), the action is simply
/// removed — a fresh decision follows next tick, and NOTHING was
/// transferred. On success, wallets, the seller's books and stock, the
/// conservation counters, the buyer's need, and the event all move in
/// this one call.
fn execute_purchase(
    world: &mut World,
    buyer: Entity,
    seller: Entity,
    sales_tax_per_mille: i64,
    belief_cap: usize,
) -> Result<(), EcsError> {
    match purchase_unit(world, buyer, seller, sales_tax_per_mille, belief_cap)? {
        Some(offer) => {
            world.insert(
                buyer,
                CurrentAction::Consume {
                    at: seller,
                    need_index: offer.need_index,
                    remaining: offer.use_ticks,
                },
            )?;
        }
        None => {
            world.remove::<CurrentAction>(buyer)?;
        }
    }
    Ok(())
}

/// The tier-agnostic core of one retail purchase (Phase 8, ADR 0011 §2:
/// Tier B/C demand goes through the SAME till): checks offer, stock,
/// and cash; on success moves money, tax, stock, counters, the buyer's
/// need, and the belief, and emits the fact — returning the offer.
/// `None` means nothing happened (no offer, empty shelf, or a short
/// wallet). No action-state side effects — the embodied wrapper above
/// owns those.
pub(crate) fn purchase_unit(
    world: &mut World,
    buyer: Entity,
    seller: Entity,
    sales_tax_per_mille: i64,
    belief_cap: usize,
) -> Result<Option<RetailOffer>, EcsError> {
    let offer = match world.get::<RetailOffer>(seller)? {
        Some(offer) => *offer,
        None => return Ok(None),
    };
    let stock = world
        .get::<Inventory>(seller)?
        .map(|inventory| inventory.stock(offer.good))
        .unwrap_or(0);
    let cash = world
        .get::<Wallet>(buyer)?
        .map(|wallet| wallet.cash.mills())
        .unwrap_or(0);
    let price = offer.unit_price;
    if stock < 1 || cash < price.mills() {
        return Ok(None);
    }
    // A retail offer only exists in worlds seeded with a conservation
    // ledger; selling without one would be uncounted consumption.
    let ledger = world
        .iter::<EconCounters>()?
        .next()
        .map(|(entity, _)| entity)
        .ok_or_else(|| {
            EcsError::InvariantViolation(
                "a retail purchase ran in a world without an EconCounters ledger".to_owned(),
            )
        })?;
    let missing = |leg: &str, entity: Entity| {
        EcsError::InvariantViolation(format!(
            "purchase leg missing: entity #{} has no {leg}",
            entity.index()
        ))
    };

    // Sales tax (Phase 6, ADR 0009 §4): split out of the posted price at
    // the till — the buyer pays the posted price, the seller keeps the
    // net, the treasury takes the tax, all in this one atomic call.
    // Worlds without a treasury (migrated) tax nothing, honestly.
    let treasury = world
        .iter::<core_ecs::sim_interface::TreasuryBook>()?
        .next()
        .map(|(entity, _)| entity);
    let tax = match treasury {
        Some(_) => core_types::Money::from_mills(
            price.mills().checked_mul(sales_tax_per_mille).ok_or(
                core_ecs::EcsError::Arithmetic(core_types::ArithmeticError::Overflow {
                    op: "sales tax",
                }),
            )? / 1000,
        ),
        None => core_types::Money::ZERO,
    };
    let net = price.try_sub(tax)?;

    // The atomic transaction (ADR 0007 §§4, 8b): every leg structurally
    // required — a missing component is a typed error at the fault site,
    // never a silently skipped half-transfer. Money…
    let wallet = world
        .get_mut::<Wallet>(buyer)?
        .ok_or_else(|| missing("buyer wallet", buyer))?;
    wallet.cash = wallet.cash.try_sub(price)?;
    let wallet = world
        .get_mut::<Wallet>(seller)?
        .ok_or_else(|| missing("seller wallet", seller))?;
    wallet.cash = wallet.cash.try_add(net)?;
    let books = world
        .get_mut::<FirmBooks>(seller)?
        .ok_or_else(|| missing("seller books", seller))?;
    books.revenue = books.revenue.try_add(net)?;
    if let Some(treasury) = treasury
        && tax > core_types::Money::ZERO
    {
        let wallet = world
            .get_mut::<Wallet>(treasury)?
            .ok_or_else(|| missing("treasury wallet", treasury))?;
        wallet.cash = wallet.cash.try_add(tax)?;
        let books = world
            .get_mut::<FirmBooks>(treasury)?
            .ok_or_else(|| missing("treasury books", treasury))?;
        books.revenue = books.revenue.try_add(tax)?;
        if let Some(book) = world.get_mut::<core_ecs::sim_interface::TreasuryBook>(treasury)? {
            book.sales_tax_received = book.sales_tax_received.try_add(tax)?;
        }
        world.emit(&core_ecs::sim_interface::TaxCollected {
            payer: buyer,
            amount: tax,
            kind: core_ecs::sim_interface::TaxKind::Sales,
        })?;
    }
    // …goods (one unit off the shelf, counted as citizen consumption)…
    let stock = world
        .get_mut::<Inventory>(seller)?
        .and_then(|inventory| inventory.quantities.get_mut(offer.good as usize))
        .ok_or_else(|| missing("shelf slot for the offered good", seller))?;
    *stock -= 1;
    let consumed = world
        .get_mut::<EconCounters>(ledger)?
        .and_then(|counters| counters.consumed_by_citizens.get_mut(offer.good as usize))
        .ok_or_else(|| missing("counter slot for the offered good", ledger))?;
    *consumed += 1;
    // …the satisfaction the unit buys (clamped exact gain)…
    let level = world
        .get_mut::<Needs>(buyer)?
        .and_then(|needs| needs.levels.get_mut(offer.need_index as usize))
        .ok_or_else(|| missing("need level for the offered need", buyer))?;
    *level = level.gain(offer.gain_per_unit);
    // …the corrected belief (Phase 7, ADR 0010 §3: experience
    // overwrites — you saw the price tag)…
    let mut beliefs = world
        .get::<core_ecs::sim_interface::Beliefs>(buyer)?
        .cloned()
        .unwrap_or_default();
    crate::systems_social::experience_price(&mut beliefs, seller, price, belief_cap);
    world.insert(buyer, beliefs)?;
    // …and the fact.
    world.emit(&GoodsPurchased {
        buyer,
        seller,
        good: offer.good,
        quantity: 1,
        total: price,
    })?;
    Ok(Some(offer))
}

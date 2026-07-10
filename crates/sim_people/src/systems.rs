//! The Phase 2 people systems: needs decay (hour rate) and mortality
//! (day rate). Explicit order and rates are wired by the application's
//! schedule builder (SPEC §6).

use core_ecs::sim_interface::{
    BankBook, EconCounters, Ownership, TreasuryBook, Wallet, WorkingAge,
};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use core_rng::RngCore;
use core_types::Money;

use crate::components::{Household, HouseholdMember, Identity, Needs, Personality};
use crate::config::MortalityConfig;
use crate::config::PeopleConfig;
use crate::events::{DeathCause, PersonDied};

/// RNG stream for mortality draws (one per citizen per day).
pub const MORTALITY_STREAM: &str = "people.mortality";

// Mortality draws are exact integer comparisons against a uniform draw in
// [0, 1e9) — the per-billion unit of the data-defined curve (ADR 0005 §4),
// an algorithmic constant, not a tunable.
const PER_BILLION: u64 = 1_000_000_000;

/// Hour-rate system: every citizen's needs decay by the data-defined
/// per-hour amounts, clamped at 0 (ADR 0005 §4). No satisfaction sources
/// exist until Phase 3 — "no AI yet" means people simply get hungrier.
///
/// Invariants: holds only immutable data-derived config; iterates in
/// entity-index order; exact integer arithmetic.
pub struct NeedsDecaySystem {
    decay_per_hour: Vec<i64>,
}

impl NeedsDecaySystem {
    /// Builds from the validated needs config (decay order = need order).
    pub fn new(decay_per_hour: Vec<i64>) -> Self {
        NeedsDecaySystem { decay_per_hour }
    }
}

impl System for NeedsDecaySystem {
    fn name(&self) -> &'static str {
        "people.needs_decay"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let expected = self.decay_per_hour.len();
        for (entity, needs) in world.iter_mut::<Needs>()? {
            // Guard, not a silent zip-truncation: a saved vector that does
            // not match the loaded data definitions is corrupt state
            // (validated at load by `validate_town`; enforced here too).
            if needs.levels.len() != expected {
                return Err(EcsError::ComponentBlobMismatch(format!(
                    "citizen #{} has {} need levels but data defines {expected}",
                    entity.index(),
                    needs.levels.len()
                )));
            }
            for (level, decay) in needs.levels.iter_mut().zip(&self.decay_per_hour) {
                *level = level.decay(*decay);
            }
        }
        Ok(())
    }
}

/// Validates a (freshly built or loaded) town against the loaded data
/// definitions: every citizen's need and trait vectors must match the data
/// order/length they will be interpreted under. Call after genesis and
/// after loading a save (SPEC §8: a mismatch is a precise error, never a
/// silent reinterpretation).
pub fn validate_town(world: &World, config: &PeopleConfig) -> Result<(), EcsError> {
    let needs_len = config.needs.needs.len();
    for (entity, needs) in world.iter::<Needs>()? {
        if needs.levels.len() != needs_len {
            return Err(EcsError::ComponentBlobMismatch(format!(
                "citizen #{} has {} need levels but data defines {needs_len} \
                 (save/data mismatch)",
                entity.index(),
                needs.levels.len()
            )));
        }
    }
    let traits_len = config.traits.traits.len();
    for (entity, personality) in world.iter::<Personality>()? {
        if personality.weights.len() != traits_len {
            return Err(EcsError::ComponentBlobMismatch(format!(
                "citizen #{} has {} trait weights but data defines {traits_len} \
                 (save/data mismatch)",
                entity.index(),
                personality.weights.len()
            )));
        }
    }
    Ok(())
}

/// Day-rate system (Phase 5, ADR 0008 §2): stamps [`WorkingAge`]
/// (shared marker) on citizens who crossed the data-defined threshold —
/// the labor market's eligibility signal, maintained where age lives.
/// Age only grows, so the marker is never removed.
pub struct WorkingAgeSystem {
    min_working_age_years: u32,
    ticks_per_year: u64,
}

impl WorkingAgeSystem {
    /// Builds from the labor threshold and the calendar's year length.
    pub fn new(min_working_age_years: u32, ticks_per_year: u64) -> Self {
        WorkingAgeSystem {
            min_working_age_years,
            ticks_per_year,
        }
    }
}

impl System for WorkingAgeSystem {
    fn name(&self) -> &'static str {
        "people.working_age"
    }

    fn run(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        // Pass 1 (immutable): find citizens of age without the marker.
        let mut newly_of_age: Vec<Entity> = Vec::new();
        for (entity, identity) in world.iter::<Identity>()? {
            if identity.age_years(ctx.tick, self.ticks_per_year) >= self.min_working_age_years
                && world.get::<WorkingAge>(entity)?.is_none()
            {
                newly_of_age.push(entity);
            }
        }
        // Pass 2: stamp — and the new adult leaves the nest (ADR 0009
        // §3: new adults are the rental market's demand margin). A
        // citizen coming of age in a home they do NOT own gives up the
        // family Residence; tomorrow's rental clearing sees them
        // homeless and they bid from their savings. Owners stay put,
        // and pre-Phase 6 worlds (no Ownership rows) are untouched —
        // migrations never invent state.
        for entity in newly_of_age {
            world.insert(entity, WorkingAge)?;
            let home = world
                .get::<core_ecs::sim_interface::Residence>(entity)?
                .map(|residence| residence.home);
            if let Some(home) = home
                && let Some(ownership) = world.get::<Ownership>(home)?
                && ownership.owner != entity
            {
                world.remove::<core_ecs::sim_interface::Residence>(entity)?;
            }
        }
        Ok(())
    }
}

/// Day-rate system: each citizen faces their age band's daily death
/// chance (deterministic curve from data + one draw from
/// [`MORTALITY_STREAM`] per citizen per day, ADR 0005 §4).
///
/// On death: emits [`PersonDied`], removes the citizen from their
/// household (despawning a household that empties), and despawns the
/// citizen — all structural changes through the command buffer (SPEC §6).
pub struct MortalitySystem {
    curve: MortalityConfig,
    ticks_per_year: u64,
}

impl MortalitySystem {
    /// Builds from the validated mortality curve and the calendar's year
    /// length in ticks.
    pub fn new(curve: MortalityConfig, ticks_per_year: u64) -> Self {
        MortalitySystem {
            curve,
            ticks_per_year,
        }
    }
}

impl System for MortalitySystem {
    fn name(&self) -> &'static str {
        "people.mortality"
    }

    fn run(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
        cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        // Pass 1 (immutable): collect ages in entity-index order.
        let candidates: Vec<(core_ecs::Entity, u32)> = world
            .iter::<Identity>()?
            .map(|(entity, identity)| (entity, identity.age_years(ctx.tick, self.ticks_per_year)))
            .collect();

        // Pass 2: one draw per citizen, same order (the draw count and
        // order are functions of deterministic state).
        for (entity, age_years) in candidates {
            let chance = u64::from(self.curve.per_day_chance(age_years));
            let draw = world.rng(MORTALITY_STREAM).next_u64() % PER_BILLION;
            if draw < chance {
                world.emit(&PersonDied {
                    person: entity,
                    cause: DeathCause::OldAge,
                })?;
                let household = world
                    .get::<HouseholdMember>(entity)?
                    .map(|member| member.household);
                // Household bookkeeping and the estate settle in one
                // deferred step, immediately before the despawn in the
                // same buffer (the deceased is still alive when it runs).
                cmd.run(move |w| settle_death(w, entity, household));
                cmd.despawn(entity);
            }
        }
        Ok(())
    }
}

/// Removes the deceased from their household (despawning it if emptied)
/// and passes their estate on (ADR 0007 §8a): to the lowest-indexed
/// surviving household member, else to the ledger entity's escheat
/// wallet. Money is never destroyed — `Σ wallets == issued` holds across
/// deaths. Pre-economy worlds have no wallets: nothing to settle.
fn settle_death(w: &mut World, entity: Entity, household: Option<Entity>) -> Result<(), EcsError> {
    let mut heir: Option<Entity> = None;
    if let Some(household) = household
        && let Some(h) = w.get_mut::<Household>(household)?
    {
        h.members.retain(|m| *m != entity);
        heir = h.members.first().copied();
        if h.members.is_empty() {
            w.despawn(household)?;
        }
    }
    // Debts settle BEFORE the estate distributes (ADR 0009 §3): each of
    // the deceased's loans is repaid from wallet cash, then from the
    // deposit row (already vault-side — an internal netting); a residual
    // is written off against bank equity, with a mortgage's collateral
    // home passing to the bank as its recovery in kind. Heirs inherit
    // net of debts — the dead take nothing, and owe nothing silently.
    if let Some((bank, _)) = w.iter::<BankBook>()?.next() {
        let loans: Vec<core_ecs::sim_interface::Loan> = w
            .get::<BankBook>(bank)?
            .map(|book| {
                book.loans
                    .iter()
                    .filter(|loan| loan.borrower == entity)
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        for loan in loans {
            let mut residual = loan.principal;
            // Leg 1: wallet cash -> bank wallet.
            let cash = w
                .get::<Wallet>(entity)?
                .map(|wallet| wallet.cash)
                .unwrap_or(Money::ZERO);
            let from_cash = Money::from_mills(residual.mills().min(cash.mills()).max(0));
            if from_cash > Money::ZERO {
                if let Some(wallet) = w.get_mut::<Wallet>(entity)? {
                    wallet.cash = wallet.cash.try_sub(from_cash)?;
                }
                if let Some(wallet) = w.get_mut::<Wallet>(bank)? {
                    wallet.cash = wallet.cash.try_add(from_cash)?;
                }
                residual = residual.try_sub(from_cash)?;
            }
            // Leg 2: the deposit row (the mills already sit in the
            // vault -- row and outstanding shrink together).
            if residual > Money::ZERO
                && let Some(book) = w.get_mut::<BankBook>(bank)?
                && let Some(row) = book.deposits.iter_mut().find(|(owner, _)| *owner == entity)
            {
                let from_row = Money::from_mills(residual.mills().min(row.1.mills()).max(0));
                row.1 = row.1.try_sub(from_row)?;
                residual = residual.try_sub(from_row)?;
            }
            // Leg 3: the residual -- collateral to the bank, loss to
            // equity (the vault identity holds through all three legs).
            if residual > Money::ZERO {
                if let Some(home) = loan.collateral
                    && w.is_alive(home)
                    && w.get::<Ownership>(home)?.map(|o| o.owner) == Some(entity)
                    && let Some(ownership) = w.get_mut::<Ownership>(home)?
                {
                    ownership.owner = bank;
                }
                if let Some(book) = w.get_mut::<BankBook>(bank)? {
                    book.equity = book.equity.try_sub(residual)?;
                }
                w.emit(&core_ecs::sim_interface::LoanDefaulted {
                    borrower: entity,
                    written_off: residual,
                })?;
            }
        }
        if let Some(book) = w.get_mut::<BankBook>(bank)? {
            book.loans.retain(|loan| loan.borrower != entity);
        }
    }

    let estate = w.get::<Wallet>(entity)?.map(|wallet| wallet.cash);
    if let Some(estate) = estate
        && estate != Money::ZERO
    {
        let recipient = match heir {
            Some(heir) => Some(heir),
            // Unclaimed: the ledger's escheat wallet (ADR 0007 §8a).
            None => w.iter::<EconCounters>()?.next().map(|(ledger, _)| ledger),
        };
        if let Some(recipient) = recipient {
            match w.get_mut::<Wallet>(recipient)? {
                Some(wallet) => wallet.cash = wallet.cash.try_add(estate)?,
                None => {
                    w.insert(recipient, Wallet { cash: estate })?;
                }
            }
            if let Some(wallet) = w.get_mut::<Wallet>(entity)? {
                wallet.cash = Money::ZERO;
            }
        }
    }

    // Phase 6 estate legs (ADR 0009 §3): the deceased's deposit row and
    // owned homes pass to the heir — or to the treasury (public housing /
    // unclaimed funds) when the household died out.
    let treasury = w.iter::<TreasuryBook>()?.next().map(|(entity, _)| entity);
    if let Some((bank, _)) = w.iter::<BankBook>()?.next()
        && let Some(recipient) = heir.or(treasury)
        && let Some(book) = w.get_mut::<BankBook>(bank)?
    {
        let balance = book
            .deposits
            .iter()
            .position(|(owner, _)| *owner == entity)
            .map(|index| book.deposits.remove(index).1);
        if let Some(balance) = balance
            && balance > Money::ZERO
        {
            match book
                .deposits
                .iter_mut()
                .find(|(owner, _)| *owner == recipient)
            {
                Some(row) => row.1 = row.1.try_add(balance)?,
                None => {
                    book.deposits.push((recipient, balance));
                    book.deposits.sort_by_key(|(owner, _)| owner.index());
                }
            }
        }
    }
    let owned_homes: Vec<Entity> = w
        .iter::<Ownership>()?
        .filter(|(_, ownership)| ownership.owner == entity)
        .map(|(home, _)| home)
        .collect();
    for home in owned_homes {
        if let Some(recipient) = heir.or(treasury)
            && let Some(ownership) = w.get_mut::<Ownership>(home)?
        {
            ownership.owner = recipient;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Sex;
    use core_types::{CalendarTime, Seed, Ticks};

    /// ADR 0008 §2: the marker lands exactly on the birthday crossing the
    /// threshold — not a day before, never only at genesis.
    #[test]
    fn working_age_is_stamped_on_the_crossing_birthday() {
        const YEAR: u64 = 120 * 1440;
        let mut world = World::new(Seed::new(1), 16);
        world.register::<Identity>().expect("register");
        world.register::<WorkingAge>().expect("register");
        world
            .register::<core_ecs::sim_interface::Residence>()
            .expect("register");
        world.register::<Ownership>().expect("register");
        let child = world.spawn();
        // Born exactly 16 years before tick YEAR: at tick 0 they are 15,
        // at tick YEAR they turn 16.
        world
            .insert(
                child,
                Identity {
                    given_name: "Twig".into(),
                    family_name: "Ashdown".into(),
                    sex: Sex::Female,
                    birth_tick: -((15 * YEAR) as i64),
                },
            )
            .expect("insert");

        let mut system = WorkingAgeSystem::new(16, YEAR);
        let mut cmd = CommandBuffer::new();
        let at = |tick: u64| TickContext {
            tick: Ticks::new(tick),
            time: CalendarTime::START,
        };
        system.run(&mut world, &at(0), &mut cmd).expect("run");
        assert!(
            world.get::<WorkingAge>(child).expect("get").is_none(),
            "a 15-year-old stays outside the labor force"
        );
        system
            .run(&mut world, &at(YEAR - 1440), &mut cmd)
            .expect("run");
        assert!(
            world.get::<WorkingAge>(child).expect("get").is_none(),
            "the day before the birthday still does not count"
        );
        system.run(&mut world, &at(YEAR), &mut cmd).expect("run");
        assert!(
            world.get::<WorkingAge>(child).expect("get").is_some(),
            "the 16th birthday joins the labor force"
        );
    }

    /// ADR 0009 §3: death settles debts BEFORE the estate distributes —
    /// cash and the deposit row repay the loan, the collateral home
    /// covers (in kind) what they cannot, only the residual hits bank
    /// equity, and the heir inherits what remains (nothing silently).
    #[test]
    fn estates_settle_debts_before_inheritance() {
        use core_ecs::sim_interface::{BankBook, EconCounters, Loan, Ownership, Wallet};
        use core_types::Money;

        let mut world = World::new(Seed::new(3), 16);
        world.register::<Wallet>().expect("register");
        world.register::<BankBook>().expect("register");
        world.register::<Ownership>().expect("register");
        world
            .register::<core_ecs::sim_interface::TreasuryBook>()
            .expect("register");
        world.register::<EconCounters>().expect("register");
        world.register::<Household>().expect("register");
        world
            .register_event::<core_ecs::sim_interface::LoanDefaulted>()
            .expect("register");

        let bank = world.spawn();
        let deceased = world.spawn();
        let heir = world.spawn();
        let home = world.spawn();
        let second_home = world.spawn();
        world
            .insert(
                bank,
                Wallet {
                    // equity 10_000 + deposits 400 − outstanding 1_000:
                    // the identity holds at the start.
                    cash: Money::from_mills(9_400),
                },
            )
            .expect("insert");
        world
            .insert(
                bank,
                BankBook {
                    equity: Money::from_mills(10_000),
                    policy_rate_per_million_daily: 800,
                    last_price_index_milli: 0,
                    deposits: vec![(deceased, Money::from_mills(400))],
                    loans: vec![Loan {
                        borrower: deceased,
                        principal: Money::from_mills(1_000),
                        rate_per_million_daily: 800,
                        day_payment: Money::from_mills(20),
                        collateral: Some(home),
                    }],
                    interest_received: Money::ZERO,
                    deposit_interest_paid: Money::ZERO,
                    granted: vec![(deceased, Money::from_mills(1_000))],
                },
            )
            .expect("insert");
        world
            .insert(
                deceased,
                Wallet {
                    cash: Money::from_mills(300),
                },
            )
            .expect("insert");
        world
            .insert(heir, Wallet { cash: Money::ZERO })
            .expect("insert");
        world
            .insert(home, Ownership { owner: deceased })
            .expect("insert");
        world
            .insert(second_home, Ownership { owner: deceased })
            .expect("insert");
        let household = world.spawn();
        world
            .insert(
                household,
                Household {
                    members: vec![deceased, heir],
                },
            )
            .expect("insert");

        settle_death(&mut world, deceased, Some(household)).expect("settle");

        // Debt legs: 300 cash + 400 row repay 700; the 300 residual is
        // covered in kind — the collateral passes to the BANK — and
        // written off against equity.
        let book = world
            .iter::<BankBook>()
            .expect("query")
            .next()
            .map(|(_, book)| book.clone())
            .expect("bank");
        assert!(book.loans.is_empty(), "the debt died with the debtor");
        assert_eq!(
            world
                .get::<Wallet>(bank)
                .expect("query")
                .expect("wallet")
                .cash,
            Money::from_mills(9_700),
            "cash repayment reached the vault"
        );
        assert_eq!(
            book.equity,
            Money::from_mills(9_700),
            "only the unrecovered residual hit equity"
        );
        assert_eq!(
            world
                .get::<Ownership>(home)
                .expect("query")
                .expect("owned")
                .owner,
            bank,
            "the collateral home is the bank's recovery in kind"
        );
        // The vault identity: wallet 9_300 == deposits 0 + equity 9_700
        // − outstanding 0... the deceased's row was consumed by the debt
        // and the empty row passed to nobody.
        let deposits: i64 = book.deposits.iter().map(|(_, b)| b.mills()).sum();
        assert_eq!(deposits, 0, "the row was consumed by the debt");
        assert_eq!(
            9_700,
            deposits + book.equity.mills(),
            "the vault identity holds through the whole settlement"
        );
        // The estate: no cash left; the UNENCUMBERED home passes to the
        // heir.
        assert_eq!(
            world
                .get::<Ownership>(second_home)
                .expect("query")
                .expect("owned")
                .owner,
            heir,
            "the free-and-clear home is inherited"
        );
        assert_eq!(
            world
                .get::<Wallet>(heir)
                .expect("query")
                .expect("wallet")
                .cash,
            Money::ZERO,
            "the debt consumed the liquid estate — the heir gets no cash"
        );
    }
}

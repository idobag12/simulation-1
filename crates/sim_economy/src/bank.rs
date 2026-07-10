//! The bank (Phase 6, ADR 0009 §§1–2): a full-reserve vault. Deposits
//! and loans move real mills between real wallets; the book rows mirror
//! them so the auditor can enforce
//! `bank wallet == Σ deposits + equity − Σ outstanding`.
//! The policy rate is a Taylor-style rule on the MEASURED price index —
//! the one macro lever, and it is a modeled actor (SPEC §12).

use core_ecs::sim_interface::{
    BankBook, BorrowerStatus, Employment, FirmBooks, Loan, LoanDefaulted, LoanGranted, Needs,
    Wallet,
};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use core_types::calendar::TICKS_PER_DAY;
use core_types::{ArithmeticError, Money};

use crate::components::Firm;
use crate::config::EconTables;

fn overflow(op: &'static str) -> EcsError {
    EcsError::Arithmetic(ArithmeticError::Overflow { op })
}

/// The bank entity, if this world has one (worlds migrated from pre-v7
/// saves honestly do not — a bank needs seeded equity, which migrations
/// must not invent; ADR 0009 §6).
pub fn bank_entity(world: &World) -> Result<Option<Entity>, EcsError> {
    Ok(world.iter::<BankBook>()?.next().map(|(entity, _)| entity))
}

/// Day-rate system: the policy rule, then loan servicing, then loan
/// origination, then citizen deposits/withdrawals and deposit interest —
/// every leg a real transfer, entity order throughout.
pub struct BankSystem {
    tables: EconTables,
}

impl BankSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: EconTables) -> Self {
        BankSystem { tables }
    }

    /// The Taylor-style rule (ADR 0009 §2): measure the retail basket's
    /// average posted price; respond to its change since last period.
    fn run_policy(&self, world: &mut World, bank: Entity, day: u64) -> Result<(), EcsError> {
        let config = &self.tables.money.bank;
        if config.index_period_days == 0 || !day.is_multiple_of(config.index_period_days) {
            return Ok(());
        }
        // The index: average posted price (milli-units) across retail
        // offers — the goods citizens actually buy.
        let mut sum: i64 = 0;
        let mut count: i64 = 0;
        for (_, offer) in world.iter::<core_ecs::sim_interface::RetailOffer>()? {
            sum = sum
                .checked_add(offer.unit_price.mills())
                .ok_or_else(|| overflow("index sum"))?;
            count += 1;
        }
        if count == 0 {
            return Ok(());
        }
        let index_milli = sum
            .checked_mul(1000)
            .ok_or_else(|| overflow("index scale"))?
            / count;

        let Some(book) = world.get_mut::<BankBook>(bank)? else {
            return Ok(());
        };
        let last = book.last_price_index_milli;
        book.last_price_index_milli = index_milli;
        if last <= 0 {
            return Ok(()); // first observation: no inflation measurable yet
        }
        let inflation_per_mille = (index_milli - last)
            .checked_mul(1000)
            .ok_or_else(|| overflow("inflation"))?
            / last;
        let gap = inflation_per_mille - config.policy_target_inflation_per_mille;
        let rate = config
            .policy_neutral_per_million_daily
            .checked_add(
                config
                    .policy_sensitivity_per_million
                    .checked_mul(gap)
                    .ok_or_else(|| overflow("policy response"))?
                    / 1000,
            )
            .ok_or_else(|| overflow("policy rate"))?
            .clamp(
                config.policy_min_per_million_daily,
                config.policy_max_per_million_daily,
            );
        book.policy_rate_per_million_daily = rate;
        Ok(())
    }

    /// Services every outstanding loan: pay (interest first) or default
    /// (write-off against equity + cooldown). Loans process in grant
    /// order; all reads are live.
    fn service_loans(&self, world: &mut World, bank: Entity, day: u64) -> Result<(), EcsError> {
        let loans: Vec<Loan> = world
            .get::<BankBook>(bank)?
            .map(|book| book.loans.clone())
            .unwrap_or_default();
        let mut survivors: Vec<Loan> = Vec::with_capacity(loans.len());
        for mut loan in loans {
            let interest = Money::from_mills(
                loan.principal
                    .mills()
                    .checked_mul(loan.rate_per_million_daily)
                    .ok_or_else(|| overflow("loan interest"))?
                    / 1_000_000,
            );
            let principal_part = Money::from_mills(
                (loan.day_payment.mills() - interest.mills()).clamp(0, loan.principal.mills()),
            );
            let due = interest.try_add(principal_part)?;
            let cash = world
                .get::<Wallet>(loan.borrower)?
                .map(|wallet| wallet.cash)
                .unwrap_or(Money::ZERO);
            // Auto-debit: when the wallet is short, the bank draws the
            // payment from the borrower's own deposit row first (a
            // mortgage holder's wealth sits in the vault — defaulting a
            // depositor who can pay would be a lie).
            if cash < due && world.is_alive(loan.borrower) {
                let balance = deposit_balance(world, bank, loan.borrower)?;
                let shortfall = due.try_sub(cash)?;
                if balance >= shortfall {
                    vault_withdraw(world, bank, loan.borrower, shortfall)?;
                }
            }
            let cash = world
                .get::<Wallet>(loan.borrower)?
                .map(|wallet| wallet.cash)
                .unwrap_or(Money::ZERO);
            if cash >= due && world.is_alive(loan.borrower) {
                // Pay: borrower → bank; interest feeds equity; the
                // borrower's outflow ledger records the payment.
                if let Some(wallet) = world.get_mut::<Wallet>(loan.borrower)? {
                    wallet.cash = wallet.cash.try_sub(due)?;
                }
                if let Some(wallet) = world.get_mut::<Wallet>(bank)? {
                    wallet.cash = wallet.cash.try_add(due)?;
                }
                if let Some(books) = world.get_mut::<FirmBooks>(loan.borrower)? {
                    books.expenses = books.expenses.try_add(due)?;
                }
                if let Some(book) = world.get_mut::<BankBook>(bank)? {
                    book.equity = book.equity.try_add(interest)?;
                    book.interest_received = book.interest_received.try_add(interest)?;
                }
                loan.principal = loan.principal.try_sub(principal_part)?;
                if loan.principal > Money::ZERO {
                    survivors.push(loan);
                }
            } else {
                // Default: the cash already left the vault when the loan
                // was granted; the write-off books the loss to equity so
                // the vault identity keeps holding (ADR 0009 §2).
                if let Some(book) = world.get_mut::<BankBook>(bank)? {
                    book.equity = book.equity.try_sub(loan.principal)?;
                }
                if world.is_alive(loan.borrower) {
                    world.insert(
                        loan.borrower,
                        BorrowerStatus {
                            uncreditworthy_until_day: day
                                + self.tables.money.bank.default_cooldown_days,
                        },
                    )?;
                }
                world.emit(&LoanDefaulted {
                    borrower: loan.borrower,
                    written_off: loan.principal,
                })?;
            }
        }
        if let Some(book) = world.get_mut::<BankBook>(bank)? {
            book.loans = survivors;
        }
        Ok(())
    }

    /// Origination (ADR 0009 §§2, 5): firms short of working capital
    /// borrow a payroll multiple if the vault holds the cash and their
    /// lifetime revenue passes the serviceability screen — which is
    /// priced off the PAYMENT ("revenue ≥ data multiple of the
    /// payment"), so a higher policy rate honestly tightens credit.
    /// Builders instead take MATERIALS credit: when construction pays
    /// (the hurdle already prices the financing) but the till cannot
    /// cover the inputs, the loan is the batch's materials plus a
    /// payroll cushion — the hurdle is their serviceability screen.
    fn originate(&self, world: &mut World, bank: Entity, day: u64) -> Result<(), EcsError> {
        let config = &self.tables.money.bank;
        // Committed daily payroll per firm (entity-index keyed).
        let mut payroll: std::collections::BTreeMap<u32, i64> = std::collections::BTreeMap::new();
        for (_, employment) in world.iter::<Employment>()? {
            *payroll.entry(employment.employer.index()).or_insert(0) +=
                employment.wage_per_day.mills();
        }
        // Existing borrowers hold one loan at a time (no stacking).
        let borrowers: std::collections::BTreeSet<u32> = world
            .get::<BankBook>(bank)?
            .map(|book| {
                book.loans
                    .iter()
                    .map(|loan| loan.borrower.index())
                    .collect()
            })
            .unwrap_or_default();
        let firms: Vec<(Entity, u32)> = world
            .iter::<Firm>()?
            .map(|(entity, firm)| (entity, firm.recipe))
            .collect();
        for (firm, recipe_index) in firms {
            if borrowers.contains(&firm.index()) {
                continue;
            }
            if world
                .get::<BorrowerStatus>(firm)?
                .is_some_and(|status| status.uncreditworthy_until_day > day)
            {
                continue;
            }
            let daily = payroll.get(&firm.index()).copied().unwrap_or(0);
            let cash = world
                .get::<Wallet>(firm)?
                .map(|wallet| wallet.cash.mills())
                .unwrap_or(0);
            let builds_home = self
                .tables
                .recipes
                .get(recipe_index as usize)
                .is_some_and(|recipe| recipe.builds_home);
            let principal = if builds_home {
                // Materials credit (ADR 0009 §5): sized to buy the
                // batch's inputs and keep the crew paid meanwhile,
                // granted only when the start gate says the sale covers
                // cost + financing + margin.
                let recipe = self
                    .tables
                    .recipes
                    .get(recipe_index as usize)
                    .ok_or(EcsError::InternalCorruption(
                        "firm references a recipe outside the loaded data",
                    ))?
                    .clone();
                let materials = crate::construction::materials_cost(world, &self.tables, &recipe)?;
                let cushion = daily
                    .checked_mul(config.working_capital_floor_days)
                    .ok_or_else(|| overflow("materials cushion"))?;
                // Same switch as the gate's financing term: credit only
                // when the till cannot cover the batch — so every
                // granted loan is one the hurdle priced.
                if cash >= materials
                    || !crate::construction::construction_pays(world, &self.tables, firm, &recipe)?
                {
                    continue;
                }
                Money::from_mills(
                    materials
                        .checked_add(cushion)
                        .ok_or_else(|| overflow("materials loan"))?,
                )
            } else {
                if daily == 0 {
                    continue;
                }
                if cash
                    >= daily
                        .checked_mul(config.working_capital_floor_days)
                        .ok_or_else(|| overflow("working capital floor"))?
                {
                    continue;
                }
                Money::from_mills(
                    daily
                        .checked_mul(config.loan_payroll_multiple_per_mille)
                        .ok_or_else(|| overflow("loan size"))?
                        / 1000,
                )
            };
            if principal <= Money::ZERO {
                continue;
            }
            let rate = world
                .get::<BankBook>(bank)?
                .map(|book| book.policy_rate_per_million_daily)
                .unwrap_or(config.policy_neutral_per_million_daily)
                + config.risk_premium_per_million_daily;
            // The full obligation this loan would create: term × the
            // day payment (same formula `grant_loan` books) — the
            // screen prices the rate in, so dear money means fewer
            // grants (ADR 0009 §2, §8).
            let day_payment = (principal.mills() / config.repay_term_days.max(1))
                .checked_add(
                    principal
                        .mills()
                        .checked_mul(rate)
                        .ok_or_else(|| overflow("screen interest"))?
                        / 1_000_000,
                )
                .ok_or_else(|| overflow("screen payment"))?
                .max(1);
            let obligation = day_payment
                .checked_mul(config.repay_term_days.max(1))
                .ok_or_else(|| overflow("screen obligation"))?;
            // Builders were screened by the construction hurdle above
            // (the sale price covers cost + financing + margin); working
            // capital screens on the borrower's own revenue history.
            if !builds_home {
                let revenue = world
                    .get::<FirmBooks>(firm)?
                    .map(|books| books.revenue.mills())
                    .unwrap_or(0);
                if revenue
                    .checked_mul(config.serviceability_revenue_per_mille)
                    .ok_or_else(|| overflow("serviceability"))?
                    / 1000
                    < obligation
                {
                    continue;
                }
            }
            let bank_cash = world
                .get::<Wallet>(bank)?
                .map(|wallet| wallet.cash)
                .unwrap_or(Money::ZERO);
            if bank_cash < principal {
                continue;
            }
            grant_loan(world, bank, firm, principal, rate, config.repay_term_days)?;
        }
        Ok(())
    }

    /// Citizen float management + deposit interest (ADR 0009 §2).
    fn deposits(&self, world: &mut World, bank: Entity) -> Result<(), EcsError> {
        let float = self.tables.money.bank.target_cash_float_mills;
        // Citizens in entity order (`Needs` is the citizen signal the
        // economy may see, SPEC §4).
        let citizens: Vec<Entity> = world.iter::<Needs>()?.map(|(entity, _)| entity).collect();
        for citizen in citizens {
            let cash = world
                .get::<Wallet>(citizen)?
                .map(|wallet| wallet.cash.mills())
                .unwrap_or(0);
            if cash > float {
                vault_deposit(world, bank, citizen, Money::from_mills(cash - float))?;
            } else if cash < float {
                let balance = deposit_balance(world, bank, citizen)?;
                let need = Money::from_mills((float - cash).min(balance.mills()));
                if need > Money::ZERO {
                    vault_withdraw(world, bank, citizen, need)?;
                }
            }
        }
        // Interest: equity → deposit rows, entirely inside the vault.
        let policy = world
            .get::<BankBook>(bank)?
            .map(|book| book.policy_rate_per_million_daily)
            .unwrap_or(0);
        let rate = (policy - self.tables.money.bank.deposit_spread_per_million_daily).max(0);
        if rate == 0 {
            return Ok(());
        }
        if let Some(book) = world.get_mut::<BankBook>(bank)? {
            for row in &mut book.deposits {
                let interest = Money::from_mills(
                    row.1
                        .mills()
                        .checked_mul(rate)
                        .ok_or_else(|| overflow("deposit interest"))?
                        / 1_000_000,
                );
                if interest > Money::ZERO && book.equity >= interest {
                    row.1 = row.1.try_add(interest)?;
                    book.equity = book.equity.try_sub(interest)?;
                    book.deposit_interest_paid = book.deposit_interest_paid.try_add(interest)?;
                }
            }
        }
        Ok(())
    }
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
) -> Result<(), EcsError> {
    let day_payment = Money::from_mills(
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
    );
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
        });
    }
    world.emit(&LoanGranted {
        borrower,
        principal,
        rate_per_million_daily,
    })?;
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

impl System for BankSystem {
    fn name(&self) -> &'static str {
        "econ.bank"
    }

    fn run(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let Some(bank) = bank_entity(world)? else {
            return Ok(()); // no bank (migrated pre-v7 world; ADR 0009 §6)
        };
        let day = ctx.tick.raw() / TICKS_PER_DAY;
        self.run_policy(world, bank, day)?;
        self.service_loans(world, bank, day)?;
        self.originate(world, bank, day)?;
        self.deposits(world, bank)?;
        Ok(())
    }
}

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
        // Pass 2: stamp.
        for entity in newly_of_age {
            world.insert(entity, WorkingAge)?;
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
}

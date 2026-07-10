//! World assembly and deterministic runs with periodic hashing.

use core_ecs::{EcsError, Rate, Schedule, World};
use core_types::calendar::{SEASONS_PER_YEAR, TICKS_PER_DAY};
use core_types::{Seed, Ticks, WorldHash};
use data_defs::DataDefs;
use persistence::LoadConfig;
use sim_time::{Calendar, Simulation, TimeError};
use thiserror::Error;

use crate::fixture;

/// Errors assembling or running a simulation.
#[derive(Debug, Error)]
pub enum RunnerError {
    /// ECS/world failure.
    #[error(transparent)]
    Ecs(#[from] EcsError),
    /// Time/calendar configuration failure.
    #[error(transparent)]
    Time(#[from] TimeError),
}

/// A run's observable: `(tick, hash)` checkpoints in chronological order.
pub type HashSequence = Vec<(Ticks, WorldHash)>;

/// How to assemble a world. Tunables come separately from [`DataDefs`]
/// (loaded from `data/` by the CLI, pinned explicitly by tests).
///
/// Invariant: equal `(WorldSpec, DataDefs)` pairs assemble bit-identical
/// simulations — together they are the "same inputs" half of determinism
/// invariant 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldSpec {
    /// Master seed.
    pub seed: Seed,
    /// Whether the determinism fixture is populated and its systems
    /// scheduled (ADR 0002 §10).
    pub fixture: bool,
    /// Initial fixture population (ignored unless `fixture`).
    pub fixture_entities: u32,
    /// Initial citizen count (0 = no town; Phase 2, ADR 0005).
    pub citizens: u32,
    /// Whether the world carries an economy even with zero citizens
    /// (ADR 0007 §8b). Fresh builds derive their economy from
    /// `citizens > 0`, so constructors leave this `false`; loads set it
    /// from the saved world's ledger, because firms keep producing,
    /// trading, repricing, and spoiling after the last citizen dies —
    /// a post-extinction resume must keep their systems (and the
    /// auditor) running exactly like the uninterrupted run (SPEC §9).
    pub economy: bool,
}

impl WorldSpec {
    /// An empty world: no entities, no systems.
    pub fn empty(seed: Seed) -> Self {
        WorldSpec {
            seed,
            fixture: false,
            fixture_entities: 0,
            citizens: 0,
            economy: false,
        }
    }

    /// A fixture world with `entities` initial fixture entities.
    pub fn fixture(seed: Seed, entities: u32) -> Self {
        WorldSpec {
            seed,
            fixture: true,
            fixture_entities: entities,
            citizens: 0,
            economy: false,
        }
    }

    /// A town of `citizens` citizens (composable with the fixture).
    pub fn town(seed: Seed, citizens: u32) -> Self {
        WorldSpec {
            seed,
            fixture: false,
            fixture_entities: 0,
            citizens,
            economy: false,
        }
    }
}

/// The calendar year length in ticks for the loaded data (definitional
/// structure × the `days_per_season` tunable).
pub fn ticks_per_year(defs: &DataDefs) -> u64 {
    SEASONS_PER_YEAR * u64::from(defs.calendar.days_per_season) * TICKS_PER_DAY
}

/// The values `persistence::load_from_bytes` needs to reconstruct a world.
pub fn load_config(defs: &DataDefs) -> Result<LoadConfig, RunnerError> {
    Ok(LoadConfig {
        calendar: Calendar::new(defs.calendar.days_per_season)?,
        event_log_capacity: defs.engine.event_log_capacity as usize,
    })
}

/// Registers every component and event this application knows, in the
/// fixed order that is part of the save/hash format (ADR 0002 §6,
/// ADR 0004 §4). Fresh worlds and loaded worlds both go through this
/// exact function.
///
/// Invariant (ADR 0005 §9): this list only ever GROWS AT THE END, and any
/// growth bumps `persistence::FORMAT_VERSION` with a list-extension
/// migration. History: v2 = fixture set; v3 = + people set; v4 = + world/
/// AI set (Phase 3, ADR 0006 §8); v5 = + economy set (Phase 4,
/// ADR 0007 §9 — components AND the two economy events); v6 = + labor
/// set (Phase 5, ADR 0008 §8); v7 = + money set (Phase 6, ADR 0009 §6).
// v8 (Phase 7): + social registrations and events (ADR 0010 §6).
pub fn register_world(world: &mut World) -> Result<(), EcsError> {
    world.register::<fixture::FixtureWealth>()?;
    world.register::<fixture::FixtureTag>()?;
    world.register::<sim_people::Identity>()?;
    world.register::<sim_people::Needs>()?;
    world.register::<sim_people::Personality>()?;
    world.register::<sim_people::HouseholdMember>()?;
    world.register::<sim_people::Household>()?;
    world.register::<core_ecs::sim_interface::Position>()?;
    world.register::<core_ecs::sim_interface::Location>()?;
    world.register::<core_ecs::sim_interface::Residence>()?;
    world.register::<sim_ai::CurrentAction>()?;
    world.register::<sim_ai::DailyPlan>()?;
    world.register::<sim_ai::LastDecision>()?;
    world.register::<core_ecs::sim_interface::Wallet>()?;
    world.register::<core_ecs::sim_interface::Inventory>()?;
    world.register::<core_ecs::sim_interface::RetailOffer>()?;
    world.register::<sim_economy::Firm>()?;
    world.register::<core_ecs::sim_interface::FirmBooks>()?;
    world.register::<core_ecs::sim_interface::EconCounters>()?;
    world.register::<sim_economy::Production>()?;
    world.register::<core_ecs::sim_interface::Employment>()?;
    world.register::<core_ecs::sim_interface::LaborStats>()?;
    world.register::<core_ecs::sim_interface::WorkingAge>()?;
    world.register::<core_ecs::sim_interface::BankBook>()?;
    world.register::<core_ecs::sim_interface::TreasuryBook>()?;
    world.register::<core_ecs::sim_interface::Ownership>()?;
    world.register::<core_ecs::sim_interface::Tenancy>()?;
    world.register::<core_ecs::sim_interface::BorrowerStatus>()?;
    world.register::<core_ecs::sim_interface::HousingBook>()?;
    world.register::<core_ecs::sim_interface::Skills>()?;
    world.register::<core_ecs::sim_interface::Relationships>()?;
    world.register::<core_ecs::sim_interface::Beliefs>()?;
    world.register::<core_ecs::sim_interface::SchoolAge>()?;
    world.register_event::<fixture::FixtureChurn>()?;
    world.register_event::<fixture::FixtureAlarm>()?;
    world.register_event::<sim_people::PersonDied>()?;
    world.register_event::<core_ecs::sim_interface::GoodsPurchased>()?;
    world.register_event::<core_ecs::sim_interface::PriceChanged>()?;
    world.register_event::<core_ecs::sim_interface::Hired>()?;
    world.register_event::<core_ecs::sim_interface::Fired>()?;
    world.register_event::<core_ecs::sim_interface::LoanGranted>()?;
    world.register_event::<core_ecs::sim_interface::LoanDefaulted>()?;
    world.register_event::<core_ecs::sim_interface::TenancyStarted>()?;
    world.register_event::<core_ecs::sim_interface::HomeSold>()?;
    world.register_event::<core_ecs::sim_interface::HomeBuilt>()?;
    world.register_event::<core_ecs::sim_interface::TaxCollected>()?;
    world.register_event::<core_ecs::sim_interface::Married>()?;
    world.register_event::<core_ecs::sim_interface::Born>()?;
    world.register_event::<core_ecs::sim_interface::SchoolAttended>()?;
    Ok(())
}

/// Derives the schedule-relevant world composition from a LOADED world's
/// actual content, so a resumed run always reconstructs exactly the
/// systems the world was saved under (SPEC §9: loading a save and running
/// N ticks must equal running the original those same N ticks). The save
/// is authoritative — for its composition just as for its seed; CLI flags
/// never override it.
///
/// Known degenerate case (documented): a fixture world saved with zero
/// `FixtureWealth` rows (possible only via `--fixture --entities 0`)
/// derives `fixture: false` and drops the alarm chain on resume.
pub fn derive_spec_from_world(world: &World) -> Result<WorldSpec, EcsError> {
    let citizens = world.iter::<sim_people::Identity>()?.count() as u32;
    let fixture_rows = world.iter::<fixture::FixtureWealth>()?.count() as u32;
    // The economy outlives its citizens (ADR 0007 §8b): a world whose
    // conservation ledger exists keeps its firm systems and auditor
    // scheduled even after the last citizen dies.
    let economy = world
        .iter::<core_ecs::sim_interface::EconCounters>()?
        .next()
        .is_some();
    Ok(WorldSpec {
        seed: world.seed(),
        fixture: fixture_rows > 0,
        fixture_entities: fixture_rows,
        citizens,
        economy,
    })
}

/// Builds the schedule for a spec: the explicit ordered system lists
/// (SPEC §6). Order within each rate:
/// - Tick: fixture alarm chain, fixture walk (fixture worlds only);
///   then ai.decide, ai.act (towns only — decisions land before actions
///   advance, so a fresh decision starts moving the same tick).
/// - Hour: econ.production (batches settle before the day's deciding),
///   then ai.plan (compile hour only), then needs decay (towns only).
/// - Day (ADR 0007 §5, ADR 0008 §4, ADR 0009): debug.audit FIRST
///   (validates yesterday before today moves anything), then
///   people.working_age, econ.bank (policy rule, loan servicing and
///   origination, deposits), econ.payroll (wages with income tax
///   withheld), econ.labor_market (the daily clearing incl. the public
///   employer), econ.rent (tenancies pay or evict), econ.rental_market,
///   econ.purchase_market (every N days), then econ.trade, econ.pricing,
///   goods.spoilage, and mortality last (towns only).
///
/// The town list is scheduled when the world has citizens OR an economy
/// (ADR 0007 §8b): firms keep working after the last citizen dies, and
/// the citizen systems no-op honestly over an empty town — so a
/// post-extinction resume evolves exactly like the uninterrupted run.
pub fn build_schedule(spec: &WorldSpec, defs: &DataDefs) -> Schedule {
    let mut schedule = Schedule::new();
    if spec.fixture {
        schedule.add_system(Rate::Tick, Box::new(fixture::FixtureAlarmSystem));
        schedule.add_system(Rate::Tick, Box::new(fixture::FixtureWalkSystem));
    }
    if spec.citizens > 0 || spec.economy {
        let tables = data_defs::resolve_ai(defs);
        let econ = data_defs::resolve_economy(defs);
        schedule.add_system(
            Rate::Tick,
            Box::new(sim_ai::DecideSystem::new(tables.clone())),
        );
        schedule.add_system(Rate::Tick, Box::new(sim_ai::ActSystem::new(tables.clone())));
        schedule.add_system(
            Rate::Hour,
            Box::new(sim_economy::ProductionSystem::new(econ.clone())),
        );
        schedule.add_system(
            Rate::Hour,
            Box::new(sim_ai::PlanSystem::new(tables.clone())),
        );
        // The social hour (Phase 7, ADR 0010 §§2–3): bonds drift and
        // gossip spreads wherever leisure gathers people.
        schedule.add_system(
            Rate::Hour,
            Box::new(sim_ai::SocialDriftSystem::new(tables.clone())),
        );
        let decays = defs
            .people
            .needs
            .needs
            .iter()
            .map(|need| need.decay_per_hour)
            .collect();
        schedule.add_system(
            Rate::Hour,
            Box::new(sim_people::NeedsDecaySystem::new(decays)),
        );
        schedule.add_system(Rate::Day, Box::new(debug_tools::AuditSystem));
        schedule.add_system(
            Rate::Day,
            Box::new(sim_people::WorkingAgeSystem::new(
                defs.labor.min_working_age_years,
                defs.skills.school.start_age_years,
                defs.skills.school.end_age_years,
                ticks_per_year(defs),
            )),
        );
        schedule.add_system(
            Rate::Day,
            Box::new(sim_economy::BankSystem::new(econ.clone())),
        );
        schedule.add_system(
            Rate::Day,
            Box::new(sim_economy::PayrollSystem::new(econ.clone())),
        );
        schedule.add_system(
            Rate::Day,
            Box::new(sim_economy::LaborMarketSystem::new(econ.clone())),
        );
        schedule.add_system(Rate::Day, Box::new(sim_economy::RentSystem));
        schedule.add_system(
            Rate::Day,
            Box::new(sim_economy::RentalMarketSystem::new(econ.clone())),
        );
        schedule.add_system(
            Rate::Day,
            Box::new(sim_economy::PurchaseMarketSystem::new(econ.clone())),
        );
        schedule.add_system(
            Rate::Day,
            Box::new(sim_economy::TradeSystem::new(econ.clone())),
        );
        schedule.add_system(
            Rate::Day,
            Box::new(sim_economy::PricingSystem::new(econ.clone())),
        );
        schedule.add_system(
            Rate::Day,
            Box::new(sim_goods::SpoilageSystem::new(econ.spoil_per_mille.clone())),
        );
        // The Phase 7 lifecycle (ADR 0010 §§2, 4): decay first (absence
        // erodes), then marriages (thresholds crossed yesterday), then
        // births — before mortality, so a newborn's first day counts.
        schedule.add_system(
            Rate::Day,
            Box::new(sim_ai::RelationshipDecaySystem::new(
                defs.social.decay_per_day_per_mille,
            )),
        );
        schedule.add_system(
            Rate::Day,
            Box::new(sim_people::MarriageSystem::new(
                defs.social.marriage_threshold_per_mille,
                defs.social.edge_cap as usize,
            )),
        );
        schedule.add_system(
            Rate::Day,
            Box::new(sim_people::FertilitySystem::new(
                defs.fertility.clone(),
                defs.people.clone(),
                ticks_per_year(defs),
                defs.skills.skills.len(),
                defs.social.edge_cap as usize,
            )),
        );
        schedule.add_system(
            Rate::Day,
            Box::new(sim_people::MortalitySystem::new(
                defs.people.mortality.clone(),
                ticks_per_year(defs),
            )),
        );
    }
    schedule
}

/// Assembles a fresh simulation (registrations, populations, calendar) and
/// its schedule.
pub fn build_simulation(
    spec: &WorldSpec,
    defs: &DataDefs,
) -> Result<(Simulation, Schedule), RunnerError> {
    let calendar = Calendar::new(defs.calendar.days_per_season)?;
    let mut world = World::new(spec.seed, defs.engine.event_log_capacity as usize);
    register_world(&mut world)?;
    if spec.fixture {
        fixture::populate(&mut world, spec.fixture_entities)?;
    }
    if spec.citizens > 0 {
        sim_people::genesis::populate(
            &mut world,
            &defs.people,
            spec.citizens,
            ticks_per_year(defs),
            defs.labor.min_working_age_years,
            (
                defs.skills.school.start_age_years,
                defs.skills.school.end_age_years,
            ),
            defs.skills.skills.len(),
        )?;
        // World genesis (order fixed: public locations, then homes —
        // deterministic entity indices). Household member lists are read
        // here and passed as plain entities: sim_world never touches
        // sim_people types (SPEC §4).
        sim_world::genesis::create_public_locations(&mut world, &defs.locations)?;
        let households: Vec<Vec<core_ecs::Entity>> = world
            .iter::<sim_people::Household>()?
            .map(|(_, household)| household.members.clone())
            .collect();
        sim_world::genesis::place_households(&mut world, &defs.locations, &households)?;
        // Economy genesis last (ADR 0007 §2): the ledger entity, then
        // firms (kind × instance order), then issuance recorded from
        // EVERY wallet seeded above — the identities hold from tick 0.
        sim_economy::genesis::populate(&mut world, &data_defs::resolve_economy(defs))?;
    }
    Ok((Simulation::new(world, calendar), build_schedule(spec, defs)))
}

/// Runs `ticks` ticks, recording `(tick, hash)` at every multiple of
/// `hash_interval` (including tick 0 if the run starts there) and after the
/// final tick. A zero interval means "hash only at the end".
///
/// Invariant: for the same simulation state and arguments, the returned
/// sequence is identical across runs — it is the determinism suite's
/// observable.
pub fn run_with_hashes(
    sim: &mut Simulation,
    schedule: &mut Schedule,
    ticks: u64,
    hash_interval: u64,
) -> Result<HashSequence, EcsError> {
    let mut hashes = Vec::new();
    for _ in 0..ticks {
        if hash_interval != 0 && sim.tick().raw().is_multiple_of(hash_interval) {
            hashes.push((sim.tick(), sim.state_hash()?));
        }
        sim.step(schedule)?;
    }
    hashes.push((sim.tick(), sim.state_hash()?));
    Ok(hashes)
}

/// Runs the same spec twice from scratch and returns both hash sequences.
/// Equal sequences are the SPEC §14(a) determinism check.
pub fn verify_two_fresh_runs(
    spec: &WorldSpec,
    defs: &DataDefs,
    ticks: u64,
    hash_interval: u64,
) -> Result<(HashSequence, HashSequence), RunnerError> {
    let (mut sim_a, mut schedule_a) = build_simulation(spec, defs)?;
    let (mut sim_b, mut schedule_b) = build_simulation(spec, defs)?;
    let a = run_with_hashes(&mut sim_a, &mut schedule_a, ticks, hash_interval)?;
    let b = run_with_hashes(&mut sim_b, &mut schedule_b, ticks, hash_interval)?;
    Ok((a, b))
}

//! Phase 2 exit-criterion suite (SPEC §15): 10k citizens simulate a year;
//! demographics stay within data-defined bands; conservation trivially
//! holds. Also the first statistical regression test (SPEC §14): a
//! golden-seed run asserted against bands, not exact values — validating
//! the SHIPPED balance, so this suite loads the live `data/` directory.

use core_types::Seed;
use embervale_tests::live_defs;
use headless::runner::{self, WorldSpec};

/// The passive-town schedule (needs decay + mortality, no AI): the Phase 2
/// demographic regression runs 10,000 citizens, and full per-tick Tier A
/// AI at that scale is exactly what the Phase 8 LOD tiers exist to make
/// affordable (SPEC §10/§15: "Tier A only at small scale" in Phase 3).
/// Mortality — the system under regression — is identical either way.
fn passive_schedule(defs: &data_defs::DataDefs) -> core_ecs::Schedule {
    let mut schedule = core_ecs::Schedule::new();
    schedule.add_system(
        core_ecs::Rate::Hour,
        Box::new(sim_people::NeedsDecaySystem::new(
            defs.people
                .needs
                .needs
                .iter()
                .map(|n| n.decay_per_hour)
                .collect(),
        )),
    );
    schedule.add_system(
        core_ecs::Rate::Day,
        Box::new(sim_people::MortalitySystem::new(
            defs.people.mortality.clone(),
            runner::ticks_per_year(defs),
        )),
    );
    schedule
}

/// SPEC §15 Phase 2 exit criterion, all three clauses in one golden-seed
/// run: 10,000 citizens, one simulated year.
#[test]
fn ten_thousand_citizens_simulate_a_year_within_demographic_bands() {
    let defs = live_defs();
    let spec = WorldSpec::town(Seed::new(0xC171_2E17), 10_000);
    let (mut sim, _) = runner::build_simulation(&spec, &defs).expect("build failed");
    let mut schedule = passive_schedule(&defs);

    let initial_population = headless::inspect::population(sim.world()).expect("count");
    assert_eq!(initial_population, 10_000);

    let year = runner::ticks_per_year(&defs);
    sim.run_ticks(&mut schedule, year).expect("year run failed");

    // Demographics within data-defined bands: the crude annual death rate
    // band lives in data/balance/demographics.ron (ADR 0005 §6).
    let final_population = headless::inspect::population(sim.world()).expect("count");
    let deaths = initial_population - final_population;
    let rate_per_mille = u64::from(deaths) * 1000 / u64::from(initial_population);
    let band = &defs.people.demographics;
    assert!(
        (u64::from(band.annual_death_rate_min_per_mille)
            ..=u64::from(band.annual_death_rate_max_per_mille))
            .contains(&rate_per_mille),
        "annual crude death rate {rate_per_mille}‰ left the data-defined band \
         [{}‰, {}‰] ({deaths} deaths / {initial_population})",
        band.annual_death_rate_min_per_mille,
        band.annual_death_rate_max_per_mille,
    );

    // No births until Phase 7: population can only decrease.
    assert!(final_population <= initial_population);

    // Conservation trivially holds (SPEC §15 Phase 2): no money or goods
    // exist anywhere — the world has no economic state to conserve. This
    // assertion documents the absence; the real auditors go live with the
    // first ledgers in Phase 4.
    assert!(
        sim.world()
            .iter::<headless::fixture::FixtureWealth>()
            .expect("query")
            .count()
            == 0,
        "a pure town must carry no fixture wealth"
    );
}

/// The year-long run is deterministic end-to-end: an identical second run
/// reaches the identical final hash. (Kept separate from the band test so
/// a band failure and a determinism failure are distinguishable.)
#[test]
fn the_ten_thousand_citizen_year_is_deterministic() {
    let defs = live_defs();
    let spec = WorldSpec::town(Seed::new(0xC171_2E17), 10_000);
    let year = runner::ticks_per_year(&defs);

    let run = |()| -> core_types::WorldHash {
        let (mut sim, _) = runner::build_simulation(&spec, &defs).expect("build failed");
        let mut schedule = passive_schedule(&defs);
        sim.run_ticks(&mut schedule, year).expect("run failed");
        sim.state_hash().expect("hash failed")
    };
    assert_eq!(run(()), run(()));
}

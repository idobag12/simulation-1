//! Behavioral suite for the Phase 2 people systems (SPEC §16.5: no
//! untested claims). Covers the paths the Phase 2 adversarial review found
//! untested: the death path (event, household removal, dissolution,
//! multi-death days), needs decay arithmetic, genesis invariants, and
//! save/load determinism across actual deaths.

use core_types::Seed;
use embervale_tests::pinned_defs;
use headless::runner::{self, WorldSpec};
use sim_people::{Household, HouseholdMember, Identity, Needs, PersonDied, Personality, Sex};

/// Pinned defs with the mortality curve overridden (test input): a flat
/// per-day death chance for every age.
fn defs_with_flat_mortality(per_day_chance_per_billion: u32) -> data_defs::DataDefs {
    let mut defs = pinned_defs();
    defs.people.mortality.bands.clear();
    defs.people.mortality.terminal_per_day_chance_per_billion = per_day_chance_per_billion;
    defs
}

/// With a certain-death curve (probability 1.0/day), every citizen dies on
/// the first mortality tick: one `PersonDied` per citizen, every household
/// dissolved (multi-death same household same day is the NORM here), all
/// entities gone. Exercises the CommandBuffer::run dissolution closures
/// under maximum contention.
#[test]
fn certain_mortality_kills_everyone_and_dissolves_every_household() {
    let defs = defs_with_flat_mortality(1_000_000_000);
    let spec = WorldSpec::town(Seed::new(31), 40);
    let (mut sim, mut schedule) = runner::build_simulation(&spec, &defs).expect("build failed");
    assert_eq!(
        headless::inspect::population(sim.world()).expect("count"),
        40
    );
    let households_before = sim.world().iter::<Household>().expect("query").count();
    assert!(households_before > 0);

    // Mortality fires at tick 0 (a day boundary); deaths become readable
    // events during tick 1.
    sim.step(&mut schedule).expect("tick 0");
    assert_eq!(
        headless::inspect::population(sim.world()).expect("count"),
        0,
        "a probability-1 curve must kill every citizen on day 0"
    );
    assert_eq!(
        sim.world().iter::<Household>().expect("query").count(),
        0,
        "every household must dissolve when its last member dies"
    );
    // Only the town's institutions remain: locations (homes + public
    // places persist — people die, places don't), firms (Phase 4:
    // retail firms are also locations, so count entities, not stores),
    // and the conservation-ledger entity.
    let world = sim.world();
    let mut institutions = std::collections::BTreeSet::new();
    for (entity, _) in world
        .iter::<core_ecs::sim_interface::Location>()
        .expect("query")
    {
        institutions.insert(entity.index());
    }
    for (entity, _) in world.iter::<sim_economy::Firm>().expect("query") {
        institutions.insert(entity.index());
    }
    for (entity, _) in world
        .iter::<core_ecs::sim_interface::EconCounters>()
        .expect("query")
    {
        institutions.insert(entity.index());
    }
    assert_eq!(
        world.entity_count(),
        institutions.len(),
        "besides locations, firms, and the ledger, no citizen or household entities may remain"
    );
    // The dead take nothing with them (ADR 0007 §8a): every estate
    // escheated to the ledger, so the wallet sum still equals issuance.
    assert!(
        debug_tools::audit_economy(world).expect("audit"),
        "conservation must hold across a full die-off"
    );

    sim.step(&mut schedule).expect("tick 1");
    let deaths = sim.world().events::<PersonDied>().expect("read");
    assert_eq!(deaths.len(), 40, "one PersonDied fact per citizen");
}

/// With a moderate flat curve, deaths accumulate over days while survivors
/// keep consistent household edges: every surviving member's household
/// still lists them, member lists stay sorted by entity index, and no
/// household is ever empty.
#[test]
fn partial_mortality_keeps_household_graph_consistent() {
    // ~2% per day: plenty of deaths in 10 days of a 200-citizen town.
    let defs = defs_with_flat_mortality(20_000_000);
    let spec = WorldSpec::town(Seed::new(32), 200);
    let (mut sim, mut schedule) = runner::build_simulation(&spec, &defs).expect("build failed");
    sim.run_ticks(&mut schedule, 10 * 1440).expect("run failed");

    let world = sim.world();
    let survivors = headless::inspect::population(world).expect("count");
    assert!(
        survivors < 200 && survivors > 0,
        "expected partial mortality, got {survivors}/200 survivors"
    );

    for (citizen, member) in world.iter::<HouseholdMember>().expect("query") {
        let household = world
            .get::<Household>(member.household)
            .expect("query")
            .expect("member edge points at a live household");
        assert!(
            household.members.contains(&citizen),
            "household must list its member"
        );
    }
    for (_, household) in world.iter::<Household>().expect("query") {
        assert!(!household.members.is_empty(), "no empty household survives");
        assert!(
            household
                .members
                .windows(2)
                .all(|w| w[0].index() < w[1].index()),
            "member lists stay sorted by entity index after deaths"
        );
        for member in &household.members {
            assert!(world.is_alive(*member), "member lists hold no dead handles");
        }
    }
}

/// Save/load determinism ACROSS DEATHS (SPEC §9): a run with real deaths,
/// saved mid-flight, resumes to the identical final state as the
/// uninterrupted run — including identical subsequent mortality draws.
#[test]
fn deaths_replay_identically_across_save_load() {
    let defs = defs_with_flat_mortality(20_000_000);
    let spec = WorldSpec::town(Seed::new(33), 150);
    let total = 12 * 1440;
    let split = 6 * 1440 + 300; // mid-day, mid-flight

    let (mut solid, mut solid_schedule) = runner::build_simulation(&spec, &defs).expect("build");
    solid.run_ticks(&mut solid_schedule, total).expect("run");
    let solid_pop = headless::inspect::population(solid.world()).expect("count");
    assert!(
        solid_pop < 150,
        "the run must contain deaths to be probative"
    );

    let (mut first, mut first_schedule) = runner::build_simulation(&spec, &defs).expect("build");
    first.run_ticks(&mut first_schedule, split).expect("run");
    assert!(
        headless::inspect::population(first.world()).expect("count") < 150,
        "deaths must occur BEFORE the save point too"
    );
    let save = persistence::save_to_bytes(&first).expect("save");
    let mut resumed = persistence::load_from_bytes(
        &save,
        runner::load_config(&defs).expect("config"),
        runner::register_world,
    )
    .expect("load");
    let mut resumed_schedule = runner::build_schedule(&spec, &defs);
    resumed
        .run_ticks(&mut resumed_schedule, total - split)
        .expect("resume");

    assert_eq!(
        solid.state_hash().expect("hash"),
        resumed.state_hash().expect("hash"),
        "post-load mortality diverged from the uninterrupted run"
    );
}

/// Needs decay arithmetic IN ISOLATION: after one hour-firing, every level
/// dropped by exactly the data-defined amount (clamped at zero), and
/// levels never leave the valid range over a long run. The AI systems are
/// deliberately absent (a custom decay-only schedule) so satisfaction
/// gains cannot mask the decay arithmetic; the AI/decay interplay is
/// covered by `ai.rs`.
#[test]
fn needs_decay_matches_config_exactly_and_clamps_at_zero() {
    let defs = pinned_defs();
    let spec = WorldSpec::town(Seed::new(34), 25);
    let (mut sim, _) = runner::build_simulation(&spec, &defs).expect("build");
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

    let before: Vec<(u32, Vec<i64>)> = sim
        .world()
        .iter::<Needs>()
        .expect("query")
        .map(|(e, n)| (e.index(), n.levels.iter().map(|l| l.raw()).collect()))
        .collect();

    // Tick 0 begins hour 0: the decay system fires exactly once.
    sim.step(&mut schedule).expect("step");

    let decays: Vec<i64> = defs
        .people
        .needs
        .needs
        .iter()
        .map(|n| n.decay_per_hour)
        .collect();
    for ((index, old_levels), (entity, needs)) in before
        .iter()
        .zip(sim.world().iter::<Needs>().expect("query"))
    {
        assert_eq!(*index, entity.index());
        for ((old, decay), new) in old_levels.iter().zip(&decays).zip(&needs.levels) {
            assert_eq!(new.raw(), (old - decay).max(0), "exact per-hour decay");
        }
    }

    // Long run: levels stay in range and eventually pin at zero.
    sim.run_ticks(&mut schedule, 90 * 1440).expect("run");
    for (_, needs) in sim.world().iter::<Needs>().expect("query") {
        for level in &needs.levels {
            assert!((0..=1_000_000).contains(&level.raw()));
        }
    }
}

/// Genesis invariants (ADR 0005 §5): counts, membership, ordering, ranges,
/// names, and the age pyramid within tolerance of the data weights.
#[test]
fn genesis_respects_data_distributions_and_household_invariants() {
    let defs = pinned_defs();
    let spec = WorldSpec::town(Seed::new(35), 5_000);
    let (sim, _) = runner::build_simulation(&spec, &defs).expect("build");
    let world = sim.world();
    let ticks_per_year = runner::ticks_per_year(&defs);

    assert_eq!(headless::inspect::population(world).expect("count"), 5_000);

    // Every citizen fully formed and in exactly one existing household.
    let mut male = 0u32;
    let mut band_counts = vec![0u32; defs.people.demographics.age_bands.len()];
    for (citizen, identity) in world.iter::<Identity>().expect("query") {
        let needs = world.get::<Needs>(citizen).expect("q").expect("has needs");
        let personality = world
            .get::<Personality>(citizen)
            .expect("q")
            .expect("has personality");
        let member = world
            .get::<HouseholdMember>(citizen)
            .expect("q")
            .expect("has household");
        let household = world
            .get::<Household>(member.household)
            .expect("q")
            .expect("household exists");
        assert!(household.members.contains(&citizen));

        // Ranges from data.
        for (level, def) in needs.levels.iter().zip(&defs.people.needs.needs) {
            assert!((def.initial_min..=def.initial_max).contains(&level.raw()));
        }
        for (weight, def) in personality.weights.iter().zip(&defs.people.traits.traits) {
            assert!((def.min..=def.max).contains(weight));
        }

        // Names come from the authored lists.
        let names = match identity.sex {
            Sex::Male => {
                male += 1;
                &defs.people.given_male.names
            }
            Sex::Female => &defs.people.given_female.names,
        };
        assert!(names.contains(&identity.given_name));
        assert!(defs.people.family.names.contains(&identity.family_name));

        // Age lies in one of the data bands; tally it for the pyramid check.
        let age = identity.age_years(sim.tick(), ticks_per_year);
        let band_index = defs
            .people
            .demographics
            .age_bands
            .iter()
            .position(|b| (b.min_age_years..=b.max_age_years).contains(&age))
            .unwrap_or_else(|| panic!("age {age} outside every data band"));
        band_counts[band_index] += 1;
    }
    assert!(
        male > 1_000 && male < 4_000,
        "both sexes present ({male} male)"
    );

    // Age pyramid within ±3 percentage points of the data weights at n=5000.
    for (count, band) in band_counts.iter().zip(&defs.people.demographics.age_bands) {
        let actual_per_mille = u64::from(*count) * 1000 / 5_000;
        let expected = u64::from(band.weight_per_mille);
        assert!(
            actual_per_mille.abs_diff(expected) <= 30,
            "band {}..{} expected ~{expected}‰, got {actual_per_mille}‰",
            band.min_age_years,
            band.max_age_years
        );
    }

    // Household sizes: within [min, max] except at most one tail household.
    let (min, max) = (
        defs.people.demographics.household_min as usize,
        defs.people.demographics.household_max as usize,
    );
    let mut undersized = 0;
    let mut member_total = 0;
    for (_, household) in world.iter::<Household>().expect("query") {
        member_total += household.members.len();
        assert!(household.members.len() <= max);
        if household.members.len() < min {
            undersized += 1;
        }
    }
    assert!(undersized <= 1, "only the tail household may be undersized");
    assert_eq!(
        member_total, 5_000,
        "every citizen in exactly one household"
    );
}

/// The `--load` regression (Phase 2 review): resuming a town save derives
/// its schedule from the SAVE's content, so a flag-less resume matches the
/// uninterrupted run instead of silently freezing the town.
#[test]
fn cli_resume_without_flags_matches_uninterrupted_run() {
    let dir = std::env::temp_dir().join("embervale-people-test");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let save = dir.join("resume.embersave");
    let save_str = save.to_str().expect("utf8");
    let data = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/data_v5");

    let arg = |v: &[&str]| -> Vec<String> {
        let mut a: Vec<String> = v.iter().map(|s| (*s).to_owned()).collect();
        a.extend(["--data".to_owned(), data.to_owned()]);
        a
    };

    // Uninterrupted 6-day town run vs save-at-3-days + FLAGLESS resume.
    assert_eq!(
        headless::cli::dispatch(&arg(&[
            "run",
            "--seed",
            "36",
            "--ticks",
            "4320",
            "--citizens",
            "120",
            "--save",
            save_str,
        ])),
        Ok(true)
    );
    // Resume WITHOUT --citizens: must succeed (the CLI derives the
    // schedule from the save), proven equivalent in-process below.
    assert_eq!(
        headless::cli::dispatch(&arg(&["run", "--load", save_str, "--ticks", "4320"])),
        Ok(true)
    );
    let defs = pinned_defs();
    let spec = WorldSpec::town(Seed::new(36), 120);
    let (mut solid, mut solid_schedule) = runner::build_simulation(&spec, &defs).expect("build");
    solid.run_ticks(&mut solid_schedule, 8640).expect("run");

    let loaded = persistence::load_from_file(
        &save,
        runner::load_config(&defs).expect("config"),
        runner::register_world,
    )
    .expect("load");
    let derived = runner::derive_spec_from_world(loaded.world()).expect("derive");
    assert_eq!(
        derived.citizens, 120,
        "derivation reads the town from the save"
    );
    assert!(!derived.fixture);
    let mut resumed = loaded;
    let mut derived_schedule = runner::build_schedule(&derived, &defs);
    resumed
        .run_ticks(&mut derived_schedule, 4320)
        .expect("resume");
    assert_eq!(
        solid.state_hash().expect("hash"),
        resumed.state_hash().expect("hash"),
        "derived-schedule resume must match the uninterrupted run"
    );
    std::fs::remove_file(&save).ok();
}

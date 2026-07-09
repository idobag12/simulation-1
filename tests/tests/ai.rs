//! Phase 3 exit-criteria suite (SPEC §15): citizens visibly satisfy needs
//! with individually distinct patterns; every decision inspectable;
//! determinism holds (the determinism/save-compat suites run AI towns).

use core_ecs::sim_interface::{Needs, Position, Residence};
use core_types::Seed;
use embervale_tests::pinned_defs;
use headless::runner::{self, WorldSpec};
use sim_ai::{CandidateAction, CurrentAction, LastDecision};

/// Exit criterion 1: citizens VISIBLY satisfy needs. In a Phase 2 world
/// every satisfiable need decays monotonically to zero; with the AI, a
/// 500-citizen town (SPEC §15: Tier A at small scale) runs five days and
/// (a) mean satisfiable-need levels stay well above zero, (b) hunger
/// demonstrably RISES for citizens between observations (someone ate).
#[test]
fn citizens_visibly_satisfy_needs() {
    let defs = pinned_defs();
    let spec = WorldSpec::town(Seed::new(71), 500);
    let (mut sim, mut schedule) = runner::build_simulation(&spec, &defs).expect("build");

    // Warm-up day so the town settles into behavior.
    sim.run_ticks(&mut schedule, 1440).expect("run");
    let snapshot = |sim: &sim_time::Simulation| -> Vec<(u32, Vec<i64>)> {
        sim.world()
            .iter::<Needs>()
            .expect("query")
            .map(|(e, n)| (e.index(), n.levels.iter().map(|l| l.raw()).collect()))
            .collect()
    };
    let early = snapshot(&sim);

    sim.run_ticks(&mut schedule, 4 * 1440).expect("run");
    let late = snapshot(&sim);

    // (a) The town is not starving: mean hunger and rest stay above a
    // quarter tank after five days (a decay-only world is at 0 for both by
    // day two).
    let need_index = |id: &str| {
        defs.people
            .needs
            .needs
            .iter()
            .position(|n| n.id == id)
            .expect("need exists")
    };
    for id in ["hunger", "rest", "social"] {
        let index = need_index(id);
        let mean: i64 =
            late.iter().map(|(_, levels)| levels[index]).sum::<i64>() / late.len().max(1) as i64;
        assert!(
            mean > 250_000,
            "mean {id} after 5 days is {mean} — citizens are not satisfying it"
        );
    }

    // (b) Hunger rose for many citizens between the two observations:
    // impossible without satisfaction (decay only goes down).
    let hunger = need_index("hunger");
    let early_by_index: std::collections::BTreeMap<u32, i64> = early
        .iter()
        .map(|(index, levels)| (*index, levels[hunger]))
        .collect();
    let risers = late
        .iter()
        .filter(|(index, levels)| {
            early_by_index
                .get(index)
                .is_some_and(|old| levels[hunger] > *old)
        })
        .count();
    assert!(
        risers > 100,
        "only {risers}/500 citizens ever gained hunger satisfaction"
    );
}

/// Exit criterion 2: individually distinct patterns. Citizens with
/// different personalities distribute their time differently: over three
/// days, the multiset of (location kind) positions across observation
/// snapshots must show several distinct per-citizen distributions.
#[test]
fn citizens_show_individually_distinct_patterns() {
    let defs = pinned_defs();
    let spec = WorldSpec::town(Seed::new(72), 120);
    let (mut sim, mut schedule) = runner::build_simulation(&spec, &defs).expect("build");

    // Sample every citizen's location kind once an hour for three days.
    let mut visit_profiles: std::collections::BTreeMap<u32, Vec<u32>> =
        std::collections::BTreeMap::new();
    for _ in 0..(3 * 24) {
        sim.run_ticks(&mut schedule, 60).expect("run");
        let world = sim.world();
        for (entity, position) in world.iter::<Position>().expect("query") {
            let kind = world
                .get::<core_ecs::sim_interface::Location>(position.at)
                .expect("query")
                .map(|l| l.kind)
                .unwrap_or(u32::MAX);
            visit_profiles.entry(entity.index()).or_default().push(kind);
        }
    }

    // Count distinct hour-by-hour profiles: identical robots would produce
    // one; personalities + distinct homes must produce many.
    let distinct: std::collections::BTreeSet<&Vec<u32>> = visit_profiles.values().collect();
    assert!(
        distinct.len() > visit_profiles.len() / 4,
        "only {} distinct daily patterns across {} citizens",
        distinct.len(),
        visit_profiles.len()
    );
}

/// Exit criterion 3: every decision inspectable. After a day, every
/// citizen carries a `LastDecision` whose candidate list is coherent, and
/// the inspector renders it with resolved names.
#[test]
fn every_decision_is_inspectable() {
    let defs = pinned_defs();
    let spec = WorldSpec::town(Seed::new(73), 80);
    let (mut sim, mut schedule) = runner::build_simulation(&spec, &defs).expect("build");
    sim.run_ticks(&mut schedule, 1440).expect("run");

    let world = sim.world();
    let mut checked = 0;
    for (entity, decision) in world.iter::<LastDecision>().expect("query") {
        checked += 1;
        assert!(
            (decision.chosen as usize) < decision.candidates.len(),
            "chosen index out of range"
        );
        assert!(
            !decision.candidates.is_empty(),
            "empty candidate list recorded"
        );
        // The dump is self-consistent: a Satisfy choice matches the
        // citizen's ongoing action or a completed one; Idle matches Idle.
        if let Some(action) = world.get::<CurrentAction>(entity).expect("query") {
            let chosen = decision.candidates[decision.chosen as usize].action;
            let idle_mismatch = matches!(chosen, CandidateAction::Idle)
                && !matches!(action, CurrentAction::Idle { .. });
            // An Idle decision can only coexist with an Idle (or absent)
            // action; a Satisfy decision may have progressed to any phase.
            assert!(
                !idle_mismatch,
                "idle decision but ongoing action {action:?}"
            );
        }
    }
    let population = headless::inspect::population(world).expect("count");
    assert_eq!(
        checked, population,
        "every living citizen must carry an inspectable decision"
    );

    // The inspector renders the dump with resolved names for a real
    // citizen (entity index 1 is the first genesis household's first
    // member).
    let dump = headless::inspect::inspect_entity(&sim, &defs, 1).expect("inspect");
    assert!(dump.contains("last decision"), "{dump}");
    assert!(dump.contains("candidates:"), "{dump}");
    assert!(dump.contains("micro"), "{dump}");
}

/// Citizens sleep at home at night (the DailyPlan bias works): at 03:00,
/// most citizens are at their own home.
#[test]
fn citizens_sleep_at_home_at_night() {
    let defs = pinned_defs();
    let spec = WorldSpec::town(Seed::new(74), 150);
    let (mut sim, mut schedule) = runner::build_simulation(&spec, &defs).expect("build");

    // Run to 03:00 on day 3 (past two plan compilations).
    let target = 2 * 1440 + 3 * 60;
    sim.run_ticks(&mut schedule, target).expect("run");

    let world = sim.world();
    let mut at_home = 0u32;
    let mut total = 0u32;
    for (entity, position) in world.iter::<Position>().expect("query") {
        total += 1;
        let home = world
            .get::<Residence>(entity)
            .expect("query")
            .map(|r| r.home);
        if home == Some(position.at) {
            at_home += 1;
        }
    }
    assert!(
        at_home * 3 >= total * 2,
        "at 03:00 only {at_home}/{total} citizens are home"
    );
}

/// The AI town replays identically across save/load mid-decision,
/// mid-travel, and mid-performance (SPEC §9; the fixture suites cover the
/// hash; this asserts it at the AI's own scale and cadence).
#[test]
fn ai_town_replays_identically_across_save_load() {
    let defs = pinned_defs();
    let spec = WorldSpec::town(Seed::new(75), 200);
    let total = 3 * 1440;
    let split = 1440 + 411; // mid-day, mid-everything

    let (mut solid, mut solid_schedule) = runner::build_simulation(&spec, &defs).expect("build");
    solid.run_ticks(&mut solid_schedule, total).expect("run");

    let (mut first, mut first_schedule) = runner::build_simulation(&spec, &defs).expect("build");
    first.run_ticks(&mut first_schedule, split).expect("run");
    // The save happens with live actions in flight.
    assert!(
        first
            .world()
            .iter::<CurrentAction>()
            .expect("query")
            .count()
            > 0,
        "the save point must have actions in flight to be probative"
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
        "AI town diverged after save/load"
    );
}

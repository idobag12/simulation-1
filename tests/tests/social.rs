//! Phase 7 exit-criteria suite (SPEC §15): a multi-generation run
//! produces kinship networks and skill mobility; the narrative composer
//! surfaces coherent story lines from real event chains. The
//! fast-generation town is data-shaped (the flat-mortality pattern
//! applied to time: an 8-day year, a wide fertility band — all legal
//! data), everything else pinned.

use core_ecs::sim_interface::{RelKind, Relationships, Skills};
use core_types::Seed;
use embervale_tests::pinned_defs;
use headless::runner::{self, WorldSpec};
use sim_people::Identity;

/// One shaped multi-generation run shared by both exit criteria (the
/// composer reads the same world the kinship assertions count).
fn generation_run() -> sim_time::Simulation {
    let mut defs = pinned_defs();
    defs.calendar.days_per_season = 2; // an 8-day year: generations in minutes
    defs.fertility.bands = vec![sim_people::config::FertilityBand {
        min_age_years: 16,
        max_age_years: 70,
        per_day_chance_per_billion: 250_000_000,
    }];
    defs.people.mortality.bands.clear();
    defs.people.mortality.terminal_per_day_chance_per_billion = 400_000;

    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(101), 80), &defs).expect("build");
    let year = 8 * 1440u64;
    sim.run_ticks(&mut schedule, 60 * year)
        .expect("a halted run means a daily audit failed");
    sim
}

#[test]
fn a_multi_generation_run_produces_kinship_networks_and_skill_mobility() {
    let sim = generation_run();
    let world = sim.world();

    // --- The kinship network (exit criterion 7a) --------------------
    let mut native_born = 0;
    let mut second_generation = 0;
    let mut spouse_edges = 0;
    let mut kin_edges = 0;
    let mut mobile = 0;
    let ticks_per_year = 8 * 1440i64;
    for (citizen, identity) in world.iter::<Identity>().expect("query") {
        let Some(relationships) = world.get::<Relationships>(citizen).expect("query") else {
            continue;
        };
        spouse_edges += relationships
            .edges
            .iter()
            .filter(|edge| matches!(edge.kind, RelKind::Spouse))
            .count();
        kin_edges += relationships
            .edges
            .iter()
            .filter(|edge| matches!(edge.kind, RelKind::Kin))
            .count();
        if identity.birth_tick < 0 {
            continue;
        }
        native_born += 1;
        // Elder kin = a generation up (≥ 16 world years older): parents,
        // not schooled siblings.
        let my_letters = world
            .get::<Skills>(citizen)
            .expect("query")
            .and_then(|skills| skills.levels.first().copied())
            .unwrap_or(0);
        let mut elders: Vec<(i64, u16)> = Vec::new();
        let mut has_native_parent = false;
        for edge in &relationships.edges {
            if !matches!(edge.kind, RelKind::Kin) {
                continue;
            }
            if let Some(elder) = world.get::<Identity>(edge.other).expect("query")
                && elder.birth_tick <= identity.birth_tick - 16 * ticks_per_year
            {
                elders.push((
                    elder.birth_tick,
                    world
                        .get::<Skills>(edge.other)
                        .expect("query")
                        .and_then(|skills| skills.levels.first().copied())
                        .unwrap_or(0),
                ));
                if elder.birth_tick >= 0 {
                    has_native_parent = true;
                }
            }
        }
        if has_native_parent {
            second_generation += 1;
        }
        // Skill mobility (exit criterion 7a): the school taught this
        // child what its parents never held.
        if my_letters >= 500 && !elders.is_empty() && elders.iter().all(|(_, p)| *p < 500) {
            mobile += 1;
        }
    }
    assert!(
        native_born >= 10,
        "generations were born ({native_born} natives)"
    );
    assert!(
        second_generation >= 3,
        "natives had native children — a third kinship generation \
         ({second_generation})"
    );
    assert!(
        spouse_edges >= 10,
        "marriages concluded ({spouse_edges} spouse edges)"
    );
    assert!(
        kin_edges >= 100,
        "the kinship NETWORK is real ({kin_edges} kin edges)"
    );
    assert!(
        mobile >= 3,
        "school-taught natives out-skilled their parents ({mobile} climbed)"
    );

    // --- The composer (exit criterion 7b) ---------------------------
    // The same world's event ring composes into real story lines citing
    // real citizens (SPEC §7: stories by observation, never injection).
    let lines = debug_tools::stories(world, None).expect("compose");
    assert!(
        !lines.is_empty(),
        "the retained window of a living town composes stories"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("married") || line.contains("was born")),
        "the family chain surfaces: {lines:?}"
    );
}

/// The composer's chain patterns, deterministically: hand-emitted facts
/// compose into the promised lines with the true names and days.
#[test]
fn the_composer_reads_chains_from_the_event_log() {
    use core_ecs::sim_interface::{Born, Married};
    use core_types::Ticks;
    use sim_people::Sex;

    let mut world = core_ecs::World::new(Seed::new(5), 64);
    world.register::<Identity>().expect("register");
    world.register_event::<Married>().expect("register");
    world.register_event::<Born>().expect("register");

    let alice = world.spawn();
    let bob = world.spawn();
    let carol = world.spawn();
    for (entity, name, sex) in [
        (alice, "Alice", Sex::Female),
        (bob, "Bob", Sex::Male),
        (carol, "Carol", Sex::Female),
    ] {
        world
            .insert(
                entity,
                Identity {
                    given_name: name.to_owned(),
                    family_name: "Stone".to_owned(),
                    sex,
                    birth_tick: 0,
                },
            )
            .expect("insert");
    }
    // Day 3: the wedding; day 10: the birth. Events become readable (and
    // logged) on the tick after emission.
    world.begin_tick(Ticks::new(3 * 1440));
    world
        .emit(&Married {
            partner_a: alice,
            partner_b: bob,
        })
        .expect("emit");
    world.begin_tick(Ticks::new(3 * 1440 + 1));
    world.begin_tick(Ticks::new(10 * 1440));
    world
        .emit(&Born {
            child: carol,
            parent_a: alice,
            parent_b: bob,
        })
        .expect("emit");
    world.begin_tick(Ticks::new(10 * 1440 + 1));

    let lines = debug_tools::stories(&world, None).expect("compose");
    assert!(
        lines.iter().any(|line| line.contains("Alice Stone")
            && line.contains("Bob Stone")
            && line.contains("married on day 3")
            && line.contains("Carol Stone")
            && line.contains("born on day 10")),
        "the family chain composes with true names and days: {lines:?}"
    );
    // Filtering by subject keeps only that citizen's stories.
    let filtered = debug_tools::stories(&world, Some(carol.index())).expect("compose");
    assert!(
        filtered.iter().all(|line| line.contains("Carol Stone")),
        "{filtered:?}"
    );
}

/// The composer's remaining chain patterns, deterministically: job loss
/// → new work, and default → renting again — including the sad-path
/// variants — and the remarriage attribution rule (a child binds to the
/// marriage whose FULL couple matches).
#[test]
fn the_composer_reads_labor_money_and_remarriage_chains() {
    use core_ecs::sim_interface::{
        Born, Fired, FiredReason, Hired, LoanDefaulted, Married, TenancyStarted,
    };
    use core_types::{Money, Ticks};
    use sim_people::Sex;

    let mut world = core_ecs::World::new(Seed::new(6), 64);
    world.register::<Identity>().expect("register");
    world.register_event::<Married>().expect("register");
    world.register_event::<Born>().expect("register");
    world.register_event::<Fired>().expect("register");
    world.register_event::<Hired>().expect("register");
    world.register_event::<LoanDefaulted>().expect("register");
    world.register_event::<TenancyStarted>().expect("register");

    let dana = world.spawn();
    let eli = world.spawn();
    let fern = world.spawn();
    let kid = world.spawn();
    let firm = world.spawn();
    let home = world.spawn();
    for (entity, name, sex) in [
        (dana, "Dana", Sex::Female),
        (eli, "Eli", Sex::Male),
        (fern, "Fern", Sex::Female),
        (kid, "Kit", Sex::Male),
    ] {
        world
            .insert(
                entity,
                Identity {
                    given_name: name.to_owned(),
                    family_name: "Vale".to_owned(),
                    sex,
                    birth_tick: 0,
                },
            )
            .expect("insert");
    }
    let day = |d: u64| Ticks::new(d * 1440);

    // Labor chain: fired day 2, rehired day 5.
    world.begin_tick(day(2));
    world
        .emit(&Fired {
            citizen: dana,
            employer: firm,
            reason: FiredReason::Insolvent,
        })
        .expect("emit");
    world.begin_tick(Ticks::new(2 * 1440 + 1));
    world.begin_tick(day(5));
    world
        .emit(&Hired {
            citizen: dana,
            employer: firm,
            wage_per_day: Money::from_mills(200),
        })
        .expect("emit");
    world.begin_tick(Ticks::new(5 * 1440 + 1));
    // Money chain: eli defaults day 6, rents again day 9.
    world.begin_tick(day(6));
    world
        .emit(&LoanDefaulted {
            borrower: eli,
            written_off: Money::from_mills(500),
        })
        .expect("emit");
    world.begin_tick(Ticks::new(6 * 1440 + 1));
    world.begin_tick(day(9));
    world
        .emit(&TenancyStarted {
            tenant: eli,
            home,
            rent_per_day: Money::from_mills(60),
        })
        .expect("emit");
    world.begin_tick(Ticks::new(9 * 1440 + 1));
    // Remarriage attribution: Dana marries Eli (day 10); Eli dies
    // (implicitly); Dana remarries Fern is same-sex — use male Kit's
    // parents instead: Dana+Eli married day 10; Dana+Fern married day
    // 12 (the widow remarried); the child born day 14 to (Fern, Dana)
    // must attach to the SECOND marriage's line only.
    world.begin_tick(day(10));
    world
        .emit(&Married {
            partner_a: dana,
            partner_b: eli,
        })
        .expect("emit");
    world.begin_tick(Ticks::new(10 * 1440 + 1));
    world.begin_tick(day(12));
    world
        .emit(&Married {
            partner_a: dana,
            partner_b: fern,
        })
        .expect("emit");
    world.begin_tick(Ticks::new(12 * 1440 + 1));
    world.begin_tick(day(14));
    world
        .emit(&Born {
            child: kid,
            parent_a: dana,
            parent_b: fern,
        })
        .expect("emit");
    world.begin_tick(Ticks::new(14 * 1440 + 1));

    let lines = debug_tools::stories(&world, None).expect("compose");
    assert!(
        lines.iter().any(|line| line.contains("Dana Vale")
            && line.contains("lost their job on day 2")
            && line.contains("found new work by day 5")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("Eli Vale")
            && line.contains("defaulted on day 6")
            && line.contains("renting again by day 9")),
        "{lines:?}"
    );
    let first_marriage = lines
        .iter()
        .find(|line| line.contains("married on day 10"))
        .expect("first marriage line");
    assert!(
        !first_marriage.contains("Kit Vale"),
        "the child must NOT attach to the earlier marriage: {first_marriage}"
    );
    let second_marriage = lines
        .iter()
        .find(|line| line.contains("married on day 12"))
        .expect("second marriage line");
    assert!(
        second_marriage.contains("Kit Vale") && second_marriage.contains("born on day 14"),
        "the child attaches to the marriage whose couple matches: {second_marriage}"
    );
}

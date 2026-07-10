//! Phase 10 exit-criteria suite (SPEC §15): an observer can follow one
//! citizen's week and reconstruct their story entirely from tooling.
//! The test drives ONLY the tooling layer — the exact data structures
//! the widgets render (ADR 0013 §1) — over a real seven-day town.

use core_ecs::sim_interface::Tier;
use core_types::Seed;
use debug_tools::metrics::{CAPACITY, MetricsRegistry};
use embervale_tests::pinned_defs;
use headless::runner::{self, WorldSpec};
use viewer::panes::{self, Overlay};
use viewer::snapshot::{CitizenRow, LocationRow, Snapshot};

const TICKS_PER_DAY: u64 = 1440;

#[test]
fn a_citizens_week_reconstructs_entirely_from_tooling() {
    let defs = pinned_defs();
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(271), 250), &defs).expect("build");
    schedule.enable_timing();
    let mut metrics = MetricsRegistry::new();

    // Day 1 settles jobs and plans; then the observer follows one
    // employed citizen through a week of daily snapshots.
    sim.run_ticks(&mut schedule, TICKS_PER_DAY)
        .expect("a halted run means a daily audit failed");
    let first = Snapshot::capture(
        sim.world(),
        sim.tick().raw(),
        &defs,
        schedule.last_timings(),
        &metrics,
    )
    .expect("capture");
    let followed = first
        .citizens
        .iter()
        .find(|row| row.employer.is_some() && row.home.is_some())
        .expect("an employed, housed citizen exists after day 1");
    let index = followed.index;
    let name = followed.name.clone();

    // The observer finds them by NAME SEARCH (the inspector's entry).
    assert_eq!(
        panes::search(&first, &name).first().copied(),
        Some(index),
        "search by name focuses the citizen"
    );
    assert_eq!(
        panes::search(&first, &index.to_string()).first().copied(),
        Some(index),
        "search by id focuses the citizen"
    );

    // A week of life, one snapshot per day at noon (mid-activity), all
    // through the tooling layer.
    let mut positions = std::collections::BTreeSet::new();
    let mut actions = std::collections::BTreeSet::new();
    let mut wallets = Vec::new();
    let mut days_with_decisions = 0;
    let mut days_present = 0;
    sim.run_ticks(&mut schedule, TICKS_PER_DAY / 2)
        .expect("run");
    for sample in 0..14 {
        // Noon and evening each day: the observer sees the shift AND
        // the life around it.
        let snapshot = Snapshot::capture(
            sim.world(),
            sim.tick().raw(),
            &defs,
            schedule.last_timings(),
            &metrics,
        )
        .expect("capture");
        if sample % 2 == 0 {
            metrics.sample(sim.world(), snapshot.day).expect("sample");
        }
        // Follow mode re-resolves the focus against every snapshot.
        assert_eq!(
            panes::follow_focus(&snapshot, Some(index)),
            Some(index),
            "follow keeps the living citizen focused"
        );
        if let Some(view) = panes::inspect(&snapshot, index) {
            days_present += 1;
            if let Some(at) = view.row.at {
                positions.insert(at);
            }
            if let Some(action) = &view.row.action {
                // The action label's VARIANT is the observed activity.
                let variant = action
                    .split(['{', '('])
                    .next()
                    .unwrap_or(action)
                    .trim()
                    .to_owned();
                actions.insert(variant);
            }
            wallets.push(view.row.cash_mills + view.row.deposit_mills);
            if view
                .row
                .last_decision
                .as_ref()
                .is_some_and(|(_, candidates)| !candidates.is_empty())
            {
                days_with_decisions += 1;
            }
            // The overlays classify them every day without panicking,
            // and the map places their location in a district column.
            let _ = panes::overlay_class(&snapshot, Overlay::Wealth, &view.row);
            let _ = panes::overlay_class(&snapshot, Overlay::Need(0), &view.row);
            let _ = panes::overlay_class(&snapshot, Overlay::Tier, &view.row);
            if let Some(at) = view.row.at {
                assert!(
                    panes::map_columns(&snapshot)
                        .iter()
                        .any(|(_, column)| column.contains(&at)),
                    "the citizen's location renders on the map"
                );
                assert!(
                    panes::present_at(&snapshot, at).contains(&index),
                    "the map's dot list shows them where they stand"
                );
            }
        }
        // Alternate 9-hour and 15-hour strides: noon → 21:00 → noon.
        let stride = if sample % 2 == 0 { 540 } else { 900 };
        sim.run_ticks(&mut schedule, stride).expect("run");
    }
    let last = Snapshot::capture(
        sim.world(),
        sim.tick().raw(),
        &defs,
        schedule.last_timings(),
        &metrics,
    )
    .expect("capture");

    // --- The week reconstructs -----------------------------------------
    assert!(
        days_present >= 12,
        "the citizen was observable nearly every sample ({days_present}/14)"
    );
    assert!(
        positions.len() >= 2,
        "their position moved across the week ({} places)",
        positions.len()
    );
    assert!(
        actions.len() >= 2,
        "distinct activities were observed ({actions:?})"
    );
    assert_eq!(wallets.len(), days_present, "their finances tracked daily");
    assert!(
        days_with_decisions >= 10,
        "the decision dump carried scored candidates most samples \
         ({days_with_decisions}/14)"
    );

    // The event browser attributes real events to them — including
    // their ECONOMIC life (purchases, wage-flow taxes, tenancy): the
    // week is reconstructible, not just observable.
    let theirs = viewer::events::filter(&last.events, Some(index), None, None);
    assert!(
        !theirs.is_empty(),
        "the week left attributable events in the browser's window"
    );
    assert!(
        theirs.iter().all(|row| row.subjects.contains(&index)),
        "every filtered row genuinely cites the citizen"
    );
    assert!(
        theirs.iter().any(|row| row.kind.starts_with("econ.")),
        "their economic life (purchases/taxes/tenancy) is attributable, \
         found kinds: {:?}",
        theirs
            .iter()
            .map(|row| row.kind.as_str())
            .collect::<std::collections::BTreeSet<_>>()
    );
    // Type and tick-range filters compose.
    let purchases = viewer::events::filter(
        &last.events,
        Some(index),
        Some("econ.goods_purchased"),
        Some((0, last.tick)),
    );
    for row in &purchases {
        assert_eq!(row.kind, "econ.goods_purchased");
    }

    // The dashboard's series travel INSIDE the snapshot (ADR 0013 §2):
    // a real week of every registered metric, read where the pane
    // reads it.
    assert!(!last.metrics.is_empty());
    for (name, samples) in &last.metrics {
        assert!(samples.len() >= 7, "series `{name}` sampled the whole week");
        assert!(
            panes::normalize_series(samples)
                .iter()
                .all(|(x, y)| (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y)),
            "series `{name}` normalizes into the unit square"
        );
    }
    let population = last
        .metrics
        .iter()
        .find(|(name, _)| name == "population")
        .expect("population series");
    assert!(population.1.iter().all(|(_, value)| *value > 0));

    // The narrative view sits on the same window (SPEC §13): whenever
    // the window still holds a chain-source fact, the composer says so
    // (a quiet week whose fired/married facts rotated out of the ring
    // honestly reads as no stories — the composer never invents).
    let chain_sources = [
        "econ.fired",
        "social.married",
        "people.born",
        "econ.loan_defaulted",
    ];
    if last
        .events
        .iter()
        .any(|row| chain_sources.contains(&row.kind.as_str()))
    {
        assert!(
            !last.stories.is_empty(),
            "chain-source facts are in the window, so the composer speaks"
        );
        // Every line carries subject indices the inspector filters by.
        assert!(
            last.stories
                .iter()
                .all(|(subjects, _)| !subjects.is_empty()),
            "every story line is attributable by entity index"
        );
    }
    // The timing table renders (per-system rows, ADR 0013 §5).
    assert!(
        !last.timings.is_empty(),
        "the timing table carries per-system rows"
    );
}

/// The pane builders' pure semantics, bitten once each (ADR 0013 §7).
#[test]
fn pane_builders_classify_filter_and_lay_out_deterministically() {
    let defs = pinned_defs();
    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(277), 150), &defs).expect("build");
    sim.run_ticks(&mut schedule, TICKS_PER_DAY).expect("run");
    let snapshot = Snapshot::capture(
        sim.world(),
        sim.tick().raw(),
        &defs,
        &[],
        &MetricsRegistry::new(),
    )
    .expect("capture");

    // Search: empty query → nothing; nonsense → nothing.
    assert!(panes::search(&snapshot, "").is_empty());
    assert!(panes::search(&snapshot, "zzz-no-such-name").is_empty());

    // Overlay classes stay in band for every citizen and mode.
    for citizen in &snapshot.citizens {
        for overlay in [
            Overlay::Wealth,
            Overlay::Need(0),
            Overlay::Tier,
            Overlay::Price,
        ] {
            assert!(panes::overlay_class(&snapshot, overlay, citizen) <= 4);
        }
    }
    // The price overlay classifies every PRICED location, in band.
    for location in &snapshot.locations {
        let class = panes::location_overlay_class(&snapshot, Overlay::Price, location);
        assert_eq!(class.is_some(), location.price_mills.is_some());
        assert!(class.unwrap_or(0) <= 4);
        assert_eq!(
            panes::location_overlay_class(&snapshot, Overlay::Wealth, location),
            None,
            "only the price overlay colors locations"
        );
    }

    // The map lays out EVERY location exactly once, deterministically.
    let columns = panes::map_columns(&snapshot);
    let placed: usize = columns.iter().map(|(_, column)| column.len()).sum();
    assert_eq!(placed, snapshot.locations.len());
    assert_eq!(columns.len(), snapshot.districts.len() + 1);
    assert_eq!(panes::map_columns(&snapshot), columns, "layout is stable");

    // Commute flows only cite real districts.
    for (home, work, count) in panes::commute_flows(&snapshot) {
        assert!((home as usize) < snapshot.districts.len());
        assert!((work as usize) < snapshot.districts.len());
        assert!(count > 0);
    }

    // Locations are inspectable: a workplace shows its employees.
    let employer = snapshot
        .citizens
        .iter()
        .find_map(|citizen| citizen.employer)
        .expect("someone is employed after day 1");
    let view = panes::inspect_location(&snapshot, employer).expect("the employer inspects");
    assert_eq!(view.row.index, employer);
    assert!(
        !view.employees.is_empty(),
        "the workplace lists its employees"
    );
    assert!(
        view.employees.iter().all(|employee| snapshot
            .citizens
            .iter()
            .any(|c| c.index == *employee && c.employer == Some(employer))),
        "every listed employee genuinely works there"
    );
    assert!(
        panes::inspect_location(&snapshot, u32::MAX).is_none(),
        "a missing location inspects to None"
    );

    // Event filters are conjunctive.
    let all = viewer::events::filter(&snapshot.events, None, None, None);
    let ranged = viewer::events::filter(&snapshot.events, None, None, Some((0, snapshot.tick)));
    assert_eq!(all.len(), ranged.len());
}

fn bare_citizen(index: u32, cash_mills: i64, need: i64) -> CitizenRow {
    CitizenRow {
        index,
        name: format!("citizen {index}"),
        at: None,
        tier: Tier::A,
        needs: vec![need],
        cash_mills,
        deposit_mills: 0,
        employer: None,
        home: None,
        rent_mills: None,
        action: None,
        sleep_window: None,
        skills: Vec::new(),
        bonds: Vec::new(),
        believed: Vec::new(),
        last_decision: None,
    }
}

fn bare_snapshot(citizens: Vec<CitizenRow>, locations: Vec<LocationRow>) -> Snapshot {
    Snapshot {
        tick: 0,
        day: 0,
        districts: vec!["old_town".to_owned(), "riverside".to_owned()],
        need_names: vec!["hunger".to_owned()],
        citizens,
        locations,
        events: Vec::new(),
        stories: Vec::new(),
        district_rent_asks: Vec::new(),
        treasury_receipts_mills: (0, 0),
        metrics: Vec::new(),
        timings: Vec::new(),
    }
}

/// The classification edge cases the widgets must not lie about
/// (Phase 10 review findings — each was a real defect once).
#[test]
fn overlay_and_pane_edge_semantics() {
    // A uniform-wealth town is all class 0, never all "richest".
    let uniform = bare_snapshot(
        (0..10).map(|i| bare_citizen(i, 500, 0)).collect(),
        Vec::new(),
    );
    for citizen in &uniform.citizens {
        assert_eq!(panes::overlay_class(&uniform, Overlay::Wealth, citizen), 0);
    }
    // Distinct wealths span the full palette.
    let spread = bare_snapshot(
        (0..10)
            .map(|i| bare_citizen(i, i64::from(i) * 100, 0))
            .collect(),
        Vec::new(),
    );
    assert_eq!(
        panes::overlay_class(&spread, Overlay::Wealth, &spread.citizens[0]),
        0
    );
    assert_eq!(
        panes::overlay_class(&spread, Overlay::Wealth, &spread.citizens[9]),
        4
    );

    // The need heatmap's TOP band is reachable at a full need, and the
    // bottom at zero (the clamped scale is 0..=1_000_000).
    let needs = bare_snapshot(
        vec![bare_citizen(0, 0, 0), bare_citizen(1, 0, 1_000_000)],
        Vec::new(),
    );
    assert_eq!(
        panes::overlay_class(&needs, Overlay::Need(0), &needs.citizens[0]),
        0
    );
    assert_eq!(
        panes::overlay_class(&needs, Overlay::Need(0), &needs.citizens[1]),
        4
    );
    // An out-of-range need index classifies cold, never panics.
    assert_eq!(
        panes::overlay_class(&needs, Overlay::Need(99), &needs.citizens[1]),
        0
    );

    // A corrupt district id never reaches the commute pane, and the map
    // files the location under "(unsited)" — the panes agree.
    let mut corrupt = bare_snapshot(
        vec![bare_citizen(0, 0, 0)],
        vec![LocationRow {
            index: 7,
            kind: "hovel".to_owned(),
            district: Some(9_999),
            price_mills: None,
            stock: None,
        }],
    );
    corrupt.citizens[0].home = Some(7);
    corrupt.citizens[0].employer = Some(7);
    assert!(panes::commute_flows(&corrupt).is_empty());
    let columns = panes::map_columns(&corrupt);
    assert!(
        columns
            .last()
            .is_some_and(|(name, column)| { name == "(unsited)" && column.contains(&7) })
    );

    // follow_focus: present → kept; despawned → None; nothing → None.
    let town = bare_snapshot(vec![bare_citizen(3, 0, 0)], Vec::new());
    assert_eq!(panes::follow_focus(&town, Some(3)), Some(3));
    assert_eq!(panes::follow_focus(&town, Some(4)), None);
    assert_eq!(panes::follow_focus(&town, None), None);

    // The relationship graph resolves bonds against the SNAPSHOT: a
    // living friend carries their position; a despawned one is named
    // honestly and unplaced.
    let mut social = bare_snapshot(
        vec![bare_citizen(0, 0, 0), bare_citizen(1, 0, 0)],
        Vec::new(),
    );
    social.citizens[1].at = Some(42);
    social.citizens[0].bonds = vec![
        (1, "Friend".to_owned(), 400),
        (77, "Spouse".to_owned(), 900),
    ];
    let graph = panes::relationship_graph(&social, 0);
    assert_eq!(graph.len(), 2);
    assert_eq!(graph[0].other, 1);
    assert_eq!(graph[0].other_at, Some(42));
    assert_eq!(graph[0].other_name, "citizen 1");
    assert_eq!(graph[1].other, 77);
    assert_eq!(graph[1].other_at, None);
    assert_eq!(graph[1].other_name, "citizen #77");
    assert!(panes::relationship_graph(&social, 99).is_empty());

    // Sparkline normalization: unit square, flat series at mid-height.
    assert!(panes::normalize_series(&[]).is_empty());
    let flat = panes::normalize_series(&[(1, 5), (2, 5), (3, 5)]);
    assert!(flat.iter().all(|(_, y)| *y == 0.5));
    let ramp = panes::normalize_series(&[(0, 0), (10, 100)]);
    assert_eq!(ramp, vec![(0.0, 0.0), (1.0, 1.0)]);
}

/// The metrics registry itself: extractors read the real world, and
/// series stay bounded at [`CAPACITY`] with oldest-first eviction.
#[test]
fn metrics_registry_samples_and_evicts() {
    let defs = pinned_defs();
    let (sim, _) =
        runner::build_simulation(&WorldSpec::town(Seed::new(281), 40), &defs).expect("build");
    let mut metrics = MetricsRegistry::new();
    metrics.sample(sim.world(), 0).expect("sample");
    let population = metrics
        .series()
        .iter()
        .find(|series| series.name == "population")
        .expect("population series");
    assert_eq!(
        population.samples,
        vec![(0, 40)],
        "the population extractor counts the town"
    );

    // Overfill: the ring keeps the newest CAPACITY days.
    let extra = 8u64;
    for day in 1..(CAPACITY as u64 + extra) {
        metrics.sample(sim.world(), day).expect("sample");
    }
    for series in metrics.series() {
        assert_eq!(series.samples.len(), CAPACITY, "series `{}`", series.name);
        assert_eq!(
            series.samples.first().map(|(day, _)| *day),
            Some(extra),
            "the OLDEST days were evicted from `{}`",
            series.name
        );
    }
}

//! Loader/validation test suite (split from `lib.rs` for the SPEC §3
//! module-size rule).

use super::*;
use std::path::PathBuf;

const GOOD_CALENDAR: &str = "CalendarConfig(days_per_season: 30)";
const GOOD_ENGINE: &str = "EngineConfig(event_log_capacity: 4096)";
const GOOD_NEEDS: &str = r#"NeedsConfig(needs: [
        NeedDef(id: "hunger", decay_per_hour: 14600, initial_min: 550000, initial_max: 1000000),
    ])"#;
const GOOD_TRAITS: &str = r#"TraitsConfig(traits: [
        TraitDef(id: "ambition", min: 20, max: 990),
    ])"#;
const GOOD_MORTALITY: &str = r#"MortalityConfig(
        bands: [MortalityBand(max_age_years: 60, per_day_chance_per_billion: 60000)],
        terminal_per_day_chance_per_billion: 5000000,
    )"#;
const GOOD_DEMOGRAPHICS: &str = r#"DemographicsConfig(
        age_bands: [
            AgeBand(min_age_years: 0, max_age_years: 40, weight_per_mille: 600),
            AgeBand(min_age_years: 41, max_age_years: 90, weight_per_mille: 400),
        ],
        male_per_mille: 505,
        household_min: 1,
        household_max: 6,
        annual_death_rate_min_per_mille: 6,
        annual_death_rate_max_per_mille: 40,
        wealth_min_mills: 10000,
        wealth_max_mills: 20000,
    )"#;
const GOOD_NAMES: &str = r#"NameList(names: ["A", "B"])"#;
const GOOD_LOCATIONS: &str = r#"LocationsConfig(kinds: [
        LocationKindDef(id: "home", is_home: true, count: 0, satisfies: [
            SatisfierDef(need_id: "hunger", per_tick: 2000),
        ]),
        LocationKindDef(id: "tavern", is_home: false, count: 2, satisfies: [
            SatisfierDef(need_id: "hunger", per_tick: 9000),
        ]),
        LocationKindDef(id: "shop", is_home: false, count: 0, satisfies: []),
    ])"#;
const GOOD_GOODS: &str = r#"GoodsConfig(goods: [
        GoodDef(id: "bread", spoil_per_mille: 100),
    ])"#;
const GOOD_RECIPES: &str = r#"RecipesConfig(recipes: [
        RecipeDef(id: "bake", inputs: [], output: GoodQty(good_id: "bread", quantity: 4), batch_hours: 1),
    ])"#;
const GOOD_FIRMS: &str = r#"FirmsConfig(kinds: [
        FirmDef(id: "bakery", count: 1, recipe_id: "bake", initial_cash_mills: 1000,
            initial_inventory: [GoodQty(good_id: "bread", quantity: 8)], initial_price_mills: 50,
            location_kind_id: "shop", positions: 2, min_workers: 1,
            retail: Some(RetailDef(need_id: "hunger", gain_per_unit: 100000, use_ticks: 5))),
    ])"#;
const GOOD_LABOR: &str = r#"LaborConfig(
        shift_start_hour: 9, shift_end_hour: 17, min_working_age_years: 16,
        reservation_base_mills: 150, reservation_wealth_per_mille: 300,
        reservation_half_wealth_mills: 20000, reservation_trait_id: "ambition",
        reservation_trait_discount_per_mille: 400, bid_fraction_per_mille: 600,
        work_bias_micro: 600000, work_ticks: 60, work_need_id: "hunger",
        work_need_per_tick: 1000,
    )"#;
const GOOD_ECONOMY: &str = r#"EconomyConfig(
        markup_per_mille: 300, overhead_mills_per_batch: 100,
        controller_step_per_mille: 50, inventory_target_batches: 10,
        min_price_mills: 1, max_price_mills: 5000,
    )"#;
const GOOD_AI: &str = r#"AiConfig(
        travel_ticks: 10, urgency_exponent: 2, time_cost_micro_per_tick: 300,
        max_perform_ticks: 200, idle_ticks: 15, plan_compile_hour: 21,
        sleep: SleepDef(base_start_hour: 22, base_end_hour: 6, max_shift_minutes: 60,
            shift_trait_id: "ambition", rest_need_id: "hunger", home_bias_micro: 100000),
        purchase: PurchaseDef(mu_scale_micro: 250, half_wealth_mills: 20000),
        need_trait_weights: [
            NeedTraitWeight(need_id: "hunger", trait_id: "ambition", weight_per_mille: 500),
        ],
    )"#;

/// Writes a fully valid data tree, then applies `overrides`
/// (path -> replacement content; empty content deletes the file).
fn write_tree(overrides: &[(&str, &str)]) -> PathBuf {
    let root = std::env::temp_dir()
        .join("embervale-data-defs-tests")
        .join(format!("{:?}", std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&root);
    let base: &[(&str, &str)] = &[
        ("balance/calendar.ron", GOOD_CALENDAR),
        ("balance/engine.ron", GOOD_ENGINE),
        ("balance/needs.ron", GOOD_NEEDS),
        ("balance/traits.ron", GOOD_TRAITS),
        ("balance/mortality.ron", GOOD_MORTALITY),
        ("balance/demographics.ron", GOOD_DEMOGRAPHICS),
        ("names/given_female.ron", GOOD_NAMES),
        ("names/given_male.ron", GOOD_NAMES),
        ("names/family.ron", GOOD_NAMES),
        ("locations.ron", GOOD_LOCATIONS),
        ("balance/ai.ron", GOOD_AI),
        ("goods.ron", GOOD_GOODS),
        ("recipes.ron", GOOD_RECIPES),
        ("firms.ron", GOOD_FIRMS),
        ("balance/economy.ron", GOOD_ECONOMY),
        ("balance/labor.ron", GOOD_LABOR),
    ];
    for (rel, content) in base {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().expect("has parent")).expect("mkdir");
        std::fs::write(path, content).expect("write");
    }
    for (rel, content) in overrides {
        let path = root.join(rel);
        if content.is_empty() {
            std::fs::remove_file(&path).expect("rm");
        } else {
            std::fs::write(path, content).expect("write");
        }
    }
    root
}

#[test]
fn valid_data_loads() {
    let defs = load(&write_tree(&[])).expect("valid data must load");
    assert_eq!(defs.calendar.days_per_season, 30);
    assert_eq!(defs.engine.event_log_capacity, 4096);
    assert_eq!(defs.people.needs.needs.len(), 1);
    assert_eq!(defs.people.mortality.per_day_chance(61), 5_000_000);
    let tables = resolve_ai(&defs);
    assert_eq!(tables.kind_is_home, vec![true, false, false]);
    assert_eq!(tables.kind_satisfiers[1], vec![(0, 9000)]);
    assert_eq!(tables.rest_need, 0);
    assert_eq!(tables.need_trait[0], Some((0, 500)));
    assert_eq!(tables.mu_scale_micro, 250);
    assert_eq!(tables.half_wealth_mills, 20000);

    let econ = resolve_economy(&defs);
    assert_eq!(econ.goods, 1);
    assert_eq!(econ.spoil_per_mille, vec![100]);
    assert_eq!(econ.recipes.len(), 1);
    assert_eq!(econ.recipes[0].output_good, 0);
    assert_eq!(econ.recipes[0].output_quantity, 4);
    let bakery = &econ.firm_kinds[0];
    assert_eq!(bakery.initial_inventory, vec![8]);
    assert_eq!(bakery.initial_cash.mills(), 1000);
    assert_eq!(bakery.location_kind, 2, "shop is location kind 2");
    assert_eq!((bakery.positions, bakery.min_workers), (2, 1));
    // Retail resolves to (need 0 = hunger, gain, use_ticks).
    assert_eq!(bakery.retail, Some((0, 100000, 5)));
    assert_eq!(econ.labor.shift_start_hour, 9);
    assert_eq!(econ.labor.reservation_trait, 0, "ambition is trait 0");
    assert_eq!(tables.work_start_minute, 9 * 60);
    assert_eq!(tables.work_need, 0);
}

/// ADR 0007 §7: seeded errors across goods/recipes/firms/economy are
/// caught with precise messages, including every cross-file reference.
#[test]
fn economy_validation_catches_seeded_errors() {
    // A recipe referencing an unknown good.
    let root = write_tree(&[(
        "recipes.ron",
        r#"RecipesConfig(recipes: [
            RecipeDef(id: "bake", inputs: [GoodQty(good_id: "unobtanium", quantity: 1)],
                output: GoodQty(good_id: "bread", quantity: 4), batch_hours: 1),
        ])"#,
    )]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("unknown good"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }

    // A good nothing produces.
    let root = write_tree(&[(
        "goods.ron",
        r#"GoodsConfig(goods: [
            GoodDef(id: "bread", spoil_per_mille: 100),
            GoodDef(id: "caviar", spoil_per_mille: 100),
        ])"#,
    )]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("no firm kind produces"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }

    // A firm's location kind that world genesis would also instantiate.
    let root = write_tree(&[(
        "locations.ron",
        r#"LocationsConfig(kinds: [
            LocationKindDef(id: "home", is_home: true, count: 0, satisfies: [
                SatisfierDef(need_id: "hunger", per_tick: 2000),
            ]),
            LocationKindDef(id: "shop", is_home: false, count: 3, satisfies: []),
        ])"#,
    )]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("count 0"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }

    // min_workers above positions.
    let root = write_tree(&[(
        "firms.ron",
        r#"FirmsConfig(kinds: [
            FirmDef(id: "bakery", count: 1, recipe_id: "bake", initial_cash_mills: 1000,
                initial_inventory: [], initial_price_mills: 50,
                location_kind_id: "shop", positions: 2, min_workers: 3,
                retail: None),
        ])"#,
    )]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("min_workers"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }

    // Labor: an overnight shift is rejected.
    let root = write_tree(&[(
        "balance/labor.ron",
        r#"LaborConfig(
            shift_start_hour: 20, shift_end_hour: 4, min_working_age_years: 16,
            reservation_base_mills: 150, reservation_wealth_per_mille: 300,
            reservation_half_wealth_mills: 20000, reservation_trait_id: "ambition",
            reservation_trait_discount_per_mille: 400, bid_fraction_per_mille: 600,
            work_bias_micro: 600000, work_ticks: 60, work_need_id: "hunger",
            work_need_per_tick: 1000,
        )"#,
    )]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("shift_start_hour"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }

    // A dead location kind: count 0, no satisfiers, and no retail firm.
    let root = write_tree(&[(
        "locations.ron",
        r#"LocationsConfig(kinds: [
            LocationKindDef(id: "home", is_home: true, count: 0, satisfies: [
                SatisfierDef(need_id: "hunger", per_tick: 2000),
            ]),
            LocationKindDef(id: "shop", is_home: false, count: 0, satisfies: []),
            LocationKindDef(id: "ruin", is_home: false, count: 0, satisfies: []),
        ])"#,
    )]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("dead data"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }

    // A recipe listing the same input good twice (ADR 0007 §8b).
    let root = write_tree(&[
        (
            "goods.ron",
            r#"GoodsConfig(goods: [
                GoodDef(id: "bread", spoil_per_mille: 100),
                GoodDef(id: "wheat", spoil_per_mille: 0),
            ])"#,
        ),
        (
            "recipes.ron",
            r#"RecipesConfig(recipes: [
                RecipeDef(id: "bake",
                    inputs: [GoodQty(good_id: "wheat", quantity: 2), GoodQty(good_id: "wheat", quantity: 3)],
                    output: GoodQty(good_id: "bread", quantity: 4), batch_hours: 1),
            ])"#,
        ),
    ]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("more than once"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }

    // Controller step out of range.
    let root = write_tree(&[(
        "balance/economy.ron",
        r#"EconomyConfig(
            markup_per_mille: 300, overhead_mills_per_batch: 100,
            controller_step_per_mille: 0, inventory_target_batches: 10,
            min_price_mills: 1, max_price_mills: 5000,
        )"#,
    )]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("controller_step_per_mille"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }
}

/// ADR 0006 §7: seeded errors in locations.ron and ai.ron are caught
/// with precise messages.
#[test]
fn location_and_ai_validation_catches_seeded_errors() {
    // Two home kinds.
    let root = write_tree(&[(
        "locations.ron",
        r#"LocationsConfig(kinds: [
                LocationKindDef(id: "a", is_home: true, count: 0, satisfies: [SatisfierDef(need_id: "hunger", per_tick: 1)]),
                LocationKindDef(id: "b", is_home: true, count: 0, satisfies: [SatisfierDef(need_id: "hunger", per_tick: 1)]),
            ])"#,
    )]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("exactly one home kind"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }

    // Unknown need in a satisfier.
    let root = write_tree(&[(
        "locations.ron",
        r#"LocationsConfig(kinds: [
                LocationKindDef(id: "home", is_home: true, count: 0, satisfies: [SatisfierDef(need_id: "hungerz", per_tick: 1)]),
            ])"#,
    )]);
    assert!(matches!(load(&root), Err(DataError::Validation { .. })));

    // AI referencing an unknown trait.
    let root = write_tree(&[(
        "balance/ai.ron",
        r#"AiConfig(
                travel_ticks: 10, urgency_exponent: 2, time_cost_micro_per_tick: 300,
                max_perform_ticks: 200, idle_ticks: 15, plan_compile_hour: 21,
                sleep: SleepDef(base_start_hour: 22, base_end_hour: 6, max_shift_minutes: 60,
                    shift_trait_id: "nonexistent", rest_need_id: "hunger", home_bias_micro: 1),
                purchase: PurchaseDef(mu_scale_micro: 250, half_wealth_mills: 20000),
                need_trait_weights: [],
            )"#,
    )]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("shift_trait_id"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }

    // No location satisfies the rest need.
    let root = write_tree(&[
        (
            "locations.ron",
            r#"LocationsConfig(kinds: [
                LocationKindDef(id: "home", is_home: true, count: 0, satisfies: [SatisfierDef(need_id: "hunger", per_tick: 1)]),
            ])"#,
        ),
        (
            "balance/needs.ron",
            r#"NeedsConfig(needs: [
                NeedDef(id: "hunger", decay_per_hour: 14600, initial_min: 550000, initial_max: 1000000),
                NeedDef(id: "rest", decay_per_hour: 26000, initial_min: 500000, initial_max: 1000000),
            ])"#,
        ),
        (
            "balance/ai.ron",
            r#"AiConfig(
                travel_ticks: 10, urgency_exponent: 2, time_cost_micro_per_tick: 300,
                max_perform_ticks: 200, idle_ticks: 15, plan_compile_hour: 21,
                sleep: SleepDef(base_start_hour: 22, base_end_hour: 6, max_shift_minutes: 60,
                    shift_trait_id: "ambition", rest_need_id: "rest", home_bias_micro: 1),
                purchase: PurchaseDef(mu_scale_micro: 250, half_wealth_mills: 20000),
                need_trait_weights: [],
            )"#,
        ),
    ]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("no location kind satisfies"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }
}

#[test]
fn missing_file_names_the_path() {
    let root = write_tree(&[("balance/engine.ron", "")]);
    match load(&root) {
        Err(DataError::Io { path, .. }) => {
            assert!(path.ends_with("balance/engine.ron"), "{path:?}");
        }
        other => panic!("expected Io error, got {other:?}"),
    }
    let root = write_tree(&[("names/family.ron", "")]);
    assert!(matches!(load(&root), Err(DataError::Io { .. })));
}

#[test]
fn unknown_field_is_a_parse_error_not_a_silent_default() {
    let root = write_tree(&[(
        "balance/calendar.ron",
        "CalendarConfig(days_per_season: 30, dayz_per_saeson: 12)",
    )]);
    assert!(matches!(load(&root), Err(DataError::Parse { .. })));
}

#[test]
fn syntax_error_is_a_parse_error() {
    let root = write_tree(&[("balance/calendar.ron", "CalendarConfig(days_per_season:")]);
    assert!(matches!(load(&root), Err(DataError::Parse { .. })));
}

#[test]
fn out_of_range_values_are_validation_errors_with_precise_messages() {
    let root = write_tree(&[("balance/calendar.ron", "CalendarConfig(days_per_season: 0)")]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("days_per_season"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }

    let root = write_tree(&[("balance/engine.ron", "EngineConfig(event_log_capacity: 0)")]);
    assert!(matches!(load(&root), Err(DataError::Validation { .. })));
}

#[test]
fn people_validation_catches_seeded_errors() {
    // Age-band weights not summing to 1000.
    let root = write_tree(&[(
        "balance/demographics.ron",
        r#"DemographicsConfig(
                age_bands: [AgeBand(min_age_years: 0, max_age_years: 90, weight_per_mille: 900)],
                male_per_mille: 505, household_min: 1, household_max: 6,
                annual_death_rate_min_per_mille: 6, annual_death_rate_max_per_mille: 40,
                wealth_min_mills: 10000, wealth_max_mills: 20000,
            )"#,
    )]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("sum to 1000"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }

    // Non-ascending mortality bands.
    let root = write_tree(&[(
        "balance/mortality.ron",
        r#"MortalityConfig(
                bands: [
                    MortalityBand(max_age_years: 60, per_day_chance_per_billion: 1),
                    MortalityBand(max_age_years: 60, per_day_chance_per_billion: 2),
                ],
                terminal_per_day_chance_per_billion: 3,
            )"#,
    )]);
    assert!(matches!(load(&root), Err(DataError::Validation { .. })));

    // Inverted need range.
    let root = write_tree(&[(
        "balance/needs.ron",
        r#"NeedsConfig(needs: [
                NeedDef(id: "hunger", decay_per_hour: 1, initial_min: 900000, initial_max: 100000),
            ])"#,
    )]);
    assert!(matches!(load(&root), Err(DataError::Validation { .. })));

    // Empty name list.
    let root = write_tree(&[("names/family.ron", "NameList(names: [])")]);
    assert!(matches!(load(&root), Err(DataError::Validation { .. })));

    // Trait range out of per-mille bounds.
    let root = write_tree(&[(
        "balance/traits.ron",
        r#"TraitsConfig(traits: [TraitDef(id: "x", min: 500, max: 1500)])"#,
    )]);
    assert!(matches!(load(&root), Err(DataError::Validation { .. })));

    // Inverted wealth seed range (ADR 0007 §8b).
    let root = write_tree(&[(
        "balance/demographics.ron",
        r#"DemographicsConfig(
                age_bands: [AgeBand(min_age_years: 0, max_age_years: 90, weight_per_mille: 1000)],
                male_per_mille: 505, household_min: 1, household_max: 6,
                annual_death_rate_min_per_mille: 6, annual_death_rate_max_per_mille: 40,
                wealth_min_mills: 50000, wealth_max_mills: 20000,
            )"#,
    )]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("wealth seed range"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }

    // Negative wealth bound.
    let root = write_tree(&[(
        "balance/demographics.ron",
        r#"DemographicsConfig(
                age_bands: [AgeBand(min_age_years: 0, max_age_years: 90, weight_per_mille: 1000)],
                male_per_mille: 505, household_min: 1, household_max: 6,
                annual_death_rate_min_per_mille: 6, annual_death_rate_max_per_mille: 40,
                wealth_min_mills: -1, wealth_max_mills: 20000,
            )"#,
    )]);
    assert!(matches!(load(&root), Err(DataError::Validation { .. })));

    // Mortality probability above 1 (per-billion > 1e9).
    let root = write_tree(&[(
        "balance/mortality.ron",
        r#"MortalityConfig(
                bands: [MortalityBand(max_age_years: 60, per_day_chance_per_billion: 2000000000)],
                terminal_per_day_chance_per_billion: 3,
            )"#,
    )]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("probability > 1"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }
}

/// ADR 0005 §6 cross-check: ages × calendar year length must fit tick
/// arithmetic; extreme-but-parseable data is rejected at load, never
/// allowed to overflow genesis (SPEC §3/§8).
#[test]
fn calendar_age_overflow_is_rejected_at_load() {
    let root = write_tree(&[
        (
            "balance/calendar.ron",
            "CalendarConfig(days_per_season: 500000)",
        ),
        (
            "balance/demographics.ron",
            r#"DemographicsConfig(
                    age_bands: [AgeBand(min_age_years: 0, max_age_years: 4000000000, weight_per_mille: 1000)],
                    male_per_mille: 505, household_min: 1, household_max: 6,
                    annual_death_rate_min_per_mille: 6, annual_death_rate_max_per_mille: 40,
                    wealth_min_mills: 10000, wealth_max_mills: 20000,
                )"#,
        ),
    ]);
    match load(&root) {
        Err(DataError::Validation { message, .. }) => {
            assert!(message.contains("overflows tick arithmetic"), "{message}");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }
}

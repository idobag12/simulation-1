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
        LocationKindDef(id: "town_hall", is_home: false, count: 0, satisfies: []),
        LocationKindDef(id: "school", is_home: false, count: 1, satisfies: []),
    ])"#;
const GOOD_GOODS: &str = r#"GoodsConfig(goods: [
        GoodDef(id: "bread", spoil_per_mille: 100),
    ])"#;
const GOOD_RECIPES: &str = r#"RecipesConfig(recipes: [
        RecipeDef(id: "bake", inputs: [], output: Some(GoodQty(good_id: "bread", quantity: 4)), batch_hours: 1),
    ])"#;
const GOOD_FIRMS: &str = r#"FirmsConfig(kinds: [
        FirmDef(id: "bakery", count: 1, recipe_id: "bake", initial_cash_mills: 1000,
            initial_inventory: [GoodQty(good_id: "bread", quantity: 8)], initial_price_mills: 50,
            location_kind_id: "shop", positions: 2, min_workers: 1,
            retail: Some(RetailDef(need_id: "hunger", gain_per_unit: 100000, use_ticks: 5))),
    ])"#;
const GOOD_BANK: &str = r#"BankConfig(
        equity_seed_mills: 100000, target_cash_float_mills: 2000,
        deposit_spread_per_million_daily: 400, loan_payroll_multiple_per_mille: 5000,
        working_capital_floor_days: 3, serviceability_revenue_per_mille: 200,
        risk_premium_per_million_daily: 400, default_cooldown_days: 30,
        repay_term_days: 60, policy_neutral_per_million_daily: 800,
        policy_target_inflation_per_mille: 0, policy_sensitivity_per_million: 50,
        policy_min_per_million_daily: 100, policy_max_per_million_daily: 5000,
        index_period_days: 5,
    )"#;
const GOOD_HOUSING: &str = r#"HousingConfig(
        upkeep_mills_per_day: 40, rent_margin_per_mille: 250, rent_bid_per_mille: 8,
        rent_vacancy_target_per_mille: 200, rent_step_per_mille: 50,
        purchase_period_days: 10, home_price_mills: 30000, buyer_savings_per_mille: 300,
        home_bid_per_mille: 1500, mortgage_ltv_per_mille: 700,
        construction_margin_per_mille: 300,
    )"#;
const GOOD_SKILLS: &str = r#"SkillsConfig(
        skills: [SkillDef(id: "letters"), SkillDef(id: "craft")],
        school: SchoolDef(taught_skill_id: "letters", gain_per_attendance_per_mille: 8,
            start_age_years: 6, end_age_years: 16, start_hour: 9, end_hour: 14,
            attend_ticks: 120, location_kind_id: "school", attend_bias_micro: 500000),
        doing_gain_per_shift_per_mille: 2, labor_skill_weight_per_mille: 600,
        public_skill_id: "letters",
    )"#;
const GOOD_SOCIAL: &str = r#"SocialConfig(
        drift_need_id: "hunger", spark_trait_id: "ambition",
        edge_cap: 12, friend_drift_per_meeting_per_mille: 30,
        romance_drift_per_meeting_per_mille: 25, decay_per_day_per_mille: 5,
        romance_min_sociability_product_per_mille: 90, marriage_threshold_per_mille: 700,
        social_bond_weight_per_mille: 400, belief_cap: 8,
    )"#;
const GOOD_FERTILITY: &str = r#"FertilityConfig(
        bands: [
            FertilityBand(min_age_years: 16, max_age_years: 24, per_day_chance_per_billion: 9000000),
            FertilityBand(min_age_years: 25, max_age_years: 34, per_day_chance_per_billion: 7000000),
        ],
        max_household_size: 6, trait_mutation_per_mille: 120,
    )"#;
const GOOD_LOD: &str = r#"LodConfig(
        tier_a_cap: 200, tier_b_cap: 2000, highlight_days: 3,
        leisure_hours_per_day: 4, macro_tolerance_per_mille: 150,
    )"#;
const GOOD_TAXES: &str = r#"TaxesConfig(
        income_per_mille: 100, sales_per_mille: 50, treasury_seed_mills: 50000,
        public_positions: 2, public_wage_bid_mills: 200,
        public_location_kind_id: "town_hall",
    )"#;
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
/// Shared with `tests_econ.rs`.
pub(crate) fn write_tree(overrides: &[(&str, &str)]) -> PathBuf {
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
        ("balance/bank.ron", GOOD_BANK),
        ("balance/housing.ron", GOOD_HOUSING),
        ("balance/taxes.ron", GOOD_TAXES),
        ("skills.ron", GOOD_SKILLS),
        ("balance/social.ron", GOOD_SOCIAL),
        ("balance/fertility.ron", GOOD_FERTILITY),
        ("balance/lod.ron", GOOD_LOD),
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
    assert_eq!(tables.kind_is_home, vec![true, false, false, false, false]);
    assert_eq!(tables.kind_satisfiers[1], vec![(0, 9000)]);
    assert_eq!(tables.rest_need, 0);
    assert_eq!(tables.need_trait[0], Some((0, 500)));
    assert_eq!(tables.mu_scale_micro, 250);
    assert_eq!(tables.half_wealth_mills, 20000);

    let econ = resolve_economy(&defs);
    assert_eq!(econ.goods, 1);
    assert_eq!(econ.spoil_per_mille, vec![100]);
    assert_eq!(econ.recipes.len(), 1);
    assert_eq!(econ.recipes[0].output, Some((0, 4)));
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

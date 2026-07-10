//! Phase 5/6 balance validation: labor (ADR 0008 §7) and the money
//! layer (ADR 0009 §7 — bank, housing, taxes). Split from
//! `validate_econ.rs` for the SPEC §3 module-size rule.

use std::path::Path;

use crate::DataError;
use crate::validate::verr;

/// Validates `data/balance/labor.ron` (ADR 0008 §7), including its
/// cross-references into needs and traits.
pub(crate) fn validate_labor(
    data_root: &Path,
    labor: &sim_economy::config::LaborConfig,
    people: &sim_people::config::PeopleConfig,
) -> Result<(), DataError> {
    let file = "balance/labor.ron";
    let e = |message: String| verr(data_root, file, message);

    if labor.shift_start_hour >= 24 || labor.shift_end_hour > 24 {
        return Err(e("shift hours must be within the day".into()));
    }
    if labor.shift_start_hour >= labor.shift_end_hour {
        return Err(e(
            "shift_start_hour must be before shift_end_hour (no overnight shifts)".into(),
        ));
    }
    if labor.reservation_base_mills < 1 {
        return Err(e("reservation_base_mills must be >= 1".into()));
    }
    for (name, value) in [
        (
            "reservation_wealth_per_mille",
            labor.reservation_wealth_per_mille,
        ),
        (
            "reservation_trait_discount_per_mille",
            labor.reservation_trait_discount_per_mille,
        ),
    ] {
        if !(0..=1000).contains(&value) {
            return Err(e(format!("{name} must be within 0..=1000")));
        }
    }
    if labor.reservation_half_wealth_mills < 1 {
        return Err(e("reservation_half_wealth_mills must be >= 1".into()));
    }
    if !(1..=1000).contains(&labor.bid_fraction_per_mille) {
        return Err(e("bid_fraction_per_mille must be within 1..=1000".into()));
    }
    if labor.work_bias_micro < 0 {
        return Err(e("work_bias_micro must be >= 0".into()));
    }
    if labor.work_ticks == 0 {
        return Err(e("work_ticks must be >= 1".into()));
    }
    if labor.work_need_per_tick < 0 {
        return Err(e("work_need_per_tick must be >= 0".into()));
    }
    if !people
        .traits
        .traits
        .iter()
        .any(|t| t.id == labor.reservation_trait_id)
    {
        return Err(e(format!(
            "reservation_trait_id `{}` is not a defined trait",
            labor.reservation_trait_id
        )));
    }
    if !people
        .needs
        .needs
        .iter()
        .any(|n| n.id == labor.work_need_id)
    {
        return Err(e(format!(
            "work_need_id `{}` is not a defined need",
            labor.work_need_id
        )));
    }
    Ok(())
}

/// Validates `data/balance/{bank,housing,taxes}.ron` (ADR 0009 §7).
pub(crate) fn validate_money(
    data_root: &Path,
    bank: &sim_economy::BankConfig,
    housing: &sim_economy::HousingConfig,
    taxes: &sim_economy::TaxesConfig,
    locations: &sim_world::config::LocationsConfig,
) -> Result<(), DataError> {
    let e = |file: &str, message: String| verr(data_root, file, message);

    let b = "balance/bank.ron";
    if bank.equity_seed_mills < 0 || bank.target_cash_float_mills < 0 {
        return Err(e(b, "seed and float must be >= 0".into()));
    }
    for (name, value) in [
        (
            "deposit_spread_per_million_daily",
            bank.deposit_spread_per_million_daily,
        ),
        (
            "risk_premium_per_million_daily",
            bank.risk_premium_per_million_daily,
        ),
        (
            "policy_neutral_per_million_daily",
            bank.policy_neutral_per_million_daily,
        ),
        (
            "policy_sensitivity_per_million",
            bank.policy_sensitivity_per_million,
        ),
    ] {
        if value < 0 {
            return Err(e(b, format!("{name} must be >= 0")));
        }
    }
    if bank.policy_min_per_million_daily < 0
        || bank.policy_min_per_million_daily > bank.policy_max_per_million_daily
    {
        return Err(e(b, "policy bounds must satisfy 0 <= min <= max".into()));
    }
    // Ceilings keep every rate multiply within i64 by construction —
    // an absurd-but-parsable rate must die at load, not halt a run.
    if bank.policy_max_per_million_daily + bank.risk_premium_per_million_daily > 1_000_000 {
        return Err(e(
            b,
            "policy_max + risk_premium must be <= 1_000_000/million/day (100%/day)".into(),
        ));
    }
    if bank.deposit_spread_per_million_daily > 1_000_000 {
        return Err(e(
            b,
            "deposit_spread_per_million_daily must be <= 1_000_000".into(),
        ));
    }
    if bank.target_cash_float_mills < 1 || bank.default_cooldown_days == 0 {
        return Err(e(
            b,
            "the cash float and default cooldown must be >= 1 (zero floats lock              every deposited mill in the vault; zero cooldowns nullify default)"
                .into(),
        ));
    }
    if bank.loan_payroll_multiple_per_mille < 1
        || bank.working_capital_floor_days < 1
        || bank.repay_term_days < 1
        || bank.index_period_days == 0
    {
        return Err(e(
            b,
            "loan multiple, floor days, term, and index period must be >= 1".into(),
        ));
    }
    if !(0..=1000).contains(&bank.serviceability_revenue_per_mille) {
        return Err(e(
            b,
            "serviceability_revenue_per_mille must be within 0..=1000".into(),
        ));
    }

    let h = "balance/housing.ron";
    if housing.upkeep_mills_per_day < 1
        || housing.home_price_mills < 1
        || housing.purchase_period_days == 0
    {
        return Err(e(
            h,
            "upkeep, home price, and purchase period must be >= 1".into(),
        ));
    }
    for (name, value) in [
        ("rent_margin_per_mille", housing.rent_margin_per_mille),
        ("rent_bid_per_mille", housing.rent_bid_per_mille),
        (
            "rent_vacancy_target_per_mille",
            housing.rent_vacancy_target_per_mille,
        ),
        ("rent_step_per_mille", housing.rent_step_per_mille),
        ("buyer_savings_per_mille", housing.buyer_savings_per_mille),
        ("mortgage_ltv_per_mille", housing.mortgage_ltv_per_mille),
        (
            "construction_margin_per_mille",
            housing.construction_margin_per_mille,
        ),
    ] {
        if !(0..=1000).contains(&value) {
            return Err(e(h, format!("{name} must be within 0..=1000")));
        }
    }
    // Leveraged bids are the point (the bank funds the stretch), but the
    // stretch must stay within what cash-first + the LTV cap can close:
    // price <= savings / (1 - ltv), i.e. bid share <= 1e6/(1000 - ltv).
    let max_bid = if housing.mortgage_ltv_per_mille >= 1000 {
        i64::MAX
    } else {
        1_000_000 / (1000 - housing.mortgage_ltv_per_mille)
    };
    if housing.home_bid_per_mille < 1 || housing.home_bid_per_mille > max_bid {
        return Err(e(
            h,
            format!(
                "home_bid_per_mille must be within 1..={max_bid}                  (beyond that no financing can close the bid)"
            ),
        ));
    }
    if housing.buyer_savings_per_mille + housing.mortgage_ltv_per_mille < 1000 {
        return Err(e(
            h,
            "savings + LTV must cover the price (>= 1000 per-mille combined)".into(),
        ));
    }

    let t = "balance/taxes.ron";
    for (name, value) in [
        ("income_per_mille", taxes.income_per_mille),
        ("sales_per_mille", taxes.sales_per_mille),
    ] {
        if !(0..=999).contains(&value) {
            return Err(e(
                t,
                format!("{name} must be within 0..=999 (a 100% tax is a typo)"),
            ));
        }
    }
    if taxes.treasury_seed_mills < 0 || taxes.public_wage_bid_mills < 1 {
        return Err(e(
            t,
            "treasury seed must be >= 0 and the wage bid >= 1".into(),
        ));
    }
    if taxes.public_positions == 0 {
        return Err(e(
            t,
            "public_positions must be >= 1 (ADR 0009 §4: the treasury IS              the public employer; zero slots turn taxes into a sink)"
                .into(),
        ));
    }
    let Some(kind) = locations
        .kinds
        .iter()
        .find(|kind| kind.id == taxes.public_location_kind_id)
    else {
        return Err(e(
            t,
            format!(
                "public_location_kind_id `{}` is not a defined location kind",
                taxes.public_location_kind_id
            ),
        ));
    };
    if kind.is_home || kind.count != 0 {
        return Err(e(
            t,
            "the public location kind must be public with count 0 (genesis creates it)".into(),
        ));
    }
    Ok(())
}

/// Phase 7 social-layer validation (ADR 0010 §7): skills/school,
/// the social graph, and fertility.
#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_social(
    data_root: &Path,
    skills: &sim_people::config::SkillsConfig,
    social: &sim_ai::config::SocialConfig,
    fertility: &sim_people::config::FertilityConfig,
    recipes: &sim_economy::config::RecipesConfig,
    people: &sim_people::config::PeopleConfig,
    locations: &sim_world::config::LocationsConfig,
    min_working_age_years: u32,
) -> Result<(), DataError> {
    let e = |file: &str, message: String| verr(data_root, file, message);

    let k = "skills.ron";
    if skills.skills.is_empty() {
        return Err(e(k, "at least one skill must be defined".into()));
    }
    for (index, skill) in skills.skills.iter().enumerate() {
        if skill.id.is_empty() {
            return Err(e(k, "skill ids must be non-empty".into()));
        }
        if skills.skills[..index].iter().any(|s| s.id == skill.id) {
            return Err(e(k, format!("duplicate skill id `{}`", skill.id)));
        }
    }
    let skill_exists = |id: &str| skills.skills.iter().any(|s| s.id == id);
    if !skill_exists(&skills.school.taught_skill_id) {
        return Err(e(
            k,
            format!(
                "school taught_skill_id `{}` is not a defined skill",
                skills.school.taught_skill_id
            ),
        ));
    }
    if !skill_exists(&skills.public_skill_id) {
        return Err(e(
            k,
            format!(
                "public_skill_id `{}` is not a defined skill",
                skills.public_skill_id
            ),
        ));
    }
    let school = &skills.school;
    if school.start_age_years >= school.end_age_years
        || school.end_age_years > min_working_age_years
    {
        return Err(e(
            k,
            "school ages must satisfy start < end <= min_working_age".into(),
        ));
    }
    if school.start_hour >= school.end_hour || school.end_hour > 24 {
        return Err(e(k, "school hours must satisfy start < end <= 24".into()));
    }
    if school.attend_ticks == 0 {
        return Err(e(k, "attend_ticks must be >= 1".into()));
    }
    if school.gain_per_attendance_per_mille == 0
        || school.gain_per_attendance_per_mille > 1000
        || skills.doing_gain_per_shift_per_mille > 1000
    {
        return Err(e(
            k,
            "skill gains must be within 1..=1000 (school) and 0..=1000 (doing)".into(),
        ));
    }
    if !(0..=1000).contains(&skills.labor_skill_weight_per_mille) {
        return Err(e(
            k,
            "labor_skill_weight_per_mille must be within 0..=1000".into(),
        ));
    }
    let Some(kind) = locations
        .kinds
        .iter()
        .find(|kind| kind.id == school.location_kind_id)
    else {
        return Err(e(
            k,
            format!(
                "school location_kind_id `{}` is not a defined location kind",
                school.location_kind_id
            ),
        ));
    };
    if kind.is_home || kind.count == 0 {
        return Err(e(
            k,
            "the school kind must be public with count >= 1 (children must \
             be able to attend from day one)"
                .into(),
        ));
    }
    for recipe in &recipes.recipes {
        if let Some(id) = &recipe.skill_id
            && !skill_exists(id)
        {
            return Err(e(
                "recipes.ron",
                format!("recipe `{}` names unknown skill `{id}`", recipe.id),
            ));
        }
    }

    let s = "balance/social.ron";
    if !people
        .needs
        .needs
        .iter()
        .any(|need| need.id == social.drift_need_id)
    {
        return Err(e(
            s,
            format!(
                "drift_need_id `{}` is not a defined need",
                social.drift_need_id
            ),
        ));
    }
    if !people
        .traits
        .traits
        .iter()
        .any(|t| t.id == social.spark_trait_id)
    {
        return Err(e(
            s,
            format!(
                "spark_trait_id `{}` is not a defined trait",
                social.spark_trait_id
            ),
        ));
    }
    if social.edge_cap == 0 || social.belief_cap == 0 {
        return Err(e(s, "edge_cap and belief_cap must be >= 1".into()));
    }
    for (name, value) in [
        (
            "friend_drift_per_meeting_per_mille",
            social.friend_drift_per_meeting_per_mille,
        ),
        (
            "romance_drift_per_meeting_per_mille",
            social.romance_drift_per_meeting_per_mille,
        ),
        ("decay_per_day_per_mille", social.decay_per_day_per_mille),
        (
            "romance_min_sociability_product_per_mille",
            social.romance_min_sociability_product_per_mille,
        ),
    ] {
        if !(0..=1000).contains(&value) {
            return Err(e(s, format!("{name} must be within 0..=1000")));
        }
    }
    if !(1..=1000).contains(&social.marriage_threshold_per_mille) {
        return Err(e(
            s,
            "marriage_threshold_per_mille must be within 1..=1000".into(),
        ));
    }
    if !(0..=1000).contains(&social.social_bond_weight_per_mille) {
        return Err(e(
            s,
            "social_bond_weight_per_mille must be within 0..=1000".into(),
        ));
    }

    let f = "balance/fertility.ron";
    let mut last_max: Option<u32> = None;
    for band in &fertility.bands {
        if band.min_age_years > band.max_age_years {
            return Err(e(f, "fertility band min must be <= max".into()));
        }
        if let Some(last) = last_max
            && band.min_age_years <= last
        {
            return Err(e(
                f,
                "fertility bands must be ascending and non-overlapping".into(),
            ));
        }
        if band.per_day_chance_per_billion > 1_000_000_000 {
            return Err(e(f, "fertility chance exceeds certainty".into()));
        }
        last_max = Some(band.max_age_years);
    }
    if fertility.max_household_size < 2 {
        return Err(e(
            f,
            "max_household_size must be >= 2 (a couple lives there)".into(),
        ));
    }
    if fertility.trait_mutation_per_mille > 1000 {
        return Err(e(f, "trait_mutation_per_mille must be <= 1000".into()));
    }
    Ok(())
}

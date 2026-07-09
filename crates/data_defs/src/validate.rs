//! Startup validation (SPEC §8: precise messages, never silent defaults;
//! split from `lib.rs` for the SPEC §3 module-size rule).

use std::path::Path;

use crate::{CalendarConfig, DataError, EngineConfig};

/// One `DataError::Validation` bound to a data file.
pub(crate) fn verr(data_root: &Path, file: &str, message: String) -> DataError {
    DataError::Validation {
        path: data_root.join(file),
        message,
    }
}

/// Validates the people configuration (SPEC §8: precise messages, no
/// silent defaults), one helper per data file plus the cross-file
/// calendar/age check.
pub(crate) fn validate_people(
    data_root: &Path,
    calendar: &CalendarConfig,
    people: &sim_people::config::PeopleConfig,
) -> Result<(), DataError> {
    validate_needs(data_root, &people.needs)?;
    validate_traits(data_root, &people.traits)?;
    validate_mortality(data_root, &people.mortality)?;
    validate_demographics(data_root, &people.demographics)?;
    validate_names(data_root, people)?;
    validate_calendar_age_fit(data_root, calendar, people)
}

fn validate_needs(
    data_root: &Path,
    needs: &sim_people::config::NeedsConfig,
) -> Result<(), DataError> {
    if needs.needs.is_empty() {
        return Err(verr(
            data_root,
            "balance/needs.ron",
            "at least one need required".into(),
        ));
    }
    for need in &needs.needs {
        let range_ok = (0..=1_000_000).contains(&need.initial_min)
            && (0..=1_000_000).contains(&need.initial_max)
            && need.initial_min <= need.initial_max;
        if !range_ok || need.decay_per_hour < 0 {
            return Err(verr(
                data_root,
                "balance/needs.ron",
                format!("need `{}` has an invalid range or negative decay", need.id),
            ));
        }
    }
    Ok(())
}

fn validate_traits(
    data_root: &Path,
    traits: &sim_people::config::TraitsConfig,
) -> Result<(), DataError> {
    for t in &traits.traits {
        if !(0..=1000).contains(&t.min) || !(0..=1000).contains(&t.max) || t.min > t.max {
            return Err(verr(
                data_root,
                "balance/traits.ron",
                format!(
                    "trait `{}` range must satisfy 0 <= min <= max <= 1000",
                    t.id
                ),
            ));
        }
    }
    Ok(())
}

fn validate_mortality(
    data_root: &Path,
    mortality: &sim_people::config::MortalityConfig,
) -> Result<(), DataError> {
    let mut previous_edge: Option<u32> = None;
    for band in &mortality.bands {
        if let Some(previous) = previous_edge
            && band.max_age_years <= previous
        {
            return Err(verr(
                data_root,
                "balance/mortality.ron",
                format!(
                    "band edges must strictly ascend ({} after {previous})",
                    band.max_age_years
                ),
            ));
        }
        previous_edge = Some(band.max_age_years);
    }
    // Chances are per-billion probabilities: > 1e9 would nominally mean a
    // probability above 1 — almost certainly a data typo, rejected rather
    // than silently meaning "certain death".
    let all_chances = mortality
        .bands
        .iter()
        .map(|b| b.per_day_chance_per_billion)
        .chain([mortality.terminal_per_day_chance_per_billion]);
    for chance in all_chances {
        if chance > 1_000_000_000 {
            return Err(verr(
                data_root,
                "balance/mortality.ron",
                format!("per-billion chance {chance} exceeds 1_000_000_000 (probability > 1)"),
            ));
        }
    }
    Ok(())
}

fn validate_demographics(
    data_root: &Path,
    demographics: &sim_people::config::DemographicsConfig,
) -> Result<(), DataError> {
    let file = "balance/demographics.ron";
    let weight_sum: u32 = demographics
        .age_bands
        .iter()
        .map(|b| b.weight_per_mille)
        .sum();
    if weight_sum != 1000 {
        return Err(verr(
            data_root,
            file,
            format!("age band weights must sum to 1000, got {weight_sum}"),
        ));
    }
    for band in &demographics.age_bands {
        if band.min_age_years > band.max_age_years {
            return Err(verr(
                data_root,
                file,
                format!(
                    "age band {}..{} is inverted",
                    band.min_age_years, band.max_age_years
                ),
            ));
        }
    }
    if demographics.male_per_mille > 1000 {
        return Err(verr(
            data_root,
            file,
            "male_per_mille must be <= 1000".into(),
        ));
    }
    if demographics.household_min == 0 || demographics.household_min > demographics.household_max {
        return Err(verr(
            data_root,
            file,
            "household size range must satisfy 1 <= min <= max".into(),
        ));
    }
    if demographics.annual_death_rate_min_per_mille >= demographics.annual_death_rate_max_per_mille
    {
        return Err(verr(
            data_root,
            file,
            "death-rate acceptance band must satisfy min < max".into(),
        ));
    }
    Ok(())
}

fn validate_names(
    data_root: &Path,
    people: &sim_people::config::PeopleConfig,
) -> Result<(), DataError> {
    for (file, list) in [
        ("names/given_female.ron", &people.given_female),
        ("names/given_male.ron", &people.given_male),
        ("names/family.ron", &people.family),
    ] {
        if list.names.is_empty() {
            return Err(verr(data_root, file, "name list must not be empty".into()));
        }
    }
    Ok(())
}

/// Cross-file check (ADR 0005 §6): the largest data-defined age, converted
/// to ticks under the loaded calendar (plus one year of birthday offset
/// and one year of margin), must fit in i64 — this is what makes the age
/// arithmetic in `sim_people` total (no overflow panic in debug, no
/// wrapped birth_tick in release).
fn validate_calendar_age_fit(
    data_root: &Path,
    calendar: &CalendarConfig,
    people: &sim_people::config::PeopleConfig,
) -> Result<(), DataError> {
    let ticks_per_year = core_types::calendar::SEASONS_PER_YEAR
        * u64::from(calendar.days_per_season)
        * core_types::calendar::TICKS_PER_DAY;
    let max_band_age = people
        .mortality
        .bands
        .iter()
        .map(|b| b.max_age_years)
        .chain(
            people
                .demographics
                .age_bands
                .iter()
                .map(|b| b.max_age_years),
        )
        .max()
        .unwrap_or(0);
    let fits = u64::from(max_band_age)
        .checked_add(2) // +1 birthday offset, +1 margin
        .and_then(|years| years.checked_mul(ticks_per_year))
        .is_some_and(|ticks| i64::try_from(ticks).is_ok());
    if !fits {
        return Err(verr(
            data_root,
            "balance/demographics.ron",
            format!(
                "max data-defined age {max_band_age} years × {ticks_per_year} ticks/year \
                 overflows tick arithmetic; shrink the age bands or days_per_season"
            ),
        ));
    }
    Ok(())
}

/// Validates `data/locations.ron` (ADR 0006 §7): exactly one home kind,
/// positive rates, need ids that exist, unique kind ids, and non-home
/// kinds that actually get instantiated.
pub(crate) fn validate_locations(
    data_root: &Path,
    locations: &sim_world::config::LocationsConfig,
    people: &sim_people::config::PeopleConfig,
) -> Result<(), DataError> {
    let file = "locations.ron";
    let home_count = locations.kinds.iter().filter(|k| k.is_home).count();
    if home_count != 1 {
        return Err(verr(
            data_root,
            file,
            format!("exactly one home kind required, found {home_count}"),
        ));
    }
    let mut seen_ids: Vec<&str> = Vec::new();
    for kind in &locations.kinds {
        if seen_ids.contains(&kind.id.as_str()) {
            return Err(verr(
                data_root,
                file,
                format!("duplicate location kind id `{}`", kind.id),
            ));
        }
        seen_ids.push(&kind.id);
        // A public kind with count 0, or a kind that satisfies nothing,
        // is dead data UNLESS a retail firm kind claims it (its instances
        // come from firm genesis and citizens reach it through the offer)
        // — that cross-file check lives in `validate_econ` (ADR 0007 §7).
        for satisfier in &kind.satisfies {
            if satisfier.per_tick <= 0 {
                return Err(verr(
                    data_root,
                    file,
                    format!(
                        "kind `{}` satisfier `{}` must have per_tick >= 1",
                        kind.id, satisfier.need_id
                    ),
                ));
            }
            if !people.needs.needs.iter().any(|n| n.id == satisfier.need_id) {
                return Err(verr(
                    data_root,
                    file,
                    format!(
                        "kind `{}` references unknown need `{}`",
                        kind.id, satisfier.need_id
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Validates `data/balance/ai.ron` (ADR 0006 §7), including its
/// cross-references into needs, traits, and location kinds.
pub(crate) fn validate_ai(
    data_root: &Path,
    ai: &sim_ai::config::AiConfig,
    people: &sim_people::config::PeopleConfig,
    locations: &sim_world::config::LocationsConfig,
) -> Result<(), DataError> {
    let file = "balance/ai.ron";
    let e = |message: String| verr(data_root, file, message);

    if ai.urgency_exponent < 1 {
        return Err(e("urgency_exponent must be >= 1".into()));
    }
    for (name, value) in [
        ("travel_ticks", ai.travel_ticks),
        ("max_perform_ticks", ai.max_perform_ticks),
        ("idle_ticks", ai.idle_ticks),
    ] {
        if value == 0 {
            return Err(e(format!("{name} must be >= 1")));
        }
    }
    if ai.time_cost_micro_per_tick < 0 {
        return Err(e("time_cost_micro_per_tick must be >= 0".into()));
    }
    for (name, hour) in [
        ("plan_compile_hour", ai.plan_compile_hour),
        ("sleep.base_start_hour", ai.sleep.base_start_hour),
        ("sleep.base_end_hour", ai.sleep.base_end_hour),
    ] {
        if hour >= 24 {
            return Err(e(format!("{name} must be < 24, got {hour}")));
        }
    }
    if ai.sleep.home_bias_micro < 0 {
        return Err(e("sleep.home_bias_micro must be >= 0".into()));
    }
    if ai.purchase.mu_scale_micro < 0 {
        return Err(e("purchase.mu_scale_micro must be >= 0".into()));
    }
    if ai.purchase.half_wealth_mills < 1 {
        return Err(e("purchase.half_wealth_mills must be >= 1".into()));
    }

    let need_exists = |id: &str| people.needs.needs.iter().any(|n| n.id == id);
    let trait_exists = |id: &str| people.traits.traits.iter().any(|t| t.id == id);
    if !need_exists(&ai.sleep.rest_need_id) {
        return Err(e(format!(
            "sleep.rest_need_id `{}` is not a defined need",
            ai.sleep.rest_need_id
        )));
    }
    if !trait_exists(&ai.sleep.shift_trait_id) {
        return Err(e(format!(
            "sleep.shift_trait_id `{}` is not a defined trait",
            ai.sleep.shift_trait_id
        )));
    }
    // A town whose citizens cannot sleep is a data error (ADR 0006 §7):
    // some kind must satisfy the rest need.
    let rest_satisfiable = locations.kinds.iter().any(|kind| {
        kind.satisfies
            .iter()
            .any(|s| s.need_id == ai.sleep.rest_need_id)
    });
    if !rest_satisfiable {
        return Err(e(format!(
            "no location kind satisfies the rest need `{}`",
            ai.sleep.rest_need_id
        )));
    }
    for weight in &ai.need_trait_weights {
        if !need_exists(&weight.need_id) {
            return Err(e(format!(
                "need_trait_weights references unknown need `{}`",
                weight.need_id
            )));
        }
        if !trait_exists(&weight.trait_id) {
            return Err(e(format!(
                "need_trait_weights references unknown trait `{}`",
                weight.trait_id
            )));
        }
        if weight.weight_per_mille > 1000 {
            return Err(e(format!(
                "need_trait_weights weight for `{}` must be <= 1000",
                weight.need_id
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate(
    data_root: &Path,
    calendar: &CalendarConfig,
    engine: &EngineConfig,
) -> Result<(), DataError> {
    if calendar.days_per_season == 0 {
        return Err(DataError::Validation {
            path: data_root.join("balance/calendar.ron"),
            message: format!(
                "days_per_season must be >= 1, got {}",
                calendar.days_per_season
            ),
        });
    }
    if engine.event_log_capacity == 0 {
        return Err(DataError::Validation {
            path: data_root.join("balance/engine.ron"),
            message: format!(
                "event_log_capacity must be >= 1, got {}",
                engine.event_log_capacity
            ),
        });
    }
    Ok(())
}

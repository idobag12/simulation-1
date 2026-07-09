//! RON schema types, loader, and startup validator (SPEC §8; ADR 0004 §8).
//!
//! Invariants owned by this crate:
//! - Every tunable number in the simulation enters through here from a file
//!   under `data/` — nothing tunable lives in code (SPEC §8, §16.2).
//! - Loading is strict: a missing file, an unknown field, a syntax error,
//!   or an out-of-range value is a precise typed error naming the file —
//!   never a silent default.
//! - Loaded configuration is immutable for the lifetime of the world.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

/// Errors loading or validating data definitions.
#[derive(Debug, Error)]
pub enum DataError {
    /// A required data file could not be read.
    #[error("cannot read data file `{path}`: {source}")]
    Io {
        /// The file that could not be read.
        path: PathBuf,
        /// The underlying filesystem error.
        source: std::io::Error,
    },
    /// A data file is not valid RON for its schema (includes unknown
    /// fields, which are rejected — no silent typo-tolerance).
    #[error("cannot parse `{path}`: {message}")]
    Parse {
        /// The offending file.
        path: PathBuf,
        /// Parser diagnostic (position + cause).
        message: String,
    },
    /// A value parsed but is outside its valid range.
    #[error("invalid value in `{path}`: {message}")]
    Validation {
        /// The offending file.
        path: PathBuf,
        /// What is out of range and what the constraint is.
        message: String,
    },
}

/// Calendar tunables (`data/balance/calendar.ron`). The calendar's
/// *structure* (1440 ticks/day, 4 seasons/year) is definitional and fixed
/// in code (SPEC §5, ADR 0004 §6); only genuinely tunable values live here.
///
/// Invariant after validation: `days_per_season >= 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarConfig {
    /// Days in each of the four seasons.
    pub days_per_season: u32,
}

/// Engine tunables (`data/balance/engine.ron`).
///
/// Invariant after validation: `event_log_capacity >= 1`. The log capacity
/// bounds the observability ring only; it cannot affect simulation
/// trajectories (ADR 0004 §5, enforced by test).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineConfig {
    /// Maximum retained entries in the event log ring.
    pub event_log_capacity: u32,
}

/// All loaded data definitions. Grows as phases add content (goods,
/// recipes, professions… — each in the phase that consumes it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDefs {
    /// Calendar tunables.
    pub calendar: CalendarConfig,
    /// Engine tunables.
    pub engine: EngineConfig,
    /// People-simulation tunables and name lists (Phase 2, ADR 0005).
    pub people: sim_people::config::PeopleConfig,
}

/// Loads and validates every data definition from a `data/` directory
/// root. This is the single entry point applications use at startup; a
/// failure here is a startup error (SPEC §8).
pub fn load(data_root: &Path) -> Result<DataDefs, DataError> {
    let calendar: CalendarConfig = load_ron(&data_root.join("balance/calendar.ron"))?;
    let engine: EngineConfig = load_ron(&data_root.join("balance/engine.ron"))?;
    let people = sim_people::config::PeopleConfig {
        needs: load_ron(&data_root.join("balance/needs.ron"))?,
        traits: load_ron(&data_root.join("balance/traits.ron"))?,
        mortality: load_ron(&data_root.join("balance/mortality.ron"))?,
        demographics: load_ron(&data_root.join("balance/demographics.ron"))?,
        given_female: load_ron(&data_root.join("names/given_female.ron"))?,
        given_male: load_ron(&data_root.join("names/given_male.ron"))?,
        family: load_ron(&data_root.join("names/family.ron"))?,
    };
    validate(data_root, &calendar, &engine)?;
    validate_people(data_root, &calendar, &people)?;
    Ok(DataDefs {
        calendar,
        engine,
        people,
    })
}

fn load_ron<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, DataError> {
    let text = std::fs::read_to_string(path).map_err(|source| DataError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    ron::from_str(&text).map_err(|e| DataError::Parse {
        path: path.to_path_buf(),
        message: e.to_string(),
    })
}

/// One `DataError::Validation` bound to a data file.
fn verr(data_root: &Path, file: &str, message: String) -> DataError {
    DataError::Validation {
        path: data_root.join(file),
        message,
    }
}

/// Validates the people configuration (SPEC §8: precise messages, no
/// silent defaults), one helper per data file plus the cross-file
/// calendar/age check.
fn validate_people(
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

fn validate(
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

#[cfg(test)]
mod tests {
    use super::*;

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
    )"#;
    const GOOD_NAMES: &str = r#"NameList(names: ["A", "B"])"#;

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
}

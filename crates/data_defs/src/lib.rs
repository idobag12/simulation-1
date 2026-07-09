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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataDefs {
    /// Calendar tunables.
    pub calendar: CalendarConfig,
    /// Engine tunables.
    pub engine: EngineConfig,
}

/// Loads and validates every data definition from a `data/` directory
/// root. This is the single entry point applications use at startup; a
/// failure here is a startup error (SPEC §8).
pub fn load(data_root: &Path) -> Result<DataDefs, DataError> {
    let calendar: CalendarConfig = load_ron(&data_root.join("balance/calendar.ron"))?;
    let engine: EngineConfig = load_ron(&data_root.join("balance/engine.ron"))?;
    validate(data_root, &calendar, &engine)?;
    Ok(DataDefs { calendar, engine })
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

    fn write_tree(files: &[(&str, &str)]) -> PathBuf {
        // Unique per-test directory (thread id) under the target tmpdir.
        let root = std::env::temp_dir()
            .join("embervale-data-defs-tests")
            .join(format!("{:?}", std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&root);
        for (rel, content) in files {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().expect("has parent")).expect("mkdir");
            std::fs::write(path, content).expect("write");
        }
        root
    }

    const GOOD_CALENDAR: &str = "CalendarConfig(days_per_season: 30)";
    const GOOD_ENGINE: &str = "EngineConfig(event_log_capacity: 4096)";

    #[test]
    fn valid_data_loads() {
        let root = write_tree(&[
            ("balance/calendar.ron", GOOD_CALENDAR),
            ("balance/engine.ron", GOOD_ENGINE),
        ]);
        let defs = load(&root).expect("valid data must load");
        assert_eq!(defs.calendar.days_per_season, 30);
        assert_eq!(defs.engine.event_log_capacity, 4096);
    }

    #[test]
    fn missing_file_names_the_path() {
        let root = write_tree(&[("balance/calendar.ron", GOOD_CALENDAR)]);
        match load(&root) {
            Err(DataError::Io { path, .. }) => {
                assert!(path.ends_with("balance/engine.ron"), "{path:?}");
            }
            other => panic!("expected Io error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_field_is_a_parse_error_not_a_silent_default() {
        let root = write_tree(&[
            (
                "balance/calendar.ron",
                "CalendarConfig(days_per_season: 30, dayz_per_saeson: 12)",
            ),
            ("balance/engine.ron", GOOD_ENGINE),
        ]);
        assert!(matches!(load(&root), Err(DataError::Parse { .. })));
    }

    #[test]
    fn syntax_error_is_a_parse_error() {
        let root = write_tree(&[
            ("balance/calendar.ron", "CalendarConfig(days_per_season:"),
            ("balance/engine.ron", GOOD_ENGINE),
        ]);
        assert!(matches!(load(&root), Err(DataError::Parse { .. })));
    }

    #[test]
    fn out_of_range_values_are_validation_errors_with_precise_messages() {
        let root = write_tree(&[
            ("balance/calendar.ron", "CalendarConfig(days_per_season: 0)"),
            ("balance/engine.ron", GOOD_ENGINE),
        ]);
        match load(&root) {
            Err(DataError::Validation { message, .. }) => {
                assert!(message.contains("days_per_season"), "{message}");
            }
            other => panic!("expected Validation error, got {other:?}"),
        }

        let root = write_tree(&[
            ("balance/calendar.ron", GOOD_CALENDAR),
            ("balance/engine.ron", "EngineConfig(event_log_capacity: 0)"),
        ]);
        assert!(matches!(load(&root), Err(DataError::Validation { .. })));
    }
}

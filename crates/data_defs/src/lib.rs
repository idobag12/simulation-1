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
    /// Location kinds (Phase 3, ADR 0006 §2).
    pub locations: sim_world::config::LocationsConfig,
    /// Utility-AI tunables (Phase 3, ADR 0006 §7).
    pub ai: sim_ai::config::AiConfig,
    /// Good definitions (Phase 4, ADR 0007 §7).
    pub goods: sim_goods::config::GoodsConfig,
    /// Recipes (Phase 4, ADR 0007 §7).
    pub recipes: sim_economy::config::RecipesConfig,
    /// Firm kinds (Phase 4, ADR 0007 §7).
    pub firms: sim_economy::config::FirmsConfig,
    /// Posted-price market tunables (Phase 4, ADR 0007 §7).
    pub economy: sim_economy::config::EconomyConfig,
    /// Labor-market tunables (Phase 5, ADR 0008 §7).
    pub labor: sim_economy::config::LaborConfig,
    /// Bank tunables (Phase 6, ADR 0009 §7).
    pub bank: sim_economy::BankConfig,
    /// Housing tunables (Phase 6, ADR 0009 §7).
    pub housing: sim_economy::HousingConfig,
    /// Taxes and the public employer (Phase 6, ADR 0009 §7).
    pub taxes: sim_economy::TaxesConfig,
    /// Skills and the education pipeline (Phase 7, ADR 0010 §1).
    pub skills: sim_people::config::SkillsConfig,
    /// The social graph, gossip, and marriage (Phase 7, ADR 0010 §§2–4).
    pub social: sim_ai::config::SocialConfig,
    /// Reproduction (Phase 7, ADR 0010 §4).
    pub fertility: sim_people::config::FertilityConfig,
    /// LOD tiers and catch-up (Phase 8, ADR 0011 §7).
    pub lod: sim_ai::config::LodConfig,
    /// The town map: districts and travel times (Phase 9, ADR 0012 §1).
    pub map: sim_world::config::MapConfig,
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
    let locations: sim_world::config::LocationsConfig = load_ron(&data_root.join("locations.ron"))?;
    let ai: sim_ai::config::AiConfig = load_ron(&data_root.join("balance/ai.ron"))?;
    let goods: sim_goods::config::GoodsConfig = load_ron(&data_root.join("goods.ron"))?;
    let recipes: sim_economy::config::RecipesConfig = load_ron(&data_root.join("recipes.ron"))?;
    let firms: sim_economy::config::FirmsConfig = load_ron(&data_root.join("firms.ron"))?;
    let economy: sim_economy::config::EconomyConfig =
        load_ron(&data_root.join("balance/economy.ron"))?;
    let labor: sim_economy::config::LaborConfig = load_ron(&data_root.join("balance/labor.ron"))?;
    let bank: sim_economy::BankConfig = load_ron(&data_root.join("balance/bank.ron"))?;
    let housing: sim_economy::HousingConfig = load_ron(&data_root.join("balance/housing.ron"))?;
    let taxes: sim_economy::TaxesConfig = load_ron(&data_root.join("balance/taxes.ron"))?;
    let skills: sim_people::config::SkillsConfig = load_ron(&data_root.join("skills.ron"))?;
    let social: sim_ai::config::SocialConfig = load_ron(&data_root.join("balance/social.ron"))?;
    let fertility: sim_people::config::FertilityConfig =
        load_ron(&data_root.join("balance/fertility.ron"))?;
    let lod: sim_ai::config::LodConfig = load_ron(&data_root.join("balance/lod.ron"))?;
    let map: sim_world::config::MapConfig = load_ron(&data_root.join("map.ron"))?;
    validate(data_root, &calendar, &engine)?;
    validate_people(data_root, &calendar, &people)?;
    validate_locations(data_root, &locations, &people)?;
    validate_ai(data_root, &ai, &people, &locations)?;
    validate_economy(
        data_root,
        &goods,
        &recipes,
        &firms,
        &economy,
        &people,
        &locations,
        &taxes.public_location_kind_id,
        &skills.school.location_kind_id,
    )?;
    validate_labor(data_root, &labor, &people)?;
    validate_money(data_root, &bank, &housing, &taxes, &locations)?;
    validate_social(
        data_root,
        &skills,
        &social,
        &fertility,
        &recipes,
        &people,
        &locations,
        labor.min_working_age_years,
    )?;
    validate_lod(data_root, &lod)?;
    validate_map(data_root, &map)?;
    Ok(DataDefs {
        calendar,
        engine,
        people,
        locations,
        ai,
        goods,
        recipes,
        firms,
        economy,
        labor,
        bank,
        housing,
        taxes,
        skills,
        social,
        fertility,
        lod,
        map,
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

mod resolve;
mod validate;
mod validate_econ;
mod validate_map;
mod validate_money;
pub use resolve::{resolve_ai, resolve_economy};
use validate::{validate, validate_ai, validate_locations, validate_people};
use validate_econ::validate_economy;
use validate_map::validate_map;
use validate_money::{validate_labor, validate_lod, validate_money, validate_social};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
#[cfg(test)]
#[path = "tests_econ.rs"]
mod tests_econ;

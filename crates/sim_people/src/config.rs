//! Data-defined tunables for the people simulation (SPEC §8). These
//! structs ARE the RON schemas for `data/balance/{needs,traits,mortality,
//! demographics}.ron` and `data/names/*.ron`; `data_defs` loads and
//! validates them (ADR 0005 §6).
//!
//! Invariant: every field here is a tunable or authored content; nothing
//! in this module carries code-decided values.

use serde::Deserialize;

/// One need definition, in the fixed order that becomes the per-citizen
/// need vector's order (ADR 0005 §3).
///
/// Invariant after validation: `initial_min <= initial_max`, both within
/// 0..=1_000_000; `decay_per_hour >= 0`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeedDef {
    /// Stable identifier, e.g. `"hunger"`.
    pub id: String,
    /// Per-million units lost each hour (needs decay is hour-rate, SPEC §5).
    pub decay_per_hour: i64,
    /// Genesis range: initial level lower bound (per-million).
    pub initial_min: i64,
    /// Genesis range: initial level upper bound (per-million).
    pub initial_max: i64,
}

/// `data/balance/needs.ron`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeedsConfig {
    /// Ordered need definitions.
    pub needs: Vec<NeedDef>,
}

/// One personality-trait definition, in the fixed order that becomes the
/// per-citizen trait vector's order.
///
/// Invariant after validation: `0 <= min <= max <= 1000` (per-mille).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraitDef {
    /// Stable identifier, e.g. `"industriousness"`.
    pub id: String,
    /// Genesis sample lower bound (per-mille).
    pub min: i16,
    /// Genesis sample upper bound (per-mille).
    pub max: i16,
}

/// `data/balance/traits.ron`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraitsConfig {
    /// Ordered trait definitions.
    pub traits: Vec<TraitDef>,
}

/// One mortality band: citizens aged `<= max_age_years` (and older than
/// the previous band) die with `per_day_chance_per_billion` each day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MortalityBand {
    /// Inclusive upper age bound (world years) for this band.
    pub max_age_years: u32,
    /// Daily death probability in per-billion units (exact integer
    /// comparison against a uniform draw, ADR 0005 §4).
    pub per_day_chance_per_billion: u32,
}

/// `data/balance/mortality.ron` — the deterministic mortality curve.
///
/// Invariant after validation: bands are in strictly ascending
/// `max_age_years` order and `terminal_per_day_chance_per_billion` covers
/// every age beyond the last band (full coverage, SPEC §8: never a silent
/// default).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MortalityConfig {
    /// Ascending age bands.
    pub bands: Vec<MortalityBand>,
    /// Daily death chance for ages beyond the last band (per-billion).
    pub terminal_per_day_chance_per_billion: u32,
}

impl MortalityConfig {
    /// The daily death chance (per-billion) for a citizen aged
    /// `age_years`. Total: every age maps to a band or the terminal rate.
    pub fn per_day_chance(&self, age_years: u32) -> u32 {
        for band in &self.bands {
            if age_years <= band.max_age_years {
                return band.per_day_chance_per_billion;
            }
        }
        self.terminal_per_day_chance_per_billion
    }
}

/// One genesis age band with a sampling weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgeBand {
    /// Inclusive lower age bound (world years).
    pub min_age_years: u32,
    /// Inclusive upper age bound (world years).
    pub max_age_years: u32,
    /// Relative sampling weight (per-mille of the population; weights must
    /// sum to 1000).
    pub weight_per_mille: u32,
}

/// `data/balance/demographics.ron` — genesis distributions AND the
/// acceptance bands the Phase 2 exit criterion asserts (the thresholds
/// are data, not code constants; ADR 0005 §6).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DemographicsConfig {
    /// Genesis age pyramid (weights sum to 1000).
    pub age_bands: Vec<AgeBand>,
    /// Probability a citizen is male, per-mille.
    pub male_per_mille: u32,
    /// Smallest household generated.
    pub household_min: u32,
    /// Largest household generated.
    pub household_max: u32,
    /// Regression band: minimum acceptable annualized crude death rate
    /// (per-mille of initial population).
    pub annual_death_rate_min_per_mille: u32,
    /// Regression band: maximum acceptable annualized crude death rate
    /// (per-mille of initial population).
    pub annual_death_rate_max_per_mille: u32,
}

/// `data/names/*.ron` — authored name lists.
///
/// Invariant after validation: every list is non-empty.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NameList {
    /// The names.
    pub names: Vec<String>,
}

/// All people-simulation configuration, assembled by `data_defs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeopleConfig {
    /// Need definitions (order = need vector order).
    pub needs: NeedsConfig,
    /// Trait definitions (order = trait vector order).
    pub traits: TraitsConfig,
    /// Mortality curve.
    pub mortality: MortalityConfig,
    /// Genesis distributions + acceptance bands.
    pub demographics: DemographicsConfig,
    /// Female given names.
    pub given_female: NameList,
    /// Male given names.
    pub given_male: NameList,
    /// Family names.
    pub family: NameList,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mortality_lookup_covers_all_ages() {
        let config = MortalityConfig {
            bands: vec![
                MortalityBand {
                    max_age_years: 40,
                    per_day_chance_per_billion: 10,
                },
                MortalityBand {
                    max_age_years: 70,
                    per_day_chance_per_billion: 100,
                },
            ],
            terminal_per_day_chance_per_billion: 1000,
        };
        assert_eq!(config.per_day_chance(0), 10);
        assert_eq!(config.per_day_chance(40), 10);
        assert_eq!(config.per_day_chance(41), 100);
        assert_eq!(config.per_day_chance(70), 100);
        assert_eq!(config.per_day_chance(71), 1000);
        assert_eq!(config.per_day_chance(200), 1000);
    }
}

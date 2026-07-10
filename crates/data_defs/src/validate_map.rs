//! Phase 9 map validation (ADR 0012 §6). Split from
//! `validate_money.rs` for the SPEC §3 module-size rule.

use crate::DataError;
/// `map.ron` bounds (Phase 9, ADR 0012 §6): at least one district; the
/// travel matrix square, symmetric, every entry ≥ 1 (the diagonal is
/// the intra-district trip) and at most one day; the commute rate
/// within `0..=1_000_000` (overflow ceiling, the bank-rate rule).
pub(crate) fn validate_map(
    data_root: &std::path::Path,
    map: &sim_world::config::MapConfig,
) -> Result<(), DataError> {
    let f = "map.ron";
    let e = |message: String| DataError::Validation {
        path: data_root.join(f),
        message,
    };
    if map.districts.is_empty() {
        return Err(e("at least one district".into()));
    }
    let n = map.districts.len();
    for (index, district) in map.districts.iter().enumerate() {
        if map.districts[..index]
            .iter()
            .any(|other| other.id == district.id)
        {
            return Err(e(format!("duplicate district id `{}`", district.id)));
        }
    }
    if map.travel_ticks.len() != n || map.travel_ticks.iter().any(|row| row.len() != n) {
        return Err(e(format!(
            "travel_ticks must be {n}x{n} (districts x districts)"
        )));
    }
    for from in 0..n {
        for to in 0..n {
            let ticks = map.travel_ticks[from][to];
            // The diagonal is the intra-district trip — no teleports.
            if ticks == 0 {
                return Err(e("travel_ticks entries must be >= 1 (the diagonal \
                     is the intra-district trip; no teleports)"
                    .into()));
            }
            if u64::from(ticks) > core_types::calendar::TICKS_PER_DAY {
                return Err(e("travel_ticks entries must fit inside a day".into()));
            }
            if ticks != map.travel_ticks[to][from] {
                return Err(e("travel_ticks must be symmetric".into()));
            }
        }
    }
    // A ceiling keeps every commute multiply within i64 by construction
    // (the bank-rate rule): an absurd-but-parsable rate must die at
    // load, not wrap a clearing.
    if !(0..=1_000_000).contains(&map.commute_mills_per_tick) {
        return Err(e(
            "commute_mills_per_tick must be within 0..=1_000_000".into()
        ));
    }
    Ok(())
}

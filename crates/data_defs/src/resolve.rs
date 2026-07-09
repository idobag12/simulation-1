//! Cross-config resolution into `sim_ai::AiTables` (split from `lib.rs`
//! for the SPEC §3 module-size rule).

use crate::DataDefs;

/// Resolves the loaded, validated configs into the index-based tables the
/// AI systems run on (`sim_ai::AiTables`; ADR 0006 §1 — `sim_ai` never
/// sees other sim crates' config types).
///
/// Invariant: only call with a `DataDefs` produced by [`load`] (or an
/// equally validated value); resolution relies on validation having
/// checked every cross-reference.
pub fn resolve_ai(defs: &DataDefs) -> sim_ai::AiTables {
    let need_index = |id: &str| -> u32 {
        defs.people
            .needs
            .needs
            .iter()
            .position(|n| n.id == id)
            .unwrap_or(0) as u32 // validation guarantees a hit; 0 is unreachable
    };
    let trait_index = |id: &str| -> u32 {
        defs.people
            .traits
            .traits
            .iter()
            .position(|t| t.id == id)
            .unwrap_or(0) as u32 // validation guarantees a hit
    };

    let mut need_trait: Vec<Option<(u32, u32)>> = vec![None; defs.people.needs.needs.len()];
    for weight in &defs.ai.need_trait_weights {
        let slot = need_index(&weight.need_id) as usize;
        if let Some(entry) = need_trait.get_mut(slot) {
            *entry = Some((trait_index(&weight.trait_id), weight.weight_per_mille));
        }
    }

    sim_ai::AiTables {
        travel_ticks: defs.ai.travel_ticks,
        urgency_exponent: defs.ai.urgency_exponent,
        time_cost_micro_per_tick: defs.ai.time_cost_micro_per_tick,
        max_perform_ticks: defs.ai.max_perform_ticks,
        idle_ticks: defs.ai.idle_ticks,
        plan_compile_hour: defs.ai.plan_compile_hour,
        sleep_start_minute: u16::from(defs.ai.sleep.base_start_hour) * 60,
        sleep_end_minute: u16::from(defs.ai.sleep.base_end_hour) * 60,
        sleep_max_shift_minutes: defs.ai.sleep.max_shift_minutes,
        sleep_shift_trait: trait_index(&defs.ai.sleep.shift_trait_id),
        rest_need: need_index(&defs.ai.sleep.rest_need_id),
        sleep_home_bias_micro: defs.ai.sleep.home_bias_micro,
        kind_is_home: defs.locations.kinds.iter().map(|k| k.is_home).collect(),
        kind_satisfiers: defs
            .locations
            .kinds
            .iter()
            .map(|kind| {
                kind.satisfies
                    .iter()
                    .map(|s| (need_index(&s.need_id), s.per_tick))
                    .collect()
            })
            .collect(),
        need_trait,
    }
}

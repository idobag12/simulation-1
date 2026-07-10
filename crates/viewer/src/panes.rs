//! Pane data builders (Phase 10, ADR 0013 §§1, 4): everything each
//! pane KNOWS, as plain data — the egui layer only draws these. The
//! exit criterion is proven against THIS module.

use crate::snapshot::{CitizenRow, EventRow, LocationRow, Snapshot};

/// Name/id search (the inspector's entry point): case-insensitive
/// substring on names, or an exact entity index; entity order.
pub fn search(snapshot: &Snapshot, query: &str) -> Vec<u32> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    if let Ok(index) = query.parse::<u32>() {
        return snapshot
            .citizens
            .iter()
            .filter(|row| row.index == index)
            .map(|row| row.index)
            .collect();
    }
    let needle = query.to_lowercase();
    snapshot
        .citizens
        .iter()
        .filter(|row| row.name.to_lowercase().contains(&needle))
        .map(|row| row.index)
        .collect()
}

/// The inspector's assembled view of one citizen: the snapshot row,
/// their recent events (newest last, capped at 50 per SPEC §13), and
/// their story lines.
#[derive(Debug, Clone)]
pub struct InspectorView {
    /// The citizen's snapshot spine.
    pub row: CitizenRow,
    /// Up to the last 50 events with this citizen as a subject.
    pub events: Vec<EventRow>,
    /// The narrative lines citing this citizen.
    pub stories: Vec<String>,
}

/// Assembles the inspector for `index` (None if the citizen is not in
/// this snapshot — despawned or never existed).
pub fn inspect(snapshot: &Snapshot, index: u32) -> Option<InspectorView> {
    let row = snapshot
        .citizens
        .iter()
        .find(|row| row.index == index)?
        .clone();
    let mut events: Vec<EventRow> =
        crate::events::filter(&snapshot.events, Some(index), None, None)
            .into_iter()
            .cloned()
            .collect();
    if events.len() > 50 {
        events.drain(..events.len() - 50);
    }
    let stories = snapshot
        .stories
        .iter()
        .filter(|(subjects, _)| subjects.contains(&index))
        .map(|(_, line)| line.clone())
        .collect();
    Some(InspectorView {
        row,
        events,
        stories,
    })
}

/// Re-resolves a followed citizen against a NEW snapshot: `Some` while
/// they still exist, `None` once they despawn (died) — the paint layer
/// shows a placeholder instead of silently blanking.
pub fn follow_focus(snapshot: &Snapshot, focused: Option<u32>) -> Option<u32> {
    focused.filter(|index| snapshot.citizens.iter().any(|row| row.index == *index))
}

/// The inspector's assembled view of one LOCATION (firms, homes, and
/// venues are inspectable too — SPEC §13's "inspector for any entity").
#[derive(Debug, Clone)]
pub struct LocationView {
    /// The location's snapshot row.
    pub row: LocationRow,
    /// Citizens employed here, entity order.
    pub employees: Vec<u32>,
    /// Citizens standing here right now, entity order.
    pub present: Vec<u32>,
    /// Up to the last 50 events citing this location.
    pub events: Vec<EventRow>,
}

/// Assembles the inspector for location `index` (None if no such
/// location exists in this snapshot).
pub fn inspect_location(snapshot: &Snapshot, index: u32) -> Option<LocationView> {
    let row = snapshot
        .locations
        .iter()
        .find(|row| row.index == index)?
        .clone();
    let employees = snapshot
        .citizens
        .iter()
        .filter(|citizen| citizen.employer == Some(index))
        .map(|citizen| citizen.index)
        .collect();
    let mut events: Vec<EventRow> =
        crate::events::filter(&snapshot.events, Some(index), None, None)
            .into_iter()
            .cloned()
            .collect();
    if events.len() > 50 {
        events.drain(..events.len() - 50);
    }
    Some(LocationView {
        row,
        employees,
        present: present_at(snapshot, index),
        events,
    })
}

/// The overlay modes (SPEC §13's list; one color rule each).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// Wallet + deposits, quantile bands.
    Wealth,
    /// A chosen need's level (index in data order).
    Need(u32),
    /// LOD tier coloring.
    Tier,
    /// Posted retail prices, quantile bands — colors LOCATIONS (via
    /// [`location_overlay_class`]); citizens render neutral.
    Price,
}

/// A citizen's overlay class in `0..=4` (the paint layer maps classes
/// to a palette; the CLASSIFICATION is the logic worth testing).
pub fn overlay_class(snapshot: &Snapshot, overlay: Overlay, citizen: &CitizenRow) -> u8 {
    match overlay {
        Overlay::Wealth => {
            let mut wealths: Vec<i64> = snapshot
                .citizens
                .iter()
                .map(|row| row.cash_mills + row.deposit_mills)
                .collect();
            wealths.sort_unstable();
            quantile_class(&wealths, citizen.cash_mills + citizen.deposit_mills)
        }
        Overlay::Need(index) => {
            let level = citizen.needs.get(index as usize).copied().unwrap_or(0);
            // Five equal bands over the clamped need scale
            // (0..=1_000_000) — the top band IS reachable at full.
            (level / 200_000).min(4) as u8
        }
        Overlay::Tier => match citizen.tier {
            core_ecs::sim_interface::Tier::A => 0,
            core_ecs::sim_interface::Tier::B => 2,
            core_ecs::sim_interface::Tier::C => 4,
        },
        // The price overlay colors locations, not people.
        Overlay::Price => 2,
    }
}

/// A location's overlay class under [`Overlay::Price`]: quantile band
/// among all PRICED locations (None = the location doesn't retail, or
/// another overlay is active — the paint layer leaves it uncolored).
pub fn location_overlay_class(
    snapshot: &Snapshot,
    overlay: Overlay,
    location: &LocationRow,
) -> Option<u8> {
    if overlay != Overlay::Price {
        return None;
    }
    let price = location.price_mills?;
    let mut prices: Vec<i64> = snapshot
        .locations
        .iter()
        .filter_map(|row| row.price_mills)
        .collect();
    prices.sort_unstable();
    Some(quantile_class(&prices, price))
}

fn quantile_class(sorted: &[i64], value: i64) -> u8 {
    if sorted.is_empty() {
        return 0;
    }
    // Rank ties LOW (first equal element): a uniform population is all
    // class 0, never all "richest".
    let rank = sorted.partition_point(|other| *other < value);
    (rank * 5 / sorted.len()).min(4) as u8
}

/// One commute flow: `(home district, work district, count)` — the
/// overlay draws thickness by count.
pub fn commute_flows(snapshot: &Snapshot) -> Vec<(u32, u32, u32)> {
    let district_of = |location: Option<u32>| -> Option<u32> {
        location.and_then(|index| {
            snapshot
                .locations
                .iter()
                .find(|row| row.index == index)
                .and_then(|row| row.district)
                // A corrupt/out-of-range district id never flows into
                // the pane (map_columns files those under "(unsited)").
                .filter(|district| (*district as usize) < snapshot.districts.len())
        })
    };
    let mut flows: std::collections::BTreeMap<(u32, u32), u32> = std::collections::BTreeMap::new();
    for citizen in &snapshot.citizens {
        if let (Some(home), Some(work)) = (district_of(citizen.home), district_of(citizen.employer))
        {
            *flows.entry((home, work)).or_insert(0) += 1;
        }
    }
    flows
        .into_iter()
        .map(|((home, work), count)| (home, work, count))
        .collect()
}

/// The abstract map layout (ADR 0013 §4): district columns in data
/// order; locations stack within their district in entity order;
/// un-sited locations form a trailing column. Deterministic.
pub fn map_columns(snapshot: &Snapshot) -> Vec<(String, Vec<u32>)> {
    let mut columns: Vec<(String, Vec<u32>)> = snapshot
        .districts
        .iter()
        .map(|name| (name.clone(), Vec::new()))
        .collect();
    columns.push(("(unsited)".to_owned(), Vec::new()));
    let last = columns.len() - 1;
    for location in &snapshot.locations {
        let column = location
            .district
            .map(|district| district as usize)
            .filter(|district| *district < last)
            .unwrap_or(last);
        columns[column].1.push(location.index);
    }
    columns
}

/// Citizens standing at `location`, entity order (the map's dots and
/// the location tooltip).
pub fn present_at(snapshot: &Snapshot, location: u32) -> Vec<u32> {
    snapshot
        .citizens
        .iter()
        .filter(|row| row.at == Some(location))
        .map(|row| row.index)
        .collect()
}

/// One edge of the focused citizen's relationship graph: the other
/// citizen, the bond, and where the other stands RIGHT NOW (None =
/// despawned or unpositioned) — the overlay draws lines from the
/// focused citizen to each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BondEdge {
    /// The other citizen's entity index.
    pub other: u32,
    /// Their display name ("citizen #N" when despawned).
    pub other_name: String,
    /// The bond kind, rendered.
    pub kind: String,
    /// Bond strength, per-mille.
    pub strength_per_mille: i32,
    /// The other citizen's current location, if any.
    pub other_at: Option<u32>,
}

/// The focused citizen's bonds resolved against the snapshot (SPEC
/// §13's relationship-graph overlay), bond order.
pub fn relationship_graph(snapshot: &Snapshot, index: u32) -> Vec<BondEdge> {
    let Some(row) = snapshot.citizens.iter().find(|row| row.index == index) else {
        return Vec::new();
    };
    row.bonds
        .iter()
        .map(|(other, kind, strength)| {
            let resolved = snapshot
                .citizens
                .iter()
                .find(|citizen| citizen.index == *other);
            BondEdge {
                other: *other,
                other_name: resolved
                    .map(|citizen| citizen.name.clone())
                    .unwrap_or_else(|| format!("citizen #{other}")),
                kind: kind.clone(),
                strength_per_mille: *strength,
                other_at: resolved.and_then(|citizen| citizen.at),
            }
        })
        .collect()
}

/// Normalizes one metrics series into `0..=1` × `0..=1` polyline points
/// (x by day span, y by value range; a flat series pins to y=0.5) — the
/// dashboard's sparkline geometry, computed here so it is TESTED.
pub fn normalize_series(samples: &[(u64, i64)]) -> Vec<(f32, f32)> {
    if samples.is_empty() {
        return Vec::new();
    }
    let (first_day, last_day) = (samples[0].0, samples[samples.len() - 1].0);
    let day_span = (last_day - first_day).max(1) as f32;
    let low = samples.iter().map(|(_, value)| *value).min().unwrap_or(0);
    let high = samples.iter().map(|(_, value)| *value).max().unwrap_or(0);
    samples
        .iter()
        .map(|(day, value)| {
            let x = (day - first_day) as f32 / day_span;
            let y = if high == low {
                0.5
            } else {
                (value - low) as f32 / (high - low) as f32
            };
            (x, y)
        })
        .collect()
}

//! The event browser's tooling layer (Phase 10, ADR 0013 §4): decodes
//! the retained event window into subject-tagged, human-readable rows
//! and filters them — the widgets only draw what these functions
//! return.

use core_ecs::World;
use core_ecs::sim_interface as si;
use core_types::codec;

use crate::snapshot::EventRow;

/// Decodes the whole retained window, oldest first. Unknown event
/// types keep their registered name with no subjects — honest, never
/// silent. Crate-internal: the snapshot is the only caller (ADR 0013
/// §2 — no other path touches `World`).
pub(crate) fn decode_window(world: &World) -> Vec<EventRow> {
    let mut rows = Vec::new();
    for (entry, name) in world.event_system().log() {
        let (subjects, text) = describe(name, &entry.bytes);
        rows.push(EventRow {
            tick: entry.tick.raw(),
            kind: name.to_owned(),
            subjects,
            text,
        });
    }
    rows
}

/// The browser's filter: entity (any subject), event kind, tick range —
/// all optional, all conjunctive (SPEC §13).
pub fn filter<'a>(
    rows: &'a [EventRow],
    entity: Option<u32>,
    kind: Option<&str>,
    ticks: Option<(u64, u64)>,
) -> Vec<&'a EventRow> {
    rows.iter()
        .filter(|row| entity.is_none_or(|index| row.subjects.contains(&index)))
        .filter(|row| kind.is_none_or(|kind| row.kind == kind))
        .filter(|row| ticks.is_none_or(|(from, to)| row.tick >= from && row.tick <= to))
        .collect()
}

// A flat decode table — one arm per registered event type, each arm a
// self-contained (subjects, line) pair. Length comes from coverage, not
// nesting; splitting it would scatter the event catalog.
fn describe(name: &str, bytes: &[u8]) -> (Vec<u32>, String) {
    match name {
        "social.married" => codec::from_bytes::<si::Married>(bytes)
            .map(|event| {
                (
                    vec![event.partner_a.index(), event.partner_b.index()],
                    format!(
                        "citizens #{} and #{} married",
                        event.partner_a.index(),
                        event.partner_b.index()
                    ),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), name.to_owned())),
        "people.born" => codec::from_bytes::<si::Born>(bytes)
            .map(|event| {
                (
                    vec![
                        event.child.index(),
                        event.parent_a.index(),
                        event.parent_b.index(),
                    ],
                    format!(
                        "citizen #{} born to #{} and #{}",
                        event.child.index(),
                        event.parent_a.index(),
                        event.parent_b.index()
                    ),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), name.to_owned())),
        "econ.hired" => codec::from_bytes::<si::Hired>(bytes)
            .map(|event| {
                (
                    vec![event.citizen.index(), event.employer.index()],
                    format!(
                        "citizen #{} hired by #{} at {} mills/day",
                        event.citizen.index(),
                        event.employer.index(),
                        event.wage_per_day.mills()
                    ),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), name.to_owned())),
        "econ.fired" => codec::from_bytes::<si::Fired>(bytes)
            .map(|event| {
                (
                    vec![event.citizen.index(), event.employer.index()],
                    format!(
                        "citizen #{} lost their job at #{}",
                        event.citizen.index(),
                        event.employer.index()
                    ),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), name.to_owned())),
        "econ.goods_purchased" => codec::from_bytes::<si::GoodsPurchased>(bytes)
            .map(|event| {
                (
                    vec![event.buyer.index(), event.seller.index()],
                    format!(
                        "citizen #{} bought {}× good {} from #{} for {} mills",
                        event.buyer.index(),
                        event.quantity,
                        event.good,
                        event.seller.index(),
                        event.total.mills()
                    ),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), name.to_owned())),
        "econ.tenancy_started" => codec::from_bytes::<si::TenancyStarted>(bytes)
            .map(|event| {
                (
                    vec![event.tenant.index(), event.home.index()],
                    format!(
                        "citizen #{} rented home #{} at {} mills/day",
                        event.tenant.index(),
                        event.home.index(),
                        event.rent_per_day.mills()
                    ),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), name.to_owned())),
        "econ.loan_granted" => codec::from_bytes::<si::LoanGranted>(bytes)
            .map(|event| {
                (
                    vec![event.borrower.index()],
                    format!(
                        "citizen #{} borrowed {} mills at {}/1M daily",
                        event.borrower.index(),
                        event.principal.mills(),
                        event.rate_per_million_daily
                    ),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), name.to_owned())),
        "econ.home_sold" => codec::from_bytes::<si::HomeSold>(bytes)
            .map(|event| {
                (
                    vec![
                        event.buyer.index(),
                        event.seller.index(),
                        event.home.index(),
                    ],
                    format!(
                        "#{} bought home #{} from #{} for {} mills",
                        event.buyer.index(),
                        event.home.index(),
                        event.seller.index(),
                        event.price.mills()
                    ),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), name.to_owned())),
        "econ.home_built" => codec::from_bytes::<si::HomeBuilt>(bytes)
            .map(|event| {
                (
                    vec![event.builder.index(), event.home.index()],
                    format!(
                        "builder #{} finished home #{}",
                        event.builder.index(),
                        event.home.index()
                    ),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), name.to_owned())),
        "econ.tax_collected" => codec::from_bytes::<si::TaxCollected>(bytes)
            .map(|event| {
                (
                    vec![event.payer.index()],
                    format!(
                        "#{} paid {} mills {:?} tax",
                        event.payer.index(),
                        event.amount.mills(),
                        event.kind
                    ),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), name.to_owned())),
        "econ.price_changed" => codec::from_bytes::<si::PriceChanged>(bytes)
            .map(|event| {
                (
                    vec![event.firm.index()],
                    format!(
                        "firm #{} repriced good {}: {} → {} mills",
                        event.firm.index(),
                        event.good,
                        event.old.mills(),
                        event.new.mills()
                    ),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), name.to_owned())),
        "econ.loan_defaulted" => codec::from_bytes::<si::LoanDefaulted>(bytes)
            .map(|event| {
                (
                    vec![event.borrower.index()],
                    format!(
                        "borrower #{} defaulted ({} mills written off)",
                        event.borrower.index(),
                        event.written_off.mills()
                    ),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), name.to_owned())),
        "people.person_died" => codec::from_bytes::<sim_people::PersonDied>(bytes)
            .map(|event| {
                (
                    vec![event.person.index()],
                    format!("citizen #{} died", event.person.index()),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), name.to_owned())),
        "people.school_attended" => codec::from_bytes::<si::SchoolAttended>(bytes)
            .map(|event| {
                (
                    vec![event.pupil.index()],
                    format!(
                        "pupil #{} attended school (skill {} now {}/1000)",
                        event.pupil.index(),
                        event.skill,
                        event.new_level
                    ),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), name.to_owned())),
        "lod.tier_changed" => codec::from_bytes::<si::TierChanged>(bytes)
            .map(|event| {
                (
                    vec![event.citizen.index()],
                    format!("citizen #{} changed simulation tier", event.citizen.index()),
                )
            })
            .unwrap_or_else(|_| (Vec::new(), name.to_owned())),
        _ => (Vec::new(), name.to_owned()),
    }
}

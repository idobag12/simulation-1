//! The narrative composer (SPEC §7/§13; ADR 0010 §5): a pure observer
//! over the event-log ring that surfaces coherent story lines from REAL
//! event chains — never injected, only noticed. Patterns are code
//! (presentation logic), the events are the record.

use core_ecs::sim_interface::{Born, Fired, Hired, LoanDefaulted, Married, TenancyStarted};
use core_ecs::{EcsError, Entity, World};
use core_types::calendar::TICKS_PER_DAY;
use core_types::codec;

/// One decoded happening on a subject's timeline.
#[derive(Debug, Clone)]
enum Happening {
    Married { partner: Entity },
    Born { child: Entity, other_parent: Entity },
    Fired,
    Hired,
    Defaulted,
    Rented,
}

/// A citizen's display name, or a stable stand-in for the dead (their
/// events remain even when the entity is gone — stories outlive people).
fn name_of(world: &World, entity: Entity) -> String {
    world
        .get::<sim_people::Identity>(entity)
        .ok()
        .flatten()
        .map(|identity| format!("{} {}", identity.given_name, identity.family_name))
        .unwrap_or_else(|| format!("citizen #{}", entity.index()))
}

/// Composes the retained event log into human-readable story lines,
/// oldest chain first. `subject` filters to one citizen's stories.
pub fn stories(world: &World, subject: Option<u32>) -> Result<Vec<String>, EcsError> {
    // Decode the ring into per-subject timelines (day, happening),
    // preserving log order (oldest first).
    let mut timelines: Vec<(Entity, u64, Happening)> = Vec::new();
    for (entry, name) in world.event_system().log() {
        let day = entry.tick.raw() / TICKS_PER_DAY;
        match name {
            "social.married" => {
                if let Ok(event) = codec::from_bytes::<Married>(&entry.bytes) {
                    timelines.push((
                        event.partner_a,
                        day,
                        Happening::Married {
                            partner: event.partner_b,
                        },
                    ));
                }
            }
            "people.born" => {
                if let Ok(event) = codec::from_bytes::<Born>(&entry.bytes) {
                    timelines.push((
                        event.parent_a,
                        day,
                        Happening::Born {
                            child: event.child,
                            other_parent: event.parent_b,
                        },
                    ));
                }
            }
            "econ.fired" => {
                if let Ok(event) = codec::from_bytes::<Fired>(&entry.bytes) {
                    timelines.push((event.citizen, day, Happening::Fired));
                }
            }
            "econ.hired" => {
                if let Ok(event) = codec::from_bytes::<Hired>(&entry.bytes) {
                    timelines.push((event.citizen, day, Happening::Hired));
                }
            }
            "econ.loan_defaulted" => {
                if let Ok(event) = codec::from_bytes::<LoanDefaulted>(&entry.bytes) {
                    timelines.push((event.borrower, day, Happening::Defaulted));
                }
            }
            "econ.tenancy_started" => {
                if let Ok(event) = codec::from_bytes::<TenancyStarted>(&entry.bytes) {
                    timelines.push((event.tenant, day, Happening::Rented));
                }
            }
            _ => {}
        }
    }

    let mut lines: Vec<String> = Vec::new();
    let visible = |entity: Entity| subject.is_none_or(|index| entity.index() == index);
    for (position, (who, day, happening)) in timelines.iter().enumerate() {
        match happening {
            // Courtship → marriage → birth (the family chain).
            Happening::Married { partner } => {
                if !(visible(*who) || visible(*partner)) {
                    continue;
                }
                let mut line = format!(
                    "{} and {} married on day {day}",
                    name_of(world, *who),
                    name_of(world, *partner)
                );
                let child = timelines[position..]
                    .iter()
                    .find_map(|(_, d2, later)| match later {
                        Happening::Born {
                            child,
                            other_parent,
                        } if other_parent == partner || other_parent == who => Some((*child, *d2)),
                        _ => None,
                    });
                if let Some((child, born_day)) = child {
                    line.push_str(&format!(
                        "; their child {} was born on day {born_day}",
                        name_of(world, child)
                    ));
                }
                line.push('.');
                lines.push(line);
            }
            // Job loss → new work (the labor chain).
            Happening::Fired => {
                if !visible(*who) {
                    continue;
                }
                let rehired = timelines[position..]
                    .iter()
                    .find(|(other, d2, later)| {
                        other == who && d2 >= day && matches!(later, Happening::Hired)
                    })
                    .map(|(_, d2, _)| *d2);
                let line = match rehired {
                    Some(d2) => format!(
                        "{} lost their job on day {day} but found new work by day {d2}.",
                        name_of(world, *who)
                    ),
                    None => format!(
                        "{} lost their job on day {day} and is still looking.",
                        name_of(world, *who)
                    ),
                };
                lines.push(line);
            }
            // Default → foreclosure → renting again (the money chain).
            Happening::Defaulted => {
                if !visible(*who) {
                    continue;
                }
                let renting = timelines[position..]
                    .iter()
                    .find(|(other, d2, later)| {
                        other == who && d2 >= day && matches!(later, Happening::Rented)
                    })
                    .map(|(_, d2, _)| *d2);
                let line = match renting {
                    Some(d2) => format!(
                        "{} defaulted on day {day}; the bank took what was owed, \
                         and they were renting again by day {d2}.",
                        name_of(world, *who)
                    ),
                    None => format!("{} defaulted on a loan on day {day}.", name_of(world, *who)),
                };
                lines.push(line);
            }
            // Births stand alone too (the town's registry).
            Happening::Born {
                child,
                other_parent,
            } => {
                if !(visible(*who) || visible(*other_parent) || visible(*child)) {
                    continue;
                }
                lines.push(format!(
                    "{} was born to {} and {} on day {day}.",
                    name_of(world, *child),
                    name_of(world, *who),
                    name_of(world, *other_parent)
                ));
            }
            Happening::Hired | Happening::Rented => {}
        }
    }
    Ok(lines)
}

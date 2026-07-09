//! Typed event bus, event log ring, and future-event scheduler (SPEC §7,
//! §5; design in ADR 0004 §§1–5).
//!
//! Invariants owned by this crate:
//! - Events are facts, not commands (SPEC §7); they are serializable,
//!   identified by stable explicit names, and delivered in emission order.
//! - Events emitted during tick T become readable during tick T+1 and are
//!   immutable for that whole tick (ADR 0004 §3). Same-tick delivery does
//!   not exist.
//! - Scheduled entries are future event emissions ordered by
//!   `(due_tick, insertion sequence)` (ADR 0004 §2); they fire at the start
//!   of their due tick, ahead of rotated pending events.
//! - All iteration is over `Vec`s and `BTreeMap`s in deterministic order;
//!   the one `HashMap` is lookup-only (SPEC §3).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::any::TypeId;
use std::collections::{BTreeMap, HashMap, VecDeque};

use core_types::codec::{self, CodecError};
use core_types::{StableHasher, Ticks};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// An event type: a serializable fact with a stable identity.
///
/// Invariants:
/// - `NAME` is the event's identity in saves, hashes, and the log — stable
///   forever, namespaced per system (e.g. `"economy.person_hired"`);
///   renaming it is a save-format break (same rules as `Component::NAME`,
///   ADR 0002 §6).
/// - Events describe things that happened; nothing sends orders through
///   the bus (SPEC §7).
pub trait Event: Serialize + DeserializeOwned + 'static {
    /// Stable, namespaced identity.
    const NAME: &'static str;
}

/// Errors produced by the event system.
#[derive(Debug, Error)]
pub enum EventError {
    /// An event type was used before being registered.
    #[error("event `{0}` is not registered")]
    Unregistered(&'static str),
    /// An event name or type was registered twice.
    #[error("event `{0}` is already registered")]
    Duplicate(String),
    /// Saved event state does not match this application's registration.
    #[error("save/world event registration mismatch: {0}")]
    StateMismatch(String),
    /// Canonical encoding/decoding failed.
    #[error(transparent)]
    Codec(#[from] CodecError),
}

/// One scheduled entry: insertion sequence number, registered type index,
/// canonical event bytes. Entries under one due tick fire in sequence
/// order (ADR 0004 §2).
type ScheduledEntry = (u64, u32, Vec<u8>);

/// One retained log record: the tick an event became readable, its
/// registered type index, and its canonical bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Tick at which the event was delivered (became readable).
    pub tick: Ticks,
    /// Index into the registration order (stable given fixed registration).
    pub type_index: u32,
    /// Canonically encoded event payload.
    pub bytes: Vec<u8>,
}

/// The serializable portion of [`Events`] (everything except capacity,
/// which is configuration, and the lookup map, which is rebuilt from
/// registration).
///
/// Invariant: `names` records the registration order the state was saved
/// under; loading verifies it matches the live registration exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EventsState {
    names: Vec<String>,
    pending: Vec<(u32, Vec<u8>)>,
    readable: Vec<(u32, Vec<u8>)>,
    log: Vec<LogEntry>,
    scheduled: Vec<(Ticks, Vec<ScheduledEntry>)>,
    next_sequence: u64,
}

/// The event bus + log ring + future-event queue. Owned by the world
/// (ADR 0004 §1); driven by the tick loop via [`Events::begin_tick`].
///
/// Invariants:
/// - Registration order is fixed per application and is part of the
///   save/hash format.
/// - `pending` accumulates this tick's emissions; `readable` is immutable
///   during a tick and holds last tick's emissions plus this tick's due
///   scheduled events (scheduled first).
/// - The log ring holds the most recent `log_capacity` delivered events;
///   no simulation system may read it (observability only, ADR 0004 §5).
#[derive(Debug)]
pub struct Events {
    names: Vec<&'static str>,
    // Lookup-only map (TypeId → registration index); never iterated (SPEC §3).
    by_type: HashMap<TypeId, u32>,
    pending: Vec<(u32, Vec<u8>)>,
    readable: Vec<(u32, Vec<u8>)>,
    log: VecDeque<LogEntry>,
    log_capacity: usize,
    scheduled: BTreeMap<Ticks, Vec<ScheduledEntry>>,
    next_sequence: u64,
}

impl Events {
    /// Creates an empty event system retaining at most `log_capacity`
    /// delivered events (a tunable from `data/balance/engine.ron`).
    pub fn new(log_capacity: usize) -> Self {
        Events {
            names: Vec::new(),
            by_type: HashMap::new(),
            pending: Vec::new(),
            readable: Vec::new(),
            log: VecDeque::new(),
            log_capacity,
            scheduled: BTreeMap::new(),
            next_sequence: 0,
        }
    }

    /// Registers an event type. Registration order is part of the
    /// save/hash format; call in one fixed order per application.
    pub fn register<E: Event>(&mut self) -> Result<(), EventError> {
        if self.by_type.contains_key(&TypeId::of::<E>()) || self.names.contains(&E::NAME) {
            return Err(EventError::Duplicate(E::NAME.to_owned()));
        }
        self.by_type
            .insert(TypeId::of::<E>(), self.names.len() as u32);
        self.names.push(E::NAME);
        Ok(())
    }

    fn type_index<E: Event>(&self) -> Result<u32, EventError> {
        self.by_type
            .get(&TypeId::of::<E>())
            .copied()
            .ok_or(EventError::Unregistered(E::NAME))
    }

    /// Emits a fact. It becomes readable at the start of the next tick
    /// (ADR 0004 §3); emission order is preserved.
    pub fn emit<E: Event>(&mut self, event: &E) -> Result<(), EventError> {
        let index = self.type_index::<E>()?;
        self.pending.push((index, codec::to_bytes(event)?));
        Ok(())
    }

    /// Schedules `event` to be delivered at the start of tick `due` (SPEC
    /// §5 future callbacks, as future facts per ADR 0004 §2). Same-tick
    /// entries deliver in scheduling order. Scheduling for a tick that has
    /// already begun delivers at the next tick start.
    pub fn schedule<E: Event>(&mut self, due: Ticks, event: &E) -> Result<(), EventError> {
        let index = self.type_index::<E>()?;
        let bytes = codec::to_bytes(event)?;
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.scheduled
            .entry(due)
            .or_default()
            .push((sequence, index, bytes));
        Ok(())
    }

    /// Reads this tick's events of type `E`, decoded, in delivery order
    /// (due scheduled events first, then last tick's emissions).
    pub fn read<E: Event>(&self) -> Result<Vec<E>, EventError> {
        let index = self.type_index::<E>()?;
        self.readable
            .iter()
            .filter(|(i, _)| *i == index)
            .map(|(_, bytes)| codec::from_bytes(bytes).map_err(EventError::from))
            .collect()
    }

    /// Tick-start transition (ADR 0004 §3), called exactly once per tick by
    /// the tick loop before any system runs:
    /// 1. retire the previous readable set,
    /// 2. fire scheduled entries with `due <= tick` (the start-of-tick
    ///    queue) in `(due, sequence)` order,
    /// 3. rotate pending emissions in after them,
    /// 4. append everything delivered to the log ring (bounded).
    pub fn begin_tick(&mut self, tick: Ticks) {
        self.readable.clear();

        let due_ticks: Vec<Ticks> = self.scheduled.range(..=tick).map(|(due, _)| *due).collect();
        for due in due_ticks {
            if let Some(entries) = self.scheduled.remove(&due) {
                for (_seq, index, bytes) in entries {
                    self.readable.push((index, bytes));
                }
            }
        }
        self.readable.append(&mut self.pending);

        for (type_index, bytes) in &self.readable {
            self.log.push_back(LogEntry {
                tick,
                type_index: *type_index,
                bytes: bytes.clone(),
            });
        }
        while self.log.len() > self.log_capacity {
            self.log.pop_front();
        }
    }

    /// The retained log, oldest first, with resolved event names.
    /// Observability only (debugger, narrative composer); simulation
    /// systems must not read it (ADR 0004 §5).
    pub fn log(&self) -> impl Iterator<Item = (&LogEntry, &'static str)> + '_ {
        self.log.iter().map(|entry| {
            let name = self
                .names
                .get(entry.type_index as usize)
                .copied()
                .unwrap_or("<unregistered>");
            (entry, name)
        })
    }

    /// Number of entries currently scheduled for the future.
    pub fn scheduled_count(&self) -> usize {
        self.scheduled.values().map(Vec::len).sum()
    }

    fn state(&self) -> EventsState {
        EventsState {
            names: self.names.iter().map(|n| (*n).to_owned()).collect(),
            pending: self.pending.clone(),
            readable: self.readable.clone(),
            log: self.log.iter().cloned().collect(),
            scheduled: self
                .scheduled
                .iter()
                .map(|(due, entries)| (*due, entries.clone()))
                .collect(),
            next_sequence: self.next_sequence,
        }
    }

    /// Canonical bytes of the full event state (queues, log, scheduler,
    /// sequence counter, registration names) for saving and hashing.
    pub fn to_bytes(&self) -> Result<Vec<u8>, EventError> {
        Ok(codec::to_bytes(&self.state())?)
    }

    /// Restores event state from canonical bytes. The saved registration
    /// names must match the live registration exactly (strict, no
    /// defaults); log contents beyond the configured capacity are dropped
    /// oldest-first (ring semantics under a shrunk capacity).
    pub fn restore(&mut self, bytes: &[u8]) -> Result<(), EventError> {
        let state: EventsState = codec::from_bytes(bytes)?;
        let live: Vec<String> = self.names.iter().map(|n| (*n).to_owned()).collect();
        if state.names != live {
            return Err(EventError::StateMismatch(format!(
                "saved event registration {:?} != live registration {:?}",
                state.names, live
            )));
        }
        self.pending = state.pending;
        self.readable = state.readable;
        self.log = state.log.into();
        while self.log.len() > self.log_capacity {
            self.log.pop_front();
        }
        self.scheduled = state.scheduled.into_iter().collect();
        self.next_sequence = state.next_sequence;
        Ok(())
    }

    /// Absorbs the full event state into the world hash, length-framed.
    pub fn hash_into(&self, hasher: &mut StableHasher) -> Result<(), EventError> {
        hasher.write_frame(&self.to_bytes()?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Ping(u64);
    impl Event for Ping {
        const NAME: &'static str = "test.ping";
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Pong(String);
    impl Event for Pong {
        const NAME: &'static str = "test.pong";
    }

    fn events() -> Events {
        let mut e = Events::new(8);
        e.register::<Ping>().unwrap();
        e.register::<Pong>().unwrap();
        e
    }

    #[test]
    fn emissions_become_readable_next_tick_in_order() {
        let mut e = events();
        e.begin_tick(Ticks::new(0));
        e.emit(&Ping(1)).unwrap();
        e.emit(&Pong("a".into())).unwrap();
        e.emit(&Ping(2)).unwrap();
        // Not visible in the same tick.
        assert!(e.read::<Ping>().unwrap().is_empty());

        e.begin_tick(Ticks::new(1));
        assert_eq!(e.read::<Ping>().unwrap(), vec![Ping(1), Ping(2)]);
        assert_eq!(e.read::<Pong>().unwrap(), vec![Pong("a".into())]);

        // Retired after the next rotation.
        e.begin_tick(Ticks::new(2));
        assert!(e.read::<Ping>().unwrap().is_empty());
    }

    #[test]
    fn scheduled_events_fire_at_their_tick_in_sequence_order_before_emissions() {
        let mut e = events();
        e.schedule(Ticks::new(5), &Ping(50)).unwrap();
        e.schedule(Ticks::new(3), &Ping(30)).unwrap();
        e.schedule(Ticks::new(3), &Ping(31)).unwrap();

        e.begin_tick(Ticks::new(2));
        e.emit(&Ping(999)).unwrap(); // emitted during tick 2
        assert!(e.read::<Ping>().unwrap().is_empty());

        e.begin_tick(Ticks::new(3));
        // Due scheduled (insertion order) first, then last tick's emission.
        assert_eq!(
            e.read::<Ping>().unwrap(),
            vec![Ping(30), Ping(31), Ping(999)]
        );

        e.begin_tick(Ticks::new(4));
        assert!(e.read::<Ping>().unwrap().is_empty());
        e.begin_tick(Ticks::new(5));
        assert_eq!(e.read::<Ping>().unwrap(), vec![Ping(50)]);
        assert_eq!(e.scheduled_count(), 0);
    }

    #[test]
    fn past_due_entries_fire_at_the_next_tick_start() {
        let mut e = events();
        e.begin_tick(Ticks::new(10));
        e.schedule(Ticks::new(4), &Ping(4)).unwrap(); // already past
        e.begin_tick(Ticks::new(11));
        assert_eq!(e.read::<Ping>().unwrap(), vec![Ping(4)]);
    }

    #[test]
    fn unregistered_event_is_a_typed_error() {
        #[derive(Debug, Serialize, Deserialize)]
        struct Ghost;
        impl Event for Ghost {
            const NAME: &'static str = "test.ghost";
        }
        let mut e = events();
        assert!(matches!(
            e.emit(&Ghost),
            Err(EventError::Unregistered("test.ghost"))
        ));
        assert!(matches!(
            e.read::<Ghost>(),
            Err(EventError::Unregistered("test.ghost"))
        ));
    }

    #[test]
    fn duplicate_registration_is_a_typed_error() {
        let mut e = events();
        assert!(matches!(
            e.register::<Ping>(),
            Err(EventError::Duplicate(_))
        ));
    }

    #[test]
    fn log_ring_keeps_only_the_newest_entries() {
        let mut e = events();
        for tick in 0..20u64 {
            e.begin_tick(Ticks::new(tick));
            e.emit(&Ping(tick)).unwrap();
        }
        e.begin_tick(Ticks::new(20));
        let logged: Vec<u64> = e
            .log()
            .map(|(entry, name)| {
                assert_eq!(name, "test.ping");
                entry.tick.raw()
            })
            .collect();
        // Capacity 8: only the 8 most recent deliveries survive. Ping(t) is
        // delivered at t+1, so deliveries at ticks 13..=20 remain.
        assert_eq!(logged, (13..=20).collect::<Vec<_>>());
    }

    #[test]
    fn save_restore_round_trips_mid_flight_state() {
        let mut e = events();
        e.begin_tick(Ticks::new(0));
        e.emit(&Ping(1)).unwrap();
        e.schedule(Ticks::new(7), &Pong("later".into())).unwrap();
        e.begin_tick(Ticks::new(1));
        e.emit(&Ping(2)).unwrap(); // pending at save time

        let bytes = e.to_bytes().unwrap();
        let mut restored = events();
        restored.restore(&bytes).unwrap();

        // Bit-identical serialized state and identical future behavior.
        assert_eq!(restored.to_bytes().unwrap(), bytes);
        e.begin_tick(Ticks::new(2));
        restored.begin_tick(Ticks::new(2));
        assert_eq!(e.read::<Ping>().unwrap(), restored.read::<Ping>().unwrap());
        for t in 3..=7 {
            e.begin_tick(Ticks::new(t));
            restored.begin_tick(Ticks::new(t));
        }
        assert_eq!(e.read::<Pong>().unwrap(), vec![Pong("later".into())]);
        assert_eq!(e.read::<Pong>().unwrap(), restored.read::<Pong>().unwrap());
    }

    #[test]
    fn restore_rejects_registration_mismatch() {
        let e = events();
        let bytes = e.to_bytes().unwrap();
        let mut other = Events::new(8);
        other.register::<Ping>().unwrap(); // missing Pong
        assert!(matches!(
            other.restore(&bytes),
            Err(EventError::StateMismatch(_))
        ));
    }
}

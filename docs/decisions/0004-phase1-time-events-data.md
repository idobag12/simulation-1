# ADR 0004 — Phase 1 design: calendar, event bus, scheduler, RON data, save v2

- Status: Accepted
- Date: 2026-07-09
- Phase: 1

SPEC §5 (time), §7 (events), §8 (data), and §9 (saves) underdetermine
several Phase 1 mechanics; per SPEC §16.7 each decision is recorded here
before the code.

## 1. Event bus and scheduler live in `core_events`, are owned by `World`

The bus's buffered events, the log ring, and the future-event queue are
**world state**: they must be saved, restored, and hashed (SPEC §9 lists
"scheduler queue, event ring tail"), and systems must reach them during a
tick. Systems receive only `&mut World`, so this state sits inside `World`
(exactly like the RNG registry, ADR 0002 §4), as one `Events` value defined
in `core_events`.

Deviation note: SPEC §4/§5 place "scheduling" in `sim_time`. The *queue*
(state) lives in `core_events` beside the bus because `core_ecs` may not
depend upward on `sim_time`; `sim_time` still owns *driving* it — the tick
loop asks the world to fire due entries at the start of each tick. Clean
layering beats file placement; the behavior contract of SPEC §5 is
unchanged.

## 2. Scheduled entries are future event emissions

SPEC §5 calls scheduler entries "future callbacks", but closures are not
serializable and "events are facts, not commands" (SPEC §7). A scheduled
entry is therefore `(due_tick, sequence, event)`: when `due_tick` arrives
the event is emitted onto the bus, and systems react to it like any other
fact. Ordering among same-tick entries is by insertion sequence number
(SPEC §5). Sequence numbers are a saved `u64` counter; they never reset.

## 3. Event delivery semantics: emitted during tick T ⇒ readable during tick T+1

SPEC §7 defines per-tick buffering with start-of-tick and end-of-tick drain
points. Phase 1 implements the single semantic every consumer so far needs:

- `emit` appends to the **pending** queue (emission order preserved).
- At the start of each tick — after due scheduled entries fire (they are
  the start-of-tick queue) — the pending queue rotates to become the
  **readable** queue, immutable for the whole tick, and the previous
  readable events are retired into the log ring.
- Systems iterate `world.events::<E>()` over the readable queue in emission
  order.

Same-tick delivery (a system reading events emitted earlier in its own
tick) is NOT provided; it would couple correctness to schedule order in a
way nothing yet requires (SPEC §16.3). If a later phase needs it, that
phase amends this ADR.

## 4. Event types are registered like components

`trait Event: Serialize + DeserializeOwned + 'static { const NAME: &str }`
with per-application registration in one fixed order; `NAME` (never
`type_name`) identifies the event in saves, hashes, and the log. Buffers
and the log store events in canonically encoded form tagged by the
registered name (encode-on-emit, decode-on-read) — the same stable-identity
rules as components (ADR 0002 §6). Emitting or reading an unregistered
event type is a typed error.

## 5. Event log ring

Bounded ring of `(tick, name, bytes)` in emission order; capacity comes
from `data/balance/engine.ron` (it shapes debugger/narrative reach, so it
is a tunable, SPEC §8). The ring is part of saves and of the world hash
(SPEC §9 "event ring tail"). The ring is observability infrastructure; no
simulation system may read it (systems read the live queues), so its
capacity cannot affect simulation trajectories — asserted by a test that
runs the same seed under two capacities and compares component/RNG state.

## 6. Calendar

`CalendarTime` (year, season, day-of-season, hour, minute) is a plain value
type in `core_types` so `TickContext` can carry it without upward
dependencies. `sim_time::Calendar` converts ticks using fixed definitional
structure — 1 tick = 1 minute, 60 minutes/hour, 24 hours/day (1440
ticks/day, all fixed by SPEC §5) and 4 seasons/year — plus one tunable,
`days_per_season`, from `data/balance/calendar.ron` (SPEC §8). Season names
are the fixed enum Spring/Summer/Autumn/Winter.

## 7. Multi-rate schedules and boundary order

`Schedule` gains the four coarser rate lists (hour/day/season/year). A rate
fires on the tick that *begins* its period (tick 0 begins hour 0, day 0,
season 0, year 0). Within one tick the order is: due scheduled events fire
→ event queues rotate → **year → season → day → hour → tick** systems run
(coarse bookkeeping like payroll settles before fine-grained agents act
within the same tick), each system followed by its command-buffer
application as in Phase 0. This order is part of the spec of the schedule
(SPEC §6) and is asserted by tests.

## 8. `data_defs`: RON registries and validation

`data_defs` owns loading `data/**.ron` into immutable, validated config
structs (`ron` crate — sanctioned by SPEC §2 "RON for authored data
definitions"). Phase 1 loads `data/balance/calendar.ron` and
`data/balance/engine.ron`. Loader rules, enforced by tests: unknown fields
are errors (`deny_unknown_fields`), missing files are errors with the path
in the message, out-of-range values (e.g. `days_per_season == 0`, zero
event-log capacity) are precise validation errors — never silent defaults
(SPEC §8). Registries with integer ids arrive with the first id-keyed
content (goods, Phase 4); building the id machinery for two config files
would be speculative (SPEC §16.3).

## 9. Save format v2 and the migration pipeline

`SaveBody` gains the events blob (bus queues + log ring + scheduler queue +
sequence counter). `FORMAT_VERSION` bumps to 2 and the pipeline gains its
first real migration. Refinement of SPEC §9's "intermediate dynamic
representation": the intermediate representation is the **frozen previous
body struct** (`SaveBodyV1`), kept verbatim in `migrations.rs`;
`v1_to_v2 : SaveBodyV1 -> SaveBodyV2` is a pure function (v1 worlds get
empty event state — exactly what they had). With bincode there is no
self-describing dynamic form to parse into; frozen versioned structs are
the representation. The chain composes for future versions
(`v1→v2→…→current`), old saves load forever, and the committed v1 golden
fixture now exercises the real migration path in CI.

## 10. Golden-hash re-recording policy

Adding event state to the world extends the domain of `hash_world()`, so
Phase 0's recorded golden hashes change meaning. Policy (applies to every
future hash-domain change): the same commit that changes the hash domain
re-records golden hashes, states why in its message, and must ALSO prove
continuity — the v1 fixture still loads through migration, and its
migrated component/entity/RNG content is byte-identical to what v1
contained (asserted by comparing the migrated component blobs against the
fixture's original bytes). Re-recording a golden without a hash-domain
change in the same commit is forbidden.

## Dependencies added (SPEC §2 requires written justification)

| Crate | Why | Sanctioned by |
|---|---|---|
| `ron` | authored data definitions | SPEC §2 explicitly ("RON for authored data definitions") |

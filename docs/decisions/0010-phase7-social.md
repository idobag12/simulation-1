# ADR 0010 — Phase 7 design: the social layer

- Status: Accepted
- Date: 2026-07-10
- Phase: 7

SPEC §15 Phase 7 ("Relationships graph, memory/beliefs, gossip,
courtship/marriage/reproduction, education pipeline affecting skills.
Exit: multi-generation run produces kinship networks and skill
mobility; narrative composer surfaces coherent story lines from real
event chains") against §11's memory/relationship bullets. Per SPEC
§16.7 the decisions precede the code; per §16.3 (YAGNI) each mechanic
ships in its smallest honest form that the exit criteria exercise.

## 1. Skills and the education pipeline

`Skills { levels: Vec<u16> }` (shared, per-mille mastery per skill in
`data/skills.ron` order — a new citizen component, ZERO rows for
migrated citizens: migrations never invent competence). Two honest
sources, both data-defined:

- **School** (`data/skills.ron`: taught skill, per-attendance gain,
  school ages): a `school` location kind (count 1, public — claimed by
  taxes.ron alongside the town hall; the treasury's public workers
  abstractly staff both, no per-teacher modeling in Phase 7).
  School-age citizens get an `AttendSchool` action variant, biased
  during data school hours exactly like the work shift (obligations
  bias, never dictate); attendance raises the taught skill by the data
  gain, saturating at 1000.
- **Learning by doing**: each worked shift raises the employer's
  recipe skill (`recipe.skill_id`) by a data gain — slower than school.

Skills bite in the labor market (SPEC §12 "vacancies (wage, skill
req)" scaled to Phase 7's honest core): a firm's bid for a worker is
multiplied by `(1000 + skill_weight_per_mille × level/1000)/1000` of
its recipe's skill — skilled workers command measurably higher wages,
which IS the mobility the exit criterion measures. Public slots use
the school's taught skill. No hard skill gates (a town this small
cannot afford them; documented deferral).

## 2. Relationships

`Relationships { edges: Vec<Edge> }` per citizen (sparse, data-capped;
the weakest non-kin edge evicts on overflow). `Edge { other: Entity,
kind: RelKind, strength_per_mille: i32 }`; `RelKind { Kin, Spouse,
Friend, Romance }` — professional/rivalry edges are later texture
(deferral documented; nothing in the exit criteria needs them).

- **Kinship** is written by facts, never decayed: parent/child edges at
  birth, spouse edges at marriage, sibling edges between a newborn and
  existing children. This is the kinship network the exit criterion
  counts.
- **Friendship/romance drift by co-presence**: a day-rate system scans
  each location's present citizens (entity order); pairs sharing a
  leisure location gain data drift on a Friend edge (Romance instead
  when both are single adults and a data compatibility screen passes —
  trait distance, opposite... no: data-defined sociability product
  only; anything richer is Phase 8+ texture). All edges except kin
  decay per-day by data amounts; edges at zero drop.
- Bonds feed utility: satisfying the `social` need at a location gains
  a multiplier per present friend (data weight × strength) — visiting
  friends genuinely satisfies more (SPEC §11).

## 3. Memory, beliefs, and gossip

Phase 7 ships SEMANTIC beliefs only — the narrative composer reads the
EVENT LOG, not per-citizen episodic memory, so episodic rings are
deferred (documented; §11 lists them under the full AI, no Phase 7
exit criterion touches them).

`Beliefs { prices: Vec<(Entity, Money)> }` per citizen (bounded, data
cap): the believed unit price of each retail shop, updated exactly by
- **experience**: every completed purchase writes the paid price;
- **gossip**: the co-presence system also averages price beliefs
  between interacting pairs (integer midpoint, deterministic order) —
  cheap-shop knowledge propagates through the social graph.

Beliefs bite in purchase scoring: the shop-choice money cost uses the
BELIEVED price when one exists (falling back to the posted price);
citizens discover real prices at the till (the belief then corrects —
being wrong is honest and self-healing).

## 4. Courtship, marriage, reproduction

- **Courtship**: the romance drift above; a `court` goal is implicit in
  the edge (no separate goal object — YAGNI).
- **Marriage** (day-rate, deterministic): a Romance edge crossing the
  data threshold with BOTH parties single adults marries them: spouse
  edges, `Married` event, household merge (the smaller-index household
  absorbs; the vacated home — owned or rented — releases to the
  housing market: marriage is a supply event, exactly like death).
- **Reproduction** (day-rate): married couples sharing a residence
  face a data fertility-per-day chance (age-banded, from
  `data/balance/fertility.ron`, drawn on the `people.fertility`
  stream), capped by data household size. A birth spawns a child:
  heritable traits = per-trait midpoint of the parents ± mutation
  (data per-mille range, same stream), age 0, joins the household and
  home, kinship edges written, `Born` event. Children age by the
  existing calendar, school between the data ages, promote to working
  age by the existing Phase 5 birthday system — the full pipeline
  closes: born → schooled → skilled → hired → wed → parent.

## 5. The narrative composer

`debug_tools::narrative` (SPEC §7, §13): a pure function over the
event log ring — no new state, stories are OBSERVED. It groups
high-signal events by subject entity and composes chains into
human-readable lines when data-defined patterns match (job loss →
eviction; courtship → marriage → birth; default → foreclosure →
rental). Surfaced by a new CLI command `embervale stories --load PATH
[--entity N]`. The ring's capacity bounds the window — stories are
recent history, which is honest for a debugger (the exit criterion
demands coherent lines from real chains, not an infinite archive).

## 6. Save format v8

Components (append): `people.skills, social.relationships,
social.beliefs`. Events: `social.married, people.born,
people.school_attended` (the skill-mobility measurement hooks).
FORMAT_VERSION 8, mechanical v7→v8 chained from v1, continuity proofs,
goldens re-recorded with reasons, new v8 fixture, pinned snapshot
`data_v8`. Migrated towns: no skills, no edges, no beliefs — every new
system no-ops over empty stores and builds state from lived events
(the catch-up lesson); nothing is invented.

## 7. Data

`data/skills.ron` (skill list; school: taught skill, gain, ages,
hours; learning-by-doing gain; labor skill_weight_per_mille; recipe →
skill map by recipe id). `data/balance/social.ron` (edge cap, drifts,
decays, romance screen, marriage threshold, social-bond utility
weight, gossip: belief cap + exchange rule). `data/balance/
fertility.ron` (age bands per-billion daily chance, household cap,
trait mutation per-mille). `data/locations.ron` appends `school`
(count 1); recipes gain optional `skill_id`. Validation: every id
resolves, rates/ranges bounded, school ages below working age,
fertility bands ordered, mutation ≤ 1000.

## 8. Exit criteria mapping

- *Multi-generation run produces kinship networks*: a data-shaped
  fast-generation town (the flat-mortality pattern applied to time:
  `days_per_season` small, fertility high, school ages compressed —
  all legal data) runs until citizens exist with grandparent kinship
  chains: assert ≥ 2 generations of parent edges linked through a
  living or dead middle generation, plus spouse/sibling edges — a real
  kinship NETWORK, counted from the graph.
- *…and skill mobility*: in the same run, assert children of
  low-skill parents reach skill levels their parents never held (via
  school), and that wages for the skilled measurably exceed the
  unskilled at the same firm kind — mobility in both the skill and the
  wage sense, measured, never assigned.
- *Narrative composer surfaces coherent story lines from real event
  chains*: seed a run whose events provably contain a chain (marriage
  after courtship, birth after marriage — from the generation run) and
  assert the composer emits the corresponding line citing the true
  entities and ticks; plus a unit suite over hand-built event chains
  for each pattern.

## Dependencies

No new external dependencies.

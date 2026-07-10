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
req)" scaled to Phase 7's honest core) through the RESERVATION side
(**amended:** a uniform double-auction bid cannot know its match; the
ask knows its owner): a worker's reservation wage gains `base ×
weight/1000 × skill/1000` of their best skill — mastery is an outside
option, and skilled workers clear at measurably higher midpoints.
Working a paid day raises the employer's recipe skill; public slots
teach the public skill. No hard skill gates (a town this small cannot
afford them; documented deferral).

## 2. Relationships

`Relationships { edges: Vec<Edge> }` per citizen (sparse, data-capped;
the weakest non-kin edge evicts on overflow). `Edge { other: Entity,
kind: RelKind, strength_per_mille: i32 }`; `RelKind { Kin, Spouse,
Friend, Romance }` — professional/rivalry edges are later texture
(deferral documented; nothing in the exit criteria needs them).
**Amended:** the cap bounds DRIFT edges only — Kin and Spouse edges are
privileged and always record (evicting the weakest drift edge, or
exceeding the cap when only family remains): a silently dropped kin or
spouse edge makes the graph one-sided, defeating the incest screen and
the bigamy check and locking widows to corpses.

- **Kinship** is written by facts, never decayed: parent/child edges at
  birth, spouse edges at marriage, sibling edges between a newborn and
  existing children. This is the kinship network the exit criterion
  counts.
- **Friendship/romance drift by co-presence**: an hour-rate system
  scans each leisure venue's present citizens (entity order); each
  meets the next present citizen, and single working-age adults
  ADDITIONALLY meet the next single (**amended:** without the singles
  pass, the last singles sit between married neighbors and never pair
  — the generations stop). Pairs gain data drift on a Friend edge —
  Romance instead when both are single adults, NOT kin (**amended:**
  siblings do not court), and the data sociability-product screen
  passes. All edges except kin/spouse decay per-day; zero edges drop.
  Death removes the survivor's SPOUSE edge (**amended:** widows may
  love again; kin edges remain — the dead stay family).
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
being wrong is honest and self-healing). **Amended:** at the cap,
experience ALWAYS lands (the most-expensive believed shop is forgotten
first — bargains are worth remembering, and the forgotten shop falls
back to its posted price); only hearsay is dropped at the cap.

## 4. Courtship, marriage, reproduction

- **Courtship**: the romance drift above; a `court` goal is implicit in
  the edge (no separate goal object — YAGNI).
- **Marriage** (day-rate, deterministic): a Romance edge crossing the
  data threshold with BOTH parties single non-kin adults marries them
  (**amended:** the kin bar is checked here too, both sides — defense
  in depth behind the drift screen): spouse
  edges, `Married` event, and a NEW household of exactly the couple
  (**amended in-phase:** the originally designed merge COMPOUNDED —
  every wedding unioned two extended families into one ever-growing
  register that busted the fertility cap and sterilized the town; a
  family is a couple and its children). The birth families keep their
  registers; a partner's own home houses the couple, a homeless pair
  of nest-leavers starts under the in-laws' roof (their own register,
  the multi-generation HOME), and the vacated tenancy releases to the
  market. **Amended:** the wedding clears EVERY Romance edge touching
  either newlywed — their own old flames and third parties' edges
  toward them (fidelity resets courtship; otherwise a stale
  above-threshold flame remarries a widow the day after the funeral —
  widows love again by NEW courtship).
- **Reproduction** (day-rate): married couples sharing a residence
  face a data fertility-per-day chance (age-banded, from
  `data/balance/fertility.ron`, drawn on the `people.fertility`
  stream), capped by data household size (**amended:** the cap counts
  the household members UNDER THE MOTHER'S ROOF — nest-left adult
  children keep their register entry but not the bedroom, and must
  not sterilize their parents; validation requires the cap ≥ 3, since
  the couple itself counts). A birth spawns a child:
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
human-readable lines when its patterns match (marriage → birth; job
loss → new work; default → renting again). **Amended:** patterns are
CODE, not data — they are presentation logic, and SPEC §8's grep test
targets tunables, not prose. Surfaced by a new CLI command `embervale stories --load PATH
[--entity N]`. The ring's capacity bounds the window — stories are
recent history, which is honest for a debugger (the exit criterion
demands coherent lines from real chains, not an infinite archive).

## 6. Save format v8

Components (append): `people.skills, social.relationships,
social.beliefs, people.school_age` (the birthday pass maintains the
school window like the working-age marker). Events: `social.married, people.born,
people.school_attended` (the skill-mobility measurement hooks).
FORMAT_VERSION 8, mechanical v7→v8 chained from v1, continuity proofs,
goldens re-recorded with reasons, new v8 fixture, pinned snapshot
`data_v8`. Migrated towns: no skills, no edges, no beliefs — every new
system no-ops over empty stores and builds state from lived events
(the catch-up lesson); nothing is invented. That includes the SCHOOL:
`Location.kind` is a persisted index, so the school kind is APPENDED
at the END of `locations.ron` (pre-v8 town halls keep their kind), and
migration spawns no school building — a migrated town's children
cannot attend until some future phase builds one (documented decision;
learning-by-doing still teaches them, and the guard is asserted by the
v7 save-compat suite).

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
  school) — mobility in the skill sense, measured in the run.
  **Amended:** the WAGE sense is proven at the unit level instead (a
  master's reservation clears above a novice's, the exact premium the
  labor market prices): a run-level wage comparison confounds the wage
  AT HIRE with the skill NOW — a worker hired young at a low wage may
  be the town's most skilled by the measurement day, so measured run
  wages do not cleanly order by current skill even when the mechanism
  is correct.
- *Narrative composer surfaces coherent story lines from real event
  chains*: seed a run whose events provably contain a chain (marriage
  after courtship, birth after marriage — from the generation run) and
  assert the composer emits the corresponding line citing the true
  entities and ticks; plus a unit suite over hand-built event chains
  for each pattern.

## Dependencies

No new external dependencies.

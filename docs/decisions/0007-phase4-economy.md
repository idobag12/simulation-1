# ADR 0007 — Phase 4 design: goods, firms, posted-price market, auditors

- Status: Accepted
- Date: 2026-07-09
- Phase: 4

SPEC §15 Phase 4 ("Inventories, recipes, a 5-good chain, firms with
double-entry ledgers, posted-price retail market, citizens buy food with
seeded wealth; money & goods auditors go live") and §12 underdetermine
mechanics; per SPEC §16.7 the decisions precede the code.

## 1. The production chain and who eats what

Goods (data order = stable id): `grain, flour, water, bread, timber,
firewood`. Firm kinds and recipes (all data): farm→grain, well→water,
forest→timber (harvest recipes: explicit modeled sources with no good
inputs), mill grain→flour, bakery flour+water→bread, woodyard
timber→firewood. Two retail endpoints face citizens: **bakery** sells
bread (satisfies hunger) and **woodyard** sells firewood (satisfies
shelter — the purchase abstracts the hearth; burning firewood over time
is later-phase texture). The needs that became economic lose their free
satisfiers (data edits): tavern/well drop hunger AND the home drops its
free shelter satisfier — a free hearth would dominate the firewood
market and make the woodyard dead code; shelter is satisfied by the
purchase, per this section's design. The well keeps a small free social
satisfier in exchange (the village well is a gathering spot), so the
kind stays live data. Social/purpose/esteem/rest remain non-economic
this phase.

## 2. Money: wallets, seeded wealth, issuance

`Wallet { cash: Money }` (shared, `sim_interface`) on citizens and firms.
Genesis seeds citizen wallets from a data range and firm wallets from
firm archetypes; **the sum of everything seeded is recorded as issuance**
in the `EconCounters` singleton at the moment of creation — the one
explicitly modeled money source. Nothing else creates or destroys money;
every later movement is a transfer. (Banking/issuance-as-actor is
Phase 6; imports/exports join when the economy opens, also Phase 6+.)

Goods seeding follows the same rule symmetrically: firm archetypes seed
initial stock, and every seeded unit is recorded in the `produced`
counter at creation — `produced` means "every unit that ever entered the
world" (genesis seed + recipe output), which is what makes the §4 goods
identity exact from tick 0 without a separate seed term.

## 3. Ledgers: Phase 4's honest double-entry slice

Full account trees arrive with banking. Phase 4 firms carry
`FirmBooks { initial_cash, revenue, expenses }` (shared — the purchase
system writes the revenue side) beside their `Wallet` and `Inventory`.
The double-entry identity this makes testable per firm, every day:
`cash − initial_cash == revenue − expenses`. Any transaction that moves
cash without booking revenue/expense breaks the identity and the test.
This is the seam the Phase 6 ledger expands, not a fake of it.

## 4. Atomic transactions and exact counters

Conservation is enforceable only if mutation and bookkeeping are atomic.
Every operation that moves money or goods updates, IN THE SAME SYSTEM
CALL: both wallets/inventories, both firms' books, and the
`EconCounters` singleton (`issued`, per-good `produced`,
`consumed_by_citizens`, `consumed_in_production`, `spoiled`). Events
(`GoodsPurchased`, `PriceChanged` — shared facts, defined in
`sim_interface`) are emitted for observability, never used to reconstruct
state. The per-good audit identity:
`Σ inventories == produced − consumed_by_citizens − consumed_in_production − spoiled`,
and for money: `Σ wallets == issued`.

## 5. Systems (explicit order; SPEC §6)

- `ProductionSystem` (hour, `sim_economy`): per firm in entity order —
  finish a running batch (outputs +, `produced` +) or start one by
  consuming inputs (inventory −, `consumed_in_production` +). Batch
  duration in hours from the recipe.
- `TradeSystem` (day, `sim_economy`): firm-to-firm procurement — each
  buyer in entity order buys missing recipe inputs from the cheapest
  posted seller with stock (ties: lowest entity index). Money and goods
  transfer atomically; both books updated.
- `PricingSystem` (day, `sim_economy`): SPEC §12 posted prices —
  cost-plus (input costs at current posted prices + data overhead)
  × markup, then the inventory controller (stock above target → cut,
  below → raise) with the per-day movement bound; all parameters in
  `data/balance/economy.ron`. Emits `PriceChanged`.
- `SpoilageSystem` (day, `sim_goods`): perishables lose
  `floor(qty × spoil_per_mille / 1000)` daily (exact; the explicit sink).
- `AuditSystem` (day, `debug_tools` — SPEC §12 "permanent", §13 auditors
  live in debug_tools): recomputes both identities from scratch; ANY
  drift is a typed error that halts the run (debug and release — a
  conserved-quantity violation is never ignorable).
- Purchases happen inside `sim_ai::ActSystem` when a purchase action's
  performance begins (below).

Day-rate order: audit runs FIRST (validates yesterday), then trade,
pricing, spoilage, then mortality; hour: production before plan/decay;
documented in the schedule builder.

## 6. Citizens buy: the AI meets the market

Retail firms are locations (`Location` + `RetailOffer` on the firm
entity). `RetailOffer` (shared): good, need index, satisfaction gain per
unit (per-million), eat/use duration ticks, current unit price (updated
by `PricingSystem`). `DecideSystem` generates a purchase candidate per
offer: `gain = min(deficit, gain_per_unit)`, cost adds
`price × marginal utility of wealth` (SPEC §11's money cost), where
`mu = mu_scale / (1 + wallet/half_wealth)` (both data); candidates with
no stock or an unaffordable price are skipped (checked at decide time).
`ActSystem` executes the purchase exactly once at performance start —
wallet transfer, inventory decrement, books revenue, counters, event —
then the citizen spends the data-defined eat ticks. If stock or cash
changed during travel, the action aborts cleanly to a fresh decision
(no partial transaction, ever).

## 7. Data files

`data/goods.ron` (id, spoil per-mille daily), `data/recipes.ron` (inputs,
outputs, batch_hours), `data/firms.ron` (kind id, count, recipe, initial
cash, initial inventory, optional retail block: location kind, need,
gain, eat ticks, initial price), `data/balance/economy.ron` (markup,
controller step and bounds, inventory targets in batches, overhead),
`ai.ron` gains the marginal-utility parameters, `locations.ron` gains
bakery/woodyard kinds and drops free hunger, `demographics.ron` gains the
citizen wealth seed range. Validation: recipes↔goods, firms↔recipes↔
location kinds↔needs, every good has a producer, retail goods produced by
their firm's recipe, exactly one firm kind per retail location kind.

## 8. Supply-shock testing (exit criterion)

"Prices respond to seeded supply shocks in the correct direction": the
shock is seeded through the modeled spoilage sink — the test destroys a
firm's stock via the same counters-consistent path spoilage uses (a
test-input intervention, like seeding wealth; conservation stays intact
and the auditor keeps passing). Assert: post-shock posted price rises
within the controller's bound over the following days versus the
unshocked baseline; the controller unit tests cover the glut direction.

## 8a. Death and the estate (amendment, same phase)

Mortality predates money by two phases; with wallets, despawning a
citizen would DESTROY their cash and break `Σ wallets == issued`. The
estate therefore passes on at death, atomically with the despawn: to the
lowest-indexed surviving household member (deterministic — member lists
are index-sorted), or, when the household empties, to a zero-seeded
escheat wallet on the ledger entity ("unclaimed with the town" — the
seam Phase 6's treasury absorbs). Nothing is destroyed; the auditor
keeps passing across deaths. Worlds migrated from pre-economy saves have
no wallets, so their deaths carry no estates — unchanged behavior.

## 8b. Post-review amendments (same phase, before the fixes landed)

The adversarial review confirmed four hardening decisions, recorded here
before the code changed:

- **Schedules derive from world content, not just citizens.** Firms,
  inventories, and the ledger evolve with zero citizens, so gating the
  economy systems on the citizen count made a post-extinction resume
  diverge from the uninterrupted run (SPEC §9). `WorldSpec` gains an
  `economy` flag; loads derive it from the presence of the ledger; town
  systems are scheduled when the world has citizens OR an economy (the
  citizen systems no-op honestly in an empty town).
- **Wallets are audited non-negative.** The Wallet invariant ("never
  negative in Phase 4") joins the daily audit alongside inventory
  non-negativity — a purchase-guard regression must halt the run, not
  slip past `Σ wallets == issued`.
- **Transaction legs never silently no-op.** The transfer/stock/counter
  helpers error (`InvariantViolation`) when a structurally required
  component or slot is missing, instead of skipping the leg and leaving
  the drift for the next day's audit to notice.
- **The wealth seed range is validated** (`0 ≤ min ≤ max`) like every
  sibling demographics range — an inverted or negative range is a
  startup error, never an underflow or a negative seeded wallet.
- **A recipe may not list the same input good twice.** The batch-start
  check tests each entry against the same stock independently, so a
  duplicated input could double-spend an inventory below zero; rejected
  at load like every other malformed cross-reference.

## 9. Save format v5

Registration grows: components `econ.wallet, goods.inventory,
econ.retail_offer, econ.firm, econ.firm_books, econ.counters,
econ.production`; events `econ.goods_purchased, econ.price_changed`.
FORMAT_VERSION 5, mechanical v4→v5 (append stores + extend event names),
chain from v1, goldens re-recorded with continuity proofs, new v5
fixture, pinned snapshot becomes `data_v5` (the balance data itself
changed this phase — old-fixture resume goldens re-record under it).

## Dependencies

No new external dependencies.

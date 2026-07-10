//! The snapshot protocol (Phase 10, ADR 0013 §2): an owned, read-only
//! view of one tick — the ONLY place the viewer touches `World`. Every
//! pane renders from this; CI proves the exit criterion by driving the
//! same structures the widgets draw.

use core_ecs::sim_interface::{
    BankBook, Employment, HousingBook, Location, LodTier, Needs, Position, Residence, RetailOffer,
    Sited, Tenancy, Tier, Wallet,
};
use core_ecs::{EcsError, Entity, World};
use core_types::calendar::TICKS_PER_DAY;
use data_defs::DataDefs;
use debug_tools::metrics::MetricsRegistry;
use sim_people::Identity;

/// One citizen's snapshot row (the map dot + the inspector's spine).
#[derive(Debug, Clone)]
pub struct CitizenRow {
    /// Entity index (the inspector's id).
    pub index: u32,
    /// Display name.
    pub name: String,
    /// Where they stand (location entity index), if positioned.
    pub at: Option<u32>,
    /// Their LOD tier (missing row = embodied, the pre-v9 default).
    pub tier: Tier,
    /// Need levels, data order (per-million).
    pub needs: Vec<i64>,
    /// Pocket cash, mills.
    pub cash_mills: i64,
    /// Vault balance, mills (0 when unbanked).
    pub deposit_mills: i64,
    /// Employer's entity index, if employed.
    pub employer: Option<u32>,
    /// Home entity index, if housed.
    pub home: Option<u32>,
    /// Daily rent, mills, when renting.
    pub rent_mills: Option<i64>,
    /// The current action, rendered (None = deciding this tick).
    pub action: Option<String>,
    /// The compiled plan's sleep window (minutes of day), when planned.
    pub sleep_window: Option<(u16, u16)>,
    /// Skill mastery per-mille, data order.
    pub skills: Vec<u16>,
    /// Social bonds: `(other index, kind, strength per-mille)`.
    pub bonds: Vec<(u32, String, i32)>,
    /// Believed shop prices `(shop index, mills)`.
    pub believed: Vec<(u32, i64)>,
    /// The last decision: `(tick, scored candidates as (label, score
    /// micro))` — SPEC §13's "last decision's scored candidates".
    pub last_decision: Option<(u64, Vec<(String, i64)>)>,
}

/// One location's snapshot row (the map box).
#[derive(Debug, Clone)]
pub struct LocationRow {
    /// Entity index.
    pub index: u32,
    /// Kind id (data order string, e.g. `"tavern"`).
    pub kind: String,
    /// District index, when sited (pre-v10 worlds are un-sited).
    pub district: Option<u32>,
    /// Posted unit price, mills, when the location retails.
    pub price_mills: Option<i64>,
    /// Retail stock units, when the location retails.
    pub stock: Option<i64>,
}

/// One decoded event row for the browser (subjects + a readable line).
#[derive(Debug, Clone)]
pub struct EventRow {
    /// The tick it was logged.
    pub tick: u64,
    /// The registered event name (the browser's type filter).
    pub kind: String,
    /// Entity indices this event is about (the entity filter).
    pub subjects: Vec<u32>,
    /// A one-line human description.
    pub text: String,
}

/// The whole read-only view of one tick.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// The captured tick.
    pub tick: u64,
    /// The captured day (tick / ticks-per-day).
    pub day: u64,
    /// District display names, data order.
    pub districts: Vec<String>,
    /// Need display names, data order (the Need overlay's selector).
    pub need_names: Vec<String>,
    /// Every citizen, entity order.
    pub citizens: Vec<CitizenRow>,
    /// Every location, entity order.
    pub locations: Vec<LocationRow>,
    /// The retained event window, oldest first.
    pub events: Vec<EventRow>,
    /// The narrative composer's lines over the same window, each with
    /// its subject entity indices (the inspector filters by INDEX).
    pub stories: Vec<(Vec<u32>, String)>,
    /// Per-district rent asks (mills/day), when the world is mapped.
    pub district_rent_asks: Vec<i64>,
    /// Lifetime `(income tax, sales tax)` receipts, mills (the
    /// dashboard's treasury summary).
    pub treasury_receipts_mills: (i64, i64),
    /// The metrics registry's day-sampled series, display order — the
    /// dashboard's polylines travel INSIDE the snapshot (ADR 0013 §2).
    pub metrics: Vec<(String, Vec<(u64, i64)>)>,
    /// The last tick's `(system, elapsed micros)` rows (empty unless
    /// the schedule collects timings).
    pub timings: Vec<(String, u64)>,
}

impl Snapshot {
    /// Captures the world at `tick`. One pass per store; no `World`
    /// reference escapes. `metrics` is the sim thread's registry —
    /// its series ride along so the dashboard renders from the same
    /// snapshot as every other pane (ADR 0013 §2).
    pub fn capture(
        world: &World,
        tick: u64,
        defs: &DataDefs,
        timings: &[(&'static str, u64)],
        metrics: &MetricsRegistry,
    ) -> Result<Snapshot, EcsError> {
        let deposits: std::collections::BTreeMap<u32, i64> = world
            .iter::<BankBook>()?
            .next()
            .map(|(_, book)| {
                book.deposits
                    .iter()
                    .map(|(owner, balance)| (owner.index(), balance.mills()))
                    .collect()
            })
            .unwrap_or_default();

        let mut citizens = Vec::new();
        for (entity, identity) in world.iter::<Identity>()? {
            citizens.push(citizen_row(world, entity, identity, &deposits)?);
        }

        let mut locations = Vec::new();
        for (entity, location) in world.iter::<Location>()? {
            locations.push(location_row(world, entity, location, defs)?);
        }

        let events = crate::events::decode_window(world);
        let stories = debug_tools::stories_with_subjects(world)?;
        let district_rent_asks = world
            .iter::<HousingBook>()?
            .next()
            .map(|(_, book)| book.district_rent_ask_mills.clone())
            .unwrap_or_default();
        let treasury_receipts_mills = world
            .iter::<core_ecs::sim_interface::TreasuryBook>()?
            .next()
            .map(|(_, book)| {
                (
                    book.income_tax_received.mills(),
                    book.sales_tax_received.mills(),
                )
            })
            .unwrap_or((0, 0));

        Ok(Snapshot {
            tick,
            day: tick / TICKS_PER_DAY,
            districts: defs
                .map
                .districts
                .iter()
                .map(|district| district.id.clone())
                .collect(),
            need_names: defs
                .people
                .needs
                .needs
                .iter()
                .map(|need| need.id.clone())
                .collect(),
            citizens,
            locations,
            events,
            stories,
            district_rent_asks,
            treasury_receipts_mills,
            metrics: metrics
                .series()
                .iter()
                .map(|series| (series.name.to_owned(), series.samples.clone()))
                .collect(),
            timings: timings
                .iter()
                .map(|(name, micros)| ((*name).to_owned(), *micros))
                .collect(),
        })
    }
}

/// One citizen's row: every component the inspector shows, one `get`
/// each (SPEC §13's state list).
fn citizen_row(
    world: &World,
    entity: Entity,
    identity: &Identity,
    deposits: &std::collections::BTreeMap<u32, i64>,
) -> Result<CitizenRow, EcsError> {
    let index = entity.index();
    Ok(CitizenRow {
        index,
        name: format!("{} {}", identity.given_name, identity.family_name),
        at: world.get::<Position>(entity)?.map(|p| p.at.index()),
        tier: world
            .get::<LodTier>(entity)?
            .map(|row| row.tier)
            .unwrap_or(Tier::A),
        needs: world
            .get::<Needs>(entity)?
            .map(|needs| needs.levels.iter().map(|level| level.raw()).collect())
            .unwrap_or_default(),
        cash_mills: world
            .get::<Wallet>(entity)?
            .map(|wallet| wallet.cash.mills())
            .unwrap_or(0),
        deposit_mills: deposits.get(&index).copied().unwrap_or(0),
        employer: world
            .get::<Employment>(entity)?
            .map(|employment| employment.employer.index()),
        home: world.get::<Residence>(entity)?.map(|r| r.home.index()),
        rent_mills: world
            .get::<Tenancy>(entity)?
            .map(|tenancy| tenancy.rent_per_day.mills()),
        action: world
            .get::<sim_ai::CurrentAction>(entity)?
            .map(|action| format!("{action:?}")),
        sleep_window: world
            .get::<sim_ai::DailyPlan>(entity)?
            .map(|plan| (plan.sleep_start_minute, plan.sleep_end_minute)),
        skills: world
            .get::<core_ecs::sim_interface::Skills>(entity)?
            .map(|skills| skills.levels.clone())
            .unwrap_or_default(),
        bonds: world
            .get::<core_ecs::sim_interface::Relationships>(entity)?
            .map(|relationships| {
                relationships
                    .edges
                    .iter()
                    .map(|edge| {
                        (
                            edge.other.index(),
                            format!("{:?}", edge.kind),
                            edge.strength_per_mille,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        believed: world
            .get::<core_ecs::sim_interface::Beliefs>(entity)?
            .map(|beliefs| {
                beliefs
                    .prices
                    .iter()
                    .map(|(shop, price)| (shop.index(), price.mills()))
                    .collect()
            })
            .unwrap_or_default(),
        last_decision: world.get::<sim_ai::LastDecision>(entity)?.map(|decision| {
            (
                decision.tick.raw(),
                decision
                    .candidates
                    .iter()
                    .map(|candidate| (format!("{:?}", candidate.action), candidate.score_micro))
                    .collect(),
            )
        }),
    })
}

/// One location's row (the map box + the location inspector).
fn location_row(
    world: &World,
    entity: Entity,
    location: &Location,
    defs: &DataDefs,
) -> Result<LocationRow, EcsError> {
    let offer = world.get::<RetailOffer>(entity)?;
    Ok(LocationRow {
        index: entity.index(),
        kind: defs
            .locations
            .kinds
            .get(location.kind as usize)
            .map(|kind| kind.id.clone())
            .unwrap_or_else(|| format!("kind #{}", location.kind)),
        district: world.get::<Sited>(entity)?.map(|sited| sited.district),
        price_mills: offer.map(|offer| offer.unit_price.mills()),
        stock: match offer.map(|offer| offer.good) {
            Some(good) => world
                .get::<core_ecs::sim_interface::Inventory>(entity)?
                .map(|inventory| inventory.stock(good)),
            None => None,
        },
    })
}

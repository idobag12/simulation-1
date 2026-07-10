//! Phase 9 exit-criteria suite (SPEC §15): location visibly stratifies
//! rents and shop revenues — emergent from the travel matrix through
//! two channels (per-district vacancy-steered asks; access-weighted
//! assortative matching), never assigned. The town is data-shaped for
//! rental churn (the fast-generation pattern: an 8-day year turns
//! nest-leaving cohorts into yearly renter waves — all legal data).

use core_ecs::sim_interface::{FirmBooks, HousingBook, RetailOffer, Sited, Tenancy};
use core_types::Seed;
use embervale_tests::pinned_defs;
use headless::runner::{self, WorldSpec};

const TICKS_PER_DAY: u64 = 1440;

/// The shipped map's poles: `old_town` (0) is the CENTER — cheap to
/// reach from everywhere — and `outfields` (2) the EDGE.
const CENTER: u32 = 0;
const EDGE: u32 = 2;

#[test]
fn location_stratifies_rents_and_shop_revenues() {
    let mut defs = pinned_defs();
    defs.calendar.days_per_season = 2; // 8-day years: yearly renter waves
    // Nobody sparks (legal data: a zeroed sociability trait), so
    // nest-leavers stay single RENTERS instead of vanishing into
    // in-law households — thirty fast years accumulate renter cohorts.
    for definition in &mut defs.people.traits.traits {
        if definition.id == defs.social.spark_trait_id {
            definition.min = 0;
            definition.max = 0;
        }
    }
    defs.people.mortality.bands.clear();
    defs.people.mortality.terminal_per_day_chance_per_billion = 600_000;
    // A LAND-CONSTRAINED town (legal data): no builder firm, so the
    // genesis stock is all the housing there will ever be — location
    // premia survive only where supply cannot chase demand. Small
    // genesis households spread that stock (and deaths open slots).
    defs.firms.kinds.retain(|kind| kind.id != "builder");
    defs.people.demographics.household_min = 1;
    defs.people.demographics.household_max = 2;

    let (mut sim, mut schedule) =
        runner::build_simulation(&WorldSpec::town(Seed::new(251), 250), &defs).expect("build");
    // Eight warm-up years: genesis starts fully-housed, so the ask
    // controllers ratchet off a zero-vacancy cold start that says
    // nothing about location. Then pool NEW tenancies (yearly diffs of
    // the live set) — the FLOW price at formation, not the stock (a
    // long-lived early tenancy would otherwise resample its stale rent
    // every year).
    sim.run_ticks(&mut schedule, 8 * 8 * TICKS_PER_DAY)
        .expect("a halted run means a daily audit failed");
    let mut rent_count = [0i64; 3];
    let mut seen: std::collections::BTreeSet<(u32, u32, i64)> = std::collections::BTreeSet::new();
    for _ in 0..192 {
        // Two-day strides: short tenancies must not slip between
        // samples (a formation IS the datum).
        sim.run_ticks(&mut schedule, 2 * TICKS_PER_DAY)
            .expect("a halted run means a daily audit failed");
        let world = sim.world();
        for (tenant, tenancy) in world.iter::<Tenancy>().expect("query") {
            let key = (
                tenant.index(),
                tenancy.home.index(),
                tenancy.rent_per_day.mills(),
            );
            if !seen.insert(key) {
                continue; // the same agreement, resampled
            }
            if let Some(sited) = world.get::<Sited>(tenancy.home).expect("query") {
                rent_count[sited.district as usize] += 1;
            }
        }
    }
    let world = sim.world();

    // --- Rents (lifecycle half): this run proves the market's
    // LIVENESS — real tenancies keep forming under the daily audit and
    // the per-district ask controllers are live state. The
    // cross-district PRICE criterion is carried by the controlled
    // clearing below: a lifecycle town's rental flow concentrates
    // where renters already live (edge owner-families rarely vacate on
    // any humane horizon), so one seed cannot sample edge formations.
    assert!(
        rent_count.iter().sum::<i64>() >= 10 && rent_count[CENTER as usize] >= 5,
        "the market genuinely clears through the run \
         (formation samples {rent_count:?})"
    );
    let asks = world
        .iter::<HousingBook>()
        .expect("query")
        .next()
        .map(|(_, book)| book.district_rent_ask_mills.clone())
        .expect("the town keeps a housing ledger");
    assert_eq!(asks.len(), 3, "one outcome-steered ask per district");

    // --- Shop revenues: lifetime takings by the shop's district ------
    let mut revenue_sum = [0i64; 3];
    let mut revenue_count = [0i64; 3];
    for (shop, _) in world.iter::<RetailOffer>().expect("query") {
        if let (Some(sited), Some(books)) = (
            world.get::<Sited>(shop).expect("query"),
            world.get::<FirmBooks>(shop).expect("query"),
        ) {
            revenue_sum[sited.district as usize] += books.revenue.mills();
            revenue_count[sited.district as usize] += 1;
        }
    }
    assert!(
        revenue_count[CENTER as usize] >= 1 && revenue_count[EDGE as usize] >= 1,
        "shops exist at both poles ({revenue_count:?})"
    );
    let mean_revenue =
        |district: u32| revenue_sum[district as usize] / revenue_count[district as usize];
    assert!(
        mean_revenue(CENTER) > mean_revenue(EDGE),
        "location stratifies shop revenues: center {} vs edge {} mills \
         (counts {revenue_count:?})",
        mean_revenue(CENTER),
        mean_revenue(EDGE),
    );
}

/// The cross-district rent criterion, as a CONTROLLED CLEARING over the
/// real machinery (the lifecycle town's rental flow cannot sample the
/// edge — see above): six identical vacant homes, two per district; six
/// bidders of descending wealth, all working in the center. One real
/// clearing must (a) sort assortatively — the deepest pockets take the
/// best access — and (b) price every agreement by the tenant's
/// location-adjusted willingness, so center rents clear ABOVE edge
/// rents from the same bidder pool.
#[test]
fn a_controlled_clearing_prices_the_center_above_the_edge() {
    use core_ecs::sim_interface::NeedLevel;
    use core_ecs::sim_interface::{
        Employment, HousingBook, Location, Needs, Ownership, Residence, Sited, TenancyStarted,
        Wallet,
    };
    use core_types::{CalendarTime, Money, Ticks};

    let defs = pinned_defs();
    let tables = data_defs::resolve_economy(&defs);
    let home_kind = tables.money.home_location_kind;

    let mut world = core_ecs::World::new(Seed::new(263), 64);
    world.register::<Needs>().expect("register");
    world.register::<Wallet>().expect("register");
    world.register::<Employment>().expect("register");
    world.register::<Residence>().expect("register");
    world
        .register::<core_ecs::sim_interface::Tenancy>()
        .expect("register");
    world.register::<Location>().expect("register");
    world.register::<Sited>().expect("register");
    world.register::<Ownership>().expect("register");
    world.register::<HousingBook>().expect("register");
    world
        .register::<core_ecs::sim_interface::BankBook>()
        .expect("register");
    world.register_event::<TenancyStarted>().expect("register");

    let landlord = world.spawn();
    world
        .insert(landlord, Wallet { cash: Money::ZERO })
        .expect("insert");
    let ledger = world.spawn();
    world
        .insert(ledger, HousingBook::default())
        .expect("insert");
    // The central workplace every bidder commutes to.
    let workplace = world.spawn();
    world
        .insert(workplace, Sited { district: CENTER })
        .expect("insert");

    let mut homes_by_district: Vec<Vec<core_ecs::Entity>> = vec![Vec::new(); 3];
    for district in 0..3u32 {
        for _ in 0..2 {
            let home = world.spawn();
            world
                .insert(home, Location { kind: home_kind })
                .expect("insert");
            world.insert(home, Sited { district }).expect("insert");
            world
                .insert(home, Ownership { owner: landlord })
                .expect("insert");
            homes_by_district[district as usize].push(home);
        }
    }
    let mut bidders = Vec::new();
    for savings in [25_000i64, 20_000, 16_000, 13_000, 11_000, 10_000] {
        let citizen = world.spawn();
        world
            .insert(
                citizen,
                Needs {
                    levels: vec![NeedLevel::MAX],
                },
            )
            .expect("insert");
        world
            .insert(
                citizen,
                Wallet {
                    cash: Money::from_mills(savings),
                },
            )
            .expect("insert");
        world
            .insert(
                citizen,
                Employment {
                    employer: workplace,
                    wage_per_day: Money::from_mills(200),
                },
            )
            .expect("insert");
        bidders.push(citizen);
    }

    let ctx = core_ecs::TickContext {
        tick: Ticks::new(0),
        time: CalendarTime::START,
    };
    let mut market = sim_economy::RentalMarketSystem::new(tables);
    core_ecs::System::run(
        &mut market,
        &mut world,
        &ctx,
        &mut core_ecs::CommandBuffer::new(),
    )
    .expect("clearing");

    let rent_of = |world: &core_ecs::World, home: core_ecs::Entity| -> Option<i64> {
        world
            .iter::<core_ecs::sim_interface::Tenancy>()
            .expect("query")
            .find(|(_, tenancy)| tenancy.home == home)
            .map(|(_, tenancy)| tenancy.rent_per_day.mills())
    };
    let mean = |homes: &[core_ecs::Entity]| -> i64 {
        let rents: Vec<i64> = homes
            .iter()
            .filter_map(|home| rent_of(&world, *home))
            .collect();
        assert_eq!(rents.len(), 2, "both homes rented");
        rents.iter().sum::<i64>() / rents.len() as i64
    };
    let center = mean(&homes_by_district[CENTER as usize]);
    let edge = mean(&homes_by_district[EDGE as usize]);
    assert!(
        center > edge,
        "one clearing, one bidder pool: center rents {center} vs edge \
         {edge} mills/day"
    );
    // Assortative: every center tenant out-bids every edge tenant.
    let tenant_savings = |home: core_ecs::Entity| -> i64 {
        world
            .iter::<core_ecs::sim_interface::Tenancy>()
            .expect("query")
            .find(|(_, tenancy)| tenancy.home == home)
            .map(|(tenant, _)| {
                world
                    .get::<Wallet>(tenant)
                    .expect("query")
                    .expect("wallet")
                    .cash
                    .mills()
            })
            .expect("rented")
    };
    let min_center = homes_by_district[CENTER as usize]
        .iter()
        .map(|home| tenant_savings(*home))
        .min()
        .expect("rented");
    let max_edge = homes_by_district[EDGE as usize]
        .iter()
        .map(|home| tenant_savings(*home))
        .max()
        .expect("rented");
    assert!(
        min_center > max_edge,
        "the deepest pockets take the best access \
         (center min {min_center} vs edge max {max_edge})"
    );
}

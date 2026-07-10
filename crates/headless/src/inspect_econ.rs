//! The economy report (SPEC §13): firms, counters, the bank, the
//! treasury, and the audit verdict — split from `inspect.rs` for the
//! SPEC §3 module-size rule.

use data_defs::DataDefs;
use sim_time::Simulation;

use super::err;

/// Renders the town's economy: every firm's posted price, stock, cash,
/// and books; the conservation counters; and the live audit verdict
/// (SPEC §13 — the Phase 4 observables).
pub fn economy(sim: &Simulation, defs: &DataDefs) -> Result<String, String> {
    use core_ecs::sim_interface::{EconCounters, FirmBooks, Inventory, Wallet};

    let world = sim.world();
    let good_name = |good: usize| {
        defs.goods
            .goods
            .get(good)
            .map(|g| g.id.as_str())
            .unwrap_or("<unknown good>")
    };

    let mut out = format!("tick {}: economy\n", sim.tick());
    let mut firm_count = 0;
    for (entity, firm) in world.iter::<sim_economy::Firm>().map_err(err)? {
        firm_count += 1;
        let kind = defs
            .firms
            .kinds
            .get(firm.kind as usize)
            .map(|k| k.id.as_str())
            .unwrap_or("<unknown kind>");
        let output = defs
            .recipes
            .recipes
            .get(firm.recipe as usize)
            .map(|r| {
                r.output
                    .as_ref()
                    .map(|output| output.good_id.as_str())
                    .unwrap_or("homes")
            })
            .unwrap_or("<unknown>");
        out.push_str(&format!(
            "firm #{} {kind}: posts {output} at {}\n",
            entity.index(),
            firm.posted_price
        ));
        if let Some(wallet) = world.get::<Wallet>(entity).map_err(err)? {
            out.push_str(&format!("  cash {}", wallet.cash));
        }
        if let Some(books) = world.get::<FirmBooks>(entity).map_err(err)? {
            out.push_str(&format!(
                " | revenue {} expenses {}",
                books.revenue, books.expenses
            ));
        }
        out.push('\n');
        if let Some(inventory) = world.get::<Inventory>(entity).map_err(err)? {
            out.push_str("  stock:");
            for (good, quantity) in inventory.quantities.iter().enumerate() {
                if *quantity > 0 {
                    out.push_str(&format!(" {} {}", good_name(good), quantity));
                }
            }
            out.push('\n');
        }
    }
    if firm_count == 0 {
        out.push_str("no firms (pre-economy world)\n");
    }

    if let Some((_, counters)) = world.iter::<EconCounters>().map_err(err)?.next() {
        out.push_str(&format!("issued: {}\n", counters.issued));
        out.push_str("counters (produced/citizens/production/spoiled):\n");
        for good in 0..counters.produced.len() {
            out.push_str(&format!(
                "  {:<10} {} / {} / {} / {}\n",
                good_name(good),
                counters.produced.get(good).copied().unwrap_or(0),
                counters
                    .consumed_by_citizens
                    .get(good)
                    .copied()
                    .unwrap_or(0),
                counters
                    .consumed_in_production
                    .get(good)
                    .copied()
                    .unwrap_or(0),
                counters.spoiled.get(good).copied().unwrap_or(0),
            ));
        }
    }
    if let Some((bank, book)) = world
        .iter::<core_ecs::sim_interface::BankBook>()
        .map_err(err)?
        .next()
    {
        let vault = world
            .get::<core_ecs::sim_interface::Wallet>(bank)
            .map_err(err)?
            .map(|wallet| wallet.cash)
            .unwrap_or(core_types::Money::ZERO);
        let deposits: i64 = book
            .deposits
            .iter()
            .map(|(_, balance)| balance.mills())
            .sum();
        let outstanding: i64 = book.loans.iter().map(|loan| loan.principal.mills()).sum();
        out.push_str(&format!(
            "bank: vault {vault}, deposits {}, equity {}, outstanding {} across {} loans, \
             policy rate {}/million/day, interest {} in / {} out\n",
            core_types::Money::from_mills(deposits),
            book.equity,
            core_types::Money::from_mills(outstanding),
            book.loans.len(),
            book.policy_rate_per_million_daily,
            book.interest_received,
            book.deposit_interest_paid,
        ));
        // Per-loan rows (SPEC §13: every ledger inspectable from a save).
        for loan in &book.loans {
            out.push_str(&format!(
                "  loan: #{} owes {} at {}/million/day, {} per day{}\n",
                loan.borrower.index(),
                loan.principal,
                loan.rate_per_million_daily,
                loan.day_payment,
                match loan.collateral {
                    Some(home) => format!(", secured by home #{}", home.index()),
                    None => String::new(),
                },
            ));
        }
    }
    if let Some((_, book)) = world
        .iter::<core_ecs::sim_interface::HousingBook>()
        .map_err(err)?
        .next()
    {
        let tenancies = world
            .iter::<core_ecs::sim_interface::Tenancy>()
            .map_err(err)?
            .count();
        out.push_str(&format!(
            "housing: rent ask {}, last clearing price {}, {} tenancies\n",
            core_types::Money::from_mills(book.rent_ask_mills),
            core_types::Money::from_mills(book.last_home_price_mills),
            tenancies,
        ));
    }
    if let Some((treasury, book)) = world
        .iter::<core_ecs::sim_interface::TreasuryBook>()
        .map_err(err)?
        .next()
    {
        let cash = world
            .get::<core_ecs::sim_interface::Wallet>(treasury)
            .map_err(err)?
            .map(|wallet| wallet.cash)
            .unwrap_or(core_types::Money::ZERO);
        out.push_str(&format!(
            "treasury: {cash} on hand, income tax {} / sales tax {} collected\n",
            book.income_tax_received, book.sales_tax_received,
        ));
    }
    if let Some((_, stats)) = world
        .iter::<core_ecs::sim_interface::LaborStats>()
        .map_err(err)?
        .next()
    {
        out.push_str(&format!(
            "labor: {} working-age, {} employed, {} sought, {} unmatched \
             (lifetime {} hires / {} firings)\n",
            stats.working_age,
            stats.employed,
            stats.seeking,
            stats.unmatched,
            stats.hires,
            stats.firings,
        ));
    }
    match debug_tools::audit_economy(world) {
        Ok(true) => out.push_str("audit: PASS\n"),
        Ok(false) => out.push_str("audit: no economy to audit\n"),
        Err(e) => out.push_str(&format!("audit: FAIL — {e}\n")),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::{demography, inspect_entity};
    use crate::runner::{self, WorldSpec};
    use core_types::Seed;

    fn defs() -> DataDefs {
        let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data"));
        data_defs::load(root).expect("live data loads")
    }

    /// Content assertions for the inspector outputs (Phase 2 review):
    /// every section ADR 0005 §7 promises actually appears, with real
    /// values.
    #[test]
    fn inspector_outputs_contain_the_promised_sections() {
        let defs = defs();
        let spec = WorldSpec::town(Seed::new(51), 30);
        let (sim, _) = runner::build_simulation(&spec, &defs).expect("build");

        // Entity 0 is the first genesis household; its first member is a
        // citizen.
        let household_dump = inspect_entity(&sim, &defs, 0).expect("household dump");
        assert!(
            household_dump.starts_with("household #0:"),
            "{household_dump}"
        );

        let citizen_dump = inspect_entity(&sim, &defs, 1).expect("citizen dump");
        assert!(citizen_dump.starts_with("citizen #1:"), "{citizen_dump}");
        for section in [
            "age ", // "age {years}y {days}d" (ADR 0005 §7: years/days)
            "y ",
            "needs (per-million):",
            "hunger",
            "personality (per-mille):",
            "industriousness",
            "household: #0",
        ] {
            assert!(
                citizen_dump.contains(section),
                "missing `{section}` in:\n{citizen_dump}"
            );
        }

        let summary = demography(&sim, &defs).expect("demography");
        assert!(summary.contains("population 30"), "{summary}");
        assert!(summary.contains("age decades:"), "{summary}");
        assert!(summary.contains("household sizes:"), "{summary}");

        let missing = inspect_entity(&sim, &defs, 9_999);
        assert!(missing.is_err(), "dead index must error, got {missing:?}");
    }
}

//! The metrics registry (SPEC §13; Phase 10, ADR 0013 §5): named
//! world-state extractors sampled into bounded ring series — the
//! dashboard's data. A pull registry: it lives OUTSIDE the world
//! (observability, not state — saves ignore it, catch-up skips it,
//! determinism cannot see it).

use core_ecs::sim_interface::{
    BankBook, EconCounters, Employment, LodTier, Needs, Residence, RetailOffer, Tier, TreasuryBook,
};
use core_ecs::{EcsError, World};

/// One named series: day-sampled values in arrival order, bounded.
#[derive(Debug, Clone)]
pub struct Series {
    /// The metric's stable display name.
    pub name: &'static str,
    /// `(day, value)` samples, oldest first, capped at [`CAPACITY`].
    pub samples: Vec<(u64, i64)>,
}

/// Samples kept per series (a bounded window, like the event ring).
pub const CAPACITY: usize = 1024;

/// The registered extractors, in display order. Any crate can extend
/// this list in code (ADR 0013 §5 — the pull registry; a push API is
/// a documented deferral).
type Extractor = fn(&World) -> Result<i64, EcsError>;

const EXTRACTORS: &[(&str, Extractor)] = &[
    ("population", population),
    ("money supply (mills)", money_supply),
    ("employed", employed),
    ("homeless", homeless),
    ("mean posted price (mills)", mean_posted_price),
    ("tier A citizens", tier_a),
    ("tier C citizens", tier_c),
    ("treasury receipts (mills)", treasury_receipts),
    ("bank deposits (mills)", bank_deposits),
];

/// The dashboard's series set.
#[derive(Debug, Clone, Default)]
pub struct MetricsRegistry {
    series: Vec<Series>,
}

impl MetricsRegistry {
    /// An empty registry with every registered extractor's series.
    pub fn new() -> Self {
        MetricsRegistry {
            series: EXTRACTORS
                .iter()
                .map(|(name, _)| Series {
                    name,
                    samples: Vec::new(),
                })
                .collect(),
        }
    }

    /// Samples every registered metric once for `day`.
    pub fn sample(&mut self, world: &World, day: u64) -> Result<(), EcsError> {
        for (slot, (_, extract)) in self.series.iter_mut().zip(EXTRACTORS.iter()) {
            let value = extract(world)?;
            slot.samples.push((day, value));
            if slot.samples.len() > CAPACITY {
                slot.samples.remove(0);
            }
        }
        Ok(())
    }

    /// The series, display order.
    pub fn series(&self) -> &[Series] {
        &self.series
    }
}

fn population(world: &World) -> Result<i64, EcsError> {
    Ok(world.iter::<Needs>()?.count() as i64)
}

fn money_supply(world: &World) -> Result<i64, EcsError> {
    Ok(world
        .iter::<EconCounters>()?
        .next()
        .map(|(_, counters)| counters.issued.mills())
        .unwrap_or(0))
}

fn employed(world: &World) -> Result<i64, EcsError> {
    Ok(world.iter::<Employment>()?.count() as i64)
}

fn homeless(world: &World) -> Result<i64, EcsError> {
    let mut count = 0i64;
    for (citizen, _) in world.iter::<Needs>()? {
        if world.get::<Residence>(citizen)?.is_none() {
            count += 1;
        }
    }
    Ok(count)
}

fn mean_posted_price(world: &World) -> Result<i64, EcsError> {
    let mut sum = 0i64;
    let mut count = 0i64;
    for (_, offer) in world.iter::<RetailOffer>()? {
        sum += offer.unit_price.mills();
        count += 1;
    }
    Ok(if count > 0 { sum / count } else { 0 })
}

fn tier_a(world: &World) -> Result<i64, EcsError> {
    Ok(world
        .iter::<LodTier>()?
        .filter(|(_, row)| matches!(row.tier, Tier::A))
        .count() as i64)
}

fn tier_c(world: &World) -> Result<i64, EcsError> {
    Ok(world
        .iter::<LodTier>()?
        .filter(|(_, row)| matches!(row.tier, Tier::C))
        .count() as i64)
}

fn treasury_receipts(world: &World) -> Result<i64, EcsError> {
    Ok(world
        .iter::<TreasuryBook>()?
        .next()
        .map(|(_, book)| book.income_tax_received.mills() + book.sales_tax_received.mills())
        .unwrap_or(0))
}

fn bank_deposits(world: &World) -> Result<i64, EcsError> {
    Ok(world
        .iter::<BankBook>()?
        .next()
        .map(|(_, book)| book.deposits.iter().map(|(_, m)| m.mills()).sum())
        .unwrap_or(0))
}

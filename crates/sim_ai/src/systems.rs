//! The Tier A decision loop (ADR 0006 §§3–5): `PlanSystem` (hour rate)
//! compiles sleep windows, `DecideSystem` (tick rate) scores and picks
//! actions, `ActSystem` (tick rate, after decide) advances them with
//! exact integer effects.

use core_ecs::sim_interface::{
    Employment, Inventory, Location, NeedLevel, Needs, Personality, Position, Residence,
    RetailOffer, Wallet,
};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use core_types::Ticks;
use core_types::calendar::MINUTES_PER_HOUR;

use crate::components::{CandidateAction, CurrentAction, DailyPlan, LastDecision, ScoredCandidate};
use crate::config::AiTables;

// Scale constants. Unit definitions (per-million need scale, micro score
// scale), not tunables.
pub(crate) const NEED_MAX: i64 = NeedLevel::MAX.raw();
pub(crate) const MICRO: f64 = 1_000_000.0;

/// Everything scoring needs about one deciding citizen, snapshotted so
/// the scoring pass borrows nothing (two-pass pattern; SPEC §6 forbids
/// mid-iteration mutation).
pub(crate) struct Decider {
    pub(crate) entity: Entity,
    pub(crate) needs: Vec<i64>,
    pub(crate) traits: Vec<i16>,
    pub(crate) at: Option<Entity>,
    pub(crate) home: Option<Entity>,
    pub(crate) asleep_window: bool,
    pub(crate) cash_mills: i64,
    /// Wallet + vault balance (Phase 6, ADR 0009 §2): what wealth FEELS
    /// like in scoring; spendability stays `cash_mills`.
    pub(crate) wealth_mills: i64,
    /// The employer, when the citizen holds a job (Phase 5).
    pub(crate) workplace: Option<Entity>,
    /// The school, when the citizen is school-age (Phase 7).
    pub(crate) school: Option<Entity>,
    /// Believed shop prices `(seller index, mills)` — gossip and
    /// experience shape shop choice (Phase 7, ADR 0010 §3).
    pub(crate) believed: Vec<(u32, i64)>,
}

/// One retail offer snapshotted for scoring: the selling entity, the
/// offer, and its stock at decide time (rechecked atomically at purchase
/// time — a stale snapshot aborts cleanly, ADR 0007 §6).
pub(crate) struct OfferSnapshot {
    pub(crate) seller: Entity,
    pub(crate) offer: RetailOffer,
    pub(crate) stock: i64,
}

/// Tick-rate system: every citizen without a `CurrentAction` enumerates
/// candidates, scores them (floats confined here, SPEC §11), and commits
/// to the argmax — recording the full scored list as `LastDecision`.
pub struct DecideSystem {
    tables: AiTables,
}

impl DecideSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: AiTables) -> Self {
        DecideSystem { tables }
    }

    /// Enumerates and scores a decider's candidates; returns the dump and
    /// the chosen action. Enumeration order (= tie-break order, SPEC §11):
    /// own home (satisfier data order), public locations (entity order ×
    /// satisfier data order), purchases (retail-entity order), the work
    /// obligation (shift hours only), Idle last.
    fn decide(
        &self,
        decider: &Decider,
        publics: &[(Entity, u32)],
        offers: &[OfferSnapshot],
        home_kind: Option<u32>,
        work_window: bool,
        school_window: bool,
    ) -> (LastDecision, CurrentAction) {
        let mut candidates: Vec<ScoredCandidate> = Vec::new();
        let mut scores: Vec<f64> = Vec::new();

        let mut push = |location: Entity, kind: u32, decider: &Decider, this: &Self| {
            let Some(kind_satisfiers) = this.tables.kind_satisfiers.get(kind as usize) else {
                return;
            };
            for (need_index, rate) in kind_satisfiers {
                let level = decider
                    .needs
                    .get(*need_index as usize)
                    .copied()
                    .unwrap_or(NEED_MAX);
                let traveling = decider.at != Some(location);
                let own_home_rest =
                    Some(location) == decider.home && *need_index == this.tables.rest_need;
                let score = this.score(
                    level,
                    *rate,
                    traveling,
                    &decider.traits,
                    *need_index,
                    own_home_rest,
                    decider.asleep_window,
                );
                candidates.push(ScoredCandidate {
                    action: CandidateAction::Satisfy {
                        location,
                        need_index: *need_index,
                    },
                    score_micro: quantize(score),
                });
                scores.push(score);
            }
        };

        if let (Some(home), Some(kind)) = (decider.home, home_kind) {
            push(home, kind, decider, self);
        }
        for (location, kind) in publics {
            push(*location, *kind, decider, self);
        }
        for snapshot in offers {
            // Skipped at decide time (ADR 0007 §6): nothing on the shelf,
            // or the citizen cannot pay the posted price.
            if snapshot.stock < 1 || decider.cash_mills < snapshot.offer.unit_price.mills() {
                continue;
            }
            let score = self.score_buy(decider, snapshot);
            candidates.push(ScoredCandidate {
                action: CandidateAction::Buy {
                    location: snapshot.seller,
                    need_index: snapshot.offer.need_index,
                },
                score_micro: quantize(score),
            });
            scores.push(score);
        }
        if work_window && let Some(workplace) = decider.workplace {
            let score = self.score_work(decider, workplace);
            candidates.push(ScoredCandidate {
                action: CandidateAction::Work {
                    location: workplace,
                },
                score_micro: quantize(score),
            });
            scores.push(score);
        }
        if school_window && let Some(school) = decider.school {
            let score = self.score_school(decider, school);
            candidates.push(ScoredCandidate {
                action: CandidateAction::AttendSchool { location: school },
                score_micro: quantize(score),
            });
            scores.push(score);
        }
        candidates.push(ScoredCandidate {
            action: CandidateAction::Idle,
            score_micro: 0,
        });
        scores.push(0.0);

        // Argmax with first-wins tie-breaking (enumeration order).
        let mut chosen = 0usize;
        for (index, score) in scores.iter().enumerate() {
            if *score > scores[chosen] {
                chosen = index;
            }
        }

        let action = match candidates[chosen].action {
            CandidateAction::Satisfy {
                location,
                need_index,
            } => {
                if decider.at == Some(location) {
                    CurrentAction::Perform {
                        at: location,
                        need_index,
                        remaining: self.tables.max_perform_ticks,
                    }
                } else {
                    CurrentAction::Travel {
                        target: location,
                        need_index,
                        remaining: self.tables.travel_ticks,
                    }
                }
            }
            CandidateAction::Idle => CurrentAction::Idle {
                remaining: self.tables.idle_ticks,
            },
            CandidateAction::Buy { location, .. } => {
                if decider.at == Some(location) {
                    CurrentAction::BuyPending { at: location }
                } else {
                    CurrentAction::BuyTravel {
                        target: location,
                        remaining: self.tables.travel_ticks,
                    }
                }
            }
            CandidateAction::Work { location } => {
                if decider.at == Some(location) {
                    CurrentAction::Work {
                        at: location,
                        remaining: self.tables.work_ticks,
                    }
                } else {
                    CurrentAction::WorkTravel {
                        target: location,
                        remaining: self.tables.travel_ticks,
                    }
                }
            }
            CandidateAction::AttendSchool { location } => {
                if decider.at == Some(location) {
                    CurrentAction::Attend {
                        at: location,
                        remaining: self.tables.school_attend_ticks,
                    }
                } else {
                    CurrentAction::SchoolTravel {
                        target: location,
                        remaining: self.tables.travel_ticks,
                    }
                }
            }
        };
        (
            LastDecision {
                tick: Ticks::ZERO, // overwritten by caller with ctx.tick
                chosen: chosen as u32,
                candidates,
            },
            action,
        )
    }
}

/// Quantizes a score to micro units for storage (floats never persisted,
/// SPEC §2). `f64::round` and `as` saturation are both deterministic.
fn quantize(score: f64) -> i64 {
    if score == f64::MIN {
        i64::MIN
    } else {
        (score * MICRO).round() as i64
    }
}

impl System for DecideSystem {
    fn name(&self) -> &'static str {
        "ai.decide"
    }

    fn run(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let minute_of_day =
            u16::from(ctx.time.hour) * (MINUTES_PER_HOUR as u16) + u16::from(ctx.time.minute);
        let home_kind = self
            .tables
            .kind_is_home
            .iter()
            .position(|is_home| *is_home)
            .map(|index| index as u32);

        // Snapshot public locations (entity order).
        let publics: Vec<(Entity, u32)> = world
            .iter::<Location>()?
            .filter(|(_, location)| {
                !self
                    .tables
                    .kind_is_home
                    .get(location.kind as usize)
                    .copied()
                    .unwrap_or(false)
            })
            .map(|(entity, location)| (entity, location.kind))
            .collect();

        // Snapshot retail offers with their current stock (entity order).
        let offers: Vec<OfferSnapshot> = {
            let mut list = Vec::new();
            for (seller, offer) in world.iter::<RetailOffer>()? {
                let stock = world
                    .get::<Inventory>(seller)?
                    .map(|inventory| inventory.stock(offer.good))
                    .unwrap_or(0);
                list.push(OfferSnapshot {
                    seller,
                    offer: *offer,
                    stock,
                });
            }
            list
        };

        // The school (Phase 7): the first location of the school kind.
        let school_entity: Option<Entity> = world
            .iter::<Location>()?
            .find(|(_, location)| location.kind == self.tables.school_location_kind)
            .map(|(entity, _)| entity);

        // Deposit balances (Phase 6): wealth = wallet + vault row.
        let mut vault: std::collections::BTreeMap<u32, i64> = std::collections::BTreeMap::new();
        if let Some((_, book)) = world.iter::<core_ecs::sim_interface::BankBook>()?.next() {
            for (owner, balance) in &book.deposits {
                vault.insert(owner.index(), balance.mills());
            }
        }

        // Pass 1 (immutable): snapshot citizens that need a decision.
        // Only the embodied tier decides per tick (Phase 8, ADR 0011
        // §2; a missing tier row means Tier A — migrations never
        // invent state).
        let mut deciders: Vec<Decider> = Vec::new();
        for (entity, needs) in world.iter::<Needs>()? {
            if world.get::<CurrentAction>(entity)?.is_some() {
                continue;
            }
            if matches!(
                world
                    .get::<core_ecs::sim_interface::LodTier>(entity)?
                    .map(|row| row.tier),
                Some(core_ecs::sim_interface::Tier::B | core_ecs::sim_interface::Tier::C)
            ) {
                continue;
            }
            let traits = world
                .get::<Personality>(entity)?
                .map(|p| p.weights.clone())
                .unwrap_or_default();
            let asleep_window = world
                .get::<DailyPlan>(entity)?
                .is_some_and(|plan| plan.in_sleep_window(minute_of_day));
            deciders.push(Decider {
                entity,
                needs: needs.levels.iter().map(|l| l.raw()).collect(),
                traits,
                at: world.get::<Position>(entity)?.map(|p| p.at),
                home: world.get::<Residence>(entity)?.map(|r| r.home),
                asleep_window,
                cash_mills: world
                    .get::<Wallet>(entity)?
                    .map(|wallet| wallet.cash.mills())
                    .unwrap_or(0),
                wealth_mills: world
                    .get::<Wallet>(entity)?
                    .map(|wallet| wallet.cash.mills())
                    .unwrap_or(0)
                    + vault.get(&entity.index()).copied().unwrap_or(0),
                workplace: world
                    .get::<Employment>(entity)?
                    .map(|employment| employment.employer),
                school: match world.get::<core_ecs::sim_interface::SchoolAge>(entity)? {
                    Some(_) => school_entity,
                    None => None,
                },
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
            });
        }
        let work_window = minute_of_day >= self.tables.work_start_minute
            && minute_of_day < self.tables.work_end_minute;
        let school_window = minute_of_day >= self.tables.school_start_minute
            && minute_of_day < self.tables.school_end_minute;

        // Pass 2: decide and write (entity order preserved).
        for decider in deciders {
            let (mut dump, action) = self.decide(
                &decider,
                &publics,
                &offers,
                home_kind,
                work_window,
                school_window,
            );
            dump.tick = ctx.tick;
            world.insert(decider.entity, action)?;
            world.insert(decider.entity, dump)?;
        }
        Ok(())
    }
}

#[path = "systems_act.rs"]
mod act;
#[path = "systems_score.rs"]
mod score;
pub use act::ActSystem;
pub(crate) use act::purchase_unit;
#[path = "systems_plan.rs"]
mod plan;
pub use plan::PlanSystem;

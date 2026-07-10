//! The Phase 7 social systems (ADR 0010 §§2–3): co-presence bond drift
//! with the romance spark, gossip (price-belief exchange), and daily
//! edge decay. Split file for the SPEC §3 module-size rule.

use core_ecs::sim_interface::{
    Beliefs, Location, Needs, Personality, Position, RelKind, Relationships, WorkingAge,
};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};

use crate::config::AiTables;

/// Hour-rate system: at every leisure venue (a location kind satisfying
/// the data drift need), each present citizen meets the NEXT present
/// citizen (entity order — one meeting per citizen per hour, O(n) and
/// deterministic): friend edges drift up; when both are single working-
/// age adults whose spark-trait product clears the data screen, romance
/// drifts instead. The same meeting exchanges price beliefs (gossip —
/// integer midpoints, bounded rows).
pub struct SocialDriftSystem {
    tables: AiTables,
}

impl SocialDriftSystem {
    /// Builds from the resolved tables.
    pub fn new(tables: AiTables) -> Self {
        SocialDriftSystem { tables }
    }
}

fn is_single(world: &World, citizen: Entity) -> Result<bool, EcsError> {
    Ok(!world
        .get::<Relationships>(citizen)?
        .is_some_and(|relationships| {
            relationships
                .edges
                .iter()
                .any(|edge| matches!(edge.kind, RelKind::Spouse))
        }))
}

fn spark_weight(world: &World, citizen: Entity, trait_index: u32) -> Result<i64, EcsError> {
    Ok(world
        .get::<Personality>(citizen)?
        .and_then(|personality| personality.weights.get(trait_index as usize).copied())
        .map(i64::from)
        .unwrap_or(0))
}

impl System for SocialDriftSystem {
    fn name(&self) -> &'static str {
        "ai.social_drift"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let social = self.tables.social;
        // The leisure venues: location kinds satisfying the drift need.
        let mut venues: Vec<u32> = Vec::new();
        for (venue, location) in world.iter::<Location>()? {
            if self
                .tables
                .satisfier_rate(location.kind, social.drift_need)
                .is_some()
            {
                venues.push(venue.index());
            }
        }
        if venues.is_empty() {
            return Ok(());
        }
        // Present citizens per venue, entity order (Needs = the citizen
        // signal; Position = presence). Everyone meets their neighbor;
        // single working-age adults ADDITIONALLY meet the next single —
        // partner-seekers find each other even when married neighbors
        // sit between them (without this, the town's last singles never
        // pair and the generations stop).
        let mut meetings: Vec<(Entity, Entity)> = Vec::new();
        for venue_index in venues {
            let mut present: Vec<Entity> = Vec::new();
            for (citizen, _) in world.iter::<Needs>()? {
                if world.get::<Position>(citizen)?.map(|p| p.at.index()) == Some(venue_index) {
                    present.push(citizen);
                }
            }
            let mut met: std::collections::BTreeSet<(u32, u32)> = std::collections::BTreeSet::new();
            for pair in present.windows(2) {
                meetings.push((pair[0], pair[1]));
                met.insert((pair[0].index(), pair[1].index()));
            }
            let mut singles: Vec<Entity> = Vec::new();
            for citizen in &present {
                if is_single(world, *citizen)? && world.get::<WorkingAge>(*citizen)?.is_some() {
                    singles.push(*citizen);
                }
            }
            for pair in singles.windows(2) {
                // Only the pairs the general pass did not already meet
                // (deduped against actual present-adjacency, so no pair
                // drifts twice in one hour).
                if !met.contains(&(pair[0].index(), pair[1].index())) {
                    meetings.push((pair[0], pair[1]));
                }
            }
        }

        for (a, b) in meetings {
            // Romance or friendship?
            let kin_edge = |world: &World, from: Entity, to: Entity| -> Result<bool, EcsError> {
                Ok(world
                    .get::<Relationships>(from)?
                    .is_some_and(|relationships| {
                        relationships
                            .edges
                            .iter()
                            .any(|edge| edge.other == to && matches!(edge.kind, RelKind::Kin))
                    }))
            };
            let are_kin = kin_edge(world, a, b)? || kin_edge(world, b, a)?;
            let spark = !are_kin
                && is_single(world, a)?
                && is_single(world, b)?
                && world.get::<WorkingAge>(a)?.is_some()
                && world.get::<WorkingAge>(b)?.is_some()
                && spark_weight(world, a, social.spark_trait)?.saturating_mul(spark_weight(
                    world,
                    b,
                    social.spark_trait,
                )?) / 1000
                    >= i64::from(social.romance_min_sociability_product_per_mille);
            let (kind, drift) = if spark {
                (RelKind::Romance, social.romance_drift_per_meeting_per_mille)
            } else {
                (RelKind::Friend, social.friend_drift_per_meeting_per_mille)
            };
            for (this, other) in [(a, b), (b, a)] {
                let mut relationships = world
                    .get::<Relationships>(this)?
                    .cloned()
                    .unwrap_or_default();
                let strength = relationships.strength(other, kind).unwrap_or(0) + drift;
                relationships.upsert(other, kind, strength, social.edge_cap as usize);
                world.insert(this, relationships)?;
            }
            // Gossip: exchange price beliefs at the integer midpoint
            // (both walk away believing the average — knowledge spreads
            // through the graph, ADR 0010 §3).
            let beliefs_a = world.get::<Beliefs>(a)?.cloned().unwrap_or_default();
            let beliefs_b = world.get::<Beliefs>(b)?.cloned().unwrap_or_default();
            let mut merged_a = beliefs_a.clone();
            let mut merged_b = beliefs_b.clone();
            for (shop, price) in &beliefs_b.prices {
                merge_belief(&mut merged_a, *shop, *price, social.belief_cap as usize);
            }
            for (shop, price) in &beliefs_a.prices {
                merge_belief(&mut merged_b, *shop, *price, social.belief_cap as usize);
            }
            if merged_a != beliefs_a {
                world.insert(a, merged_a)?;
            }
            if merged_b != beliefs_b {
                world.insert(b, merged_b)?;
            }
        }
        Ok(())
    }
}

/// Folds one heard price into `beliefs`: an existing row moves to the
/// integer midpoint; a new row lands if the cap allows (rows in shop
/// entity-index order).
pub(crate) fn merge_belief(
    beliefs: &mut Beliefs,
    shop: Entity,
    heard: core_types::Money,
    cap: usize,
) {
    if let Some(row) = beliefs.prices.iter_mut().find(|(other, _)| *other == shop) {
        row.1 = core_types::Money::from_mills((row.1.mills() + heard.mills()) / 2);
    } else if beliefs.prices.len() < cap {
        beliefs.prices.push((shop, heard));
        beliefs.prices.sort_by_key(|(other, _)| other.index());
    }
}

/// Writes an EXPERIENCED price (a completed purchase) into the buyer's
/// beliefs: experience overwrites, it never averages (you saw the tag),
/// and it ALWAYS lands — at the cap the most-expensive believed shop is
/// forgotten first (bargains are worth remembering; the forgotten shop
/// falls back to its posted price, honestly). Only hearsay
/// ([`merge_belief`]) is dropped at the cap.
pub(crate) fn experience_price(
    beliefs: &mut Beliefs,
    shop: Entity,
    paid: core_types::Money,
    cap: usize,
) {
    if let Some(row) = beliefs.prices.iter_mut().find(|(other, _)| *other == shop) {
        row.1 = paid;
        return;
    }
    if beliefs.prices.len() >= cap {
        let Some(priciest) = beliefs
            .prices
            .iter()
            .enumerate()
            .max_by_key(|(_, (other, price))| (price.mills(), other.index()))
            .map(|(index, _)| index)
        else {
            return; // a zero cap holds no beliefs at all
        };
        beliefs.prices.remove(priciest);
    }
    beliefs.prices.push((shop, paid));
    beliefs.prices.sort_by_key(|(other, _)| other.index());
}

/// Day-rate system: every non-kin, non-spouse edge decays by the data
/// per-mille; zeroed edges drop (absence dissolves what presence built).
pub struct RelationshipDecaySystem {
    decay_per_day_per_mille: i32,
}

impl RelationshipDecaySystem {
    /// Builds from the data decay (`balance/social.ron`).
    pub fn new(decay_per_day_per_mille: i32) -> Self {
        RelationshipDecaySystem {
            decay_per_day_per_mille,
        }
    }
}

impl System for RelationshipDecaySystem {
    fn name(&self) -> &'static str {
        "ai.relationship_decay"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        let decay = self.decay_per_day_per_mille;
        // Tier C citizens do not visit venues, so their drift pauses —
        // and decay pauses with it (Phase 8, ADR 0011 §9): absence from
        // the EMBODIED world must not dissolve bonds the day model
        // cannot rebuild.
        let mut citizens: Vec<Entity> = Vec::new();
        for (citizen, _) in world.iter::<Relationships>()? {
            if matches!(
                world
                    .get::<core_ecs::sim_interface::LodTier>(citizen)?
                    .map(|row| row.tier),
                Some(core_ecs::sim_interface::Tier::C)
            ) {
                continue;
            }
            citizens.push(citizen);
        }
        for citizen in citizens {
            if let Some(relationships) = world.get_mut::<Relationships>(citizen)? {
                for edge in &mut relationships.edges {
                    if matches!(edge.kind, RelKind::Friend | RelKind::Romance) {
                        edge.strength_per_mille = (edge.strength_per_mille - decay).max(0);
                    }
                }
                relationships.edges.retain(|edge| {
                    edge.strength_per_mille > 0
                        || matches!(edge.kind, RelKind::Kin | RelKind::Spouse)
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::CurrentAction;
    use crate::config::SocialTables;
    use core_ecs::CommandBuffer;
    use core_ecs::sim_interface::NeedLevel;
    use core_types::{CalendarTime, Money, Seed, Ticks};

    /// Minimal resolved tables: one location kind (0) satisfying the
    /// drift need (0) at 7000/tick; the shipped social shape otherwise.
    fn tables() -> AiTables {
        AiTables {
            travel_ticks: 1,
            urgency_exponent: 2,
            time_cost_micro_per_tick: 0,
            max_perform_ticks: 10,
            idle_ticks: 1,
            plan_compile_hour: 0,
            sleep_start_minute: 0,
            sleep_end_minute: 0,
            sleep_max_shift_minutes: 0,
            sleep_shift_trait: 0,
            rest_need: 0,
            sleep_home_bias_micro: 0,
            kind_is_home: vec![false],
            kind_satisfiers: vec![vec![(0, 7000)]],
            need_trait: vec![None],
            mu_scale_micro: 0,
            half_wealth_mills: 1,
            work_start_minute: 0,
            work_end_minute: 0,
            work_bias_micro: 0,
            work_ticks: 1,
            work_need: 0,
            work_need_per_tick: 0,
            sales_tax_per_mille: 0,
            school_start_minute: 0,
            school_end_minute: 0,
            school_bias_micro: 0,
            school_attend_ticks: 1,
            school_location_kind: 0,
            school_taught_skill: 0,
            school_gain_per_mille: 0,
            social: SocialTables {
                edge_cap: 8,
                friend_drift_per_meeting_per_mille: 30,
                romance_drift_per_meeting_per_mille: 25,
                decay_per_day_per_mille: 5,
                romance_min_sociability_product_per_mille: 90,
                marriage_threshold_per_mille: 700,
                social_bond_weight_per_mille: 400,
                belief_cap: 8,
                drift_need: 0,
                spark_trait: 0,
            },
            skill_count: 1,
            district_travel: Vec::new(),
            commute_mills_per_tick: 0,
            lod: crate::config::LodTables {
                tier_a_cap: 1000,
                tier_b_cap: 1000,
                highlight_days: 1,
                leisure_hours_per_day: 4,
            },
        }
    }

    fn ctx() -> core_ecs::TickContext {
        core_ecs::TickContext {
            tick: Ticks::new(0),
            time: CalendarTime::START,
        }
    }

    fn social_world() -> World {
        let mut world = World::new(Seed::new(21), 64);
        world.register::<Location>().expect("register");
        world.register::<Needs>().expect("register");
        world.register::<Position>().expect("register");
        world.register::<WorkingAge>().expect("register");
        world.register::<Personality>().expect("register");
        world.register::<Relationships>().expect("register");
        world.register::<Beliefs>().expect("register");
        world
    }

    /// Spawns a citizen at `venue`; `single` adds the working-age marker
    /// and a full spark trait (the screen then passes on trait product).
    fn citizen(world: &mut World, venue: Entity, single: bool) -> Entity {
        let entity = world.spawn();
        world
            .insert(
                entity,
                Needs {
                    levels: vec![NeedLevel::new_clamped(0)],
                },
            )
            .expect("insert");
        world
            .insert(entity, Position { at: venue })
            .expect("insert");
        if single {
            world
                .insert(entity, core_ecs::sim_interface::WorkingAge)
                .expect("insert");
            world
                .insert(
                    entity,
                    Personality {
                        weights: vec![1000],
                    },
                )
                .expect("insert");
        }
        entity
    }

    /// The one-meeting-per-hour invariant: two ADJACENT singles meet in
    /// the general pass, and the singles pass must not meet them again —
    /// the romance drift lands exactly once (the data-tuned pacing, not
    /// double).
    #[test]
    fn adjacent_singles_meet_exactly_once_per_hour() {
        let mut world = social_world();
        let venue = world.spawn();
        world.insert(venue, Location { kind: 0 }).expect("insert");
        let s1 = citizen(&mut world, venue, true);
        let s2 = citizen(&mut world, venue, true);
        let mut system = SocialDriftSystem::new(tables());
        system
            .run(&mut world, &ctx(), &mut CommandBuffer::new())
            .expect("run");
        for (this, other) in [(s1, s2), (s2, s1)] {
            assert_eq!(
                world
                    .get::<Relationships>(this)
                    .expect("query")
                    .expect("edges")
                    .strength(other, RelKind::Romance),
                Some(25),
                "exactly one drift application per hour"
            );
        }
    }

    /// The singles pass (ADR 0010 §2, amended): singles separated by a
    /// married couple in the meeting order still find each other — once.
    #[test]
    fn separated_singles_still_meet_exactly_once() {
        let mut world = social_world();
        let venue = world.spawn();
        world.insert(venue, Location { kind: 0 }).expect("insert");
        let m1 = citizen(&mut world, venue, false);
        let s1 = citizen(&mut world, venue, true);
        let m2 = citizen(&mut world, venue, false);
        let s2 = citizen(&mut world, venue, true);
        // m1 and m2 are married (to each other): not single.
        for (this, other) in [(m1, m2), (m2, m1)] {
            let mut rel = Relationships::default();
            rel.upsert(other, RelKind::Spouse, 1000, 8);
            world.insert(this, rel).expect("insert");
        }
        let mut system = SocialDriftSystem::new(tables());
        system
            .run(&mut world, &ctx(), &mut CommandBuffer::new())
            .expect("run");
        let s1_rel = world
            .get::<Relationships>(s1)
            .expect("query")
            .expect("edges")
            .clone();
        assert_eq!(
            s1_rel.strength(s2, RelKind::Romance),
            Some(25),
            "the singles pass paired them exactly once"
        );
        assert_eq!(
            s1_rel.strength(m1, RelKind::Friend),
            Some(30),
            "the general pass still met the married neighbor"
        );
    }

    /// The kin bar (ADR 0010 §2, amended): kin singles drift FRIENDSHIP,
    /// never romance — even when only ONE side's list carries the kin
    /// edge (the screen checks both).
    #[test]
    fn kin_singles_drift_friendship_not_romance() {
        let mut world = social_world();
        let venue = world.spawn();
        world.insert(venue, Location { kind: 0 }).expect("insert");
        let k1 = citizen(&mut world, venue, true);
        let k2 = citizen(&mut world, venue, true);
        // One-sided on purpose: only k2's list knows they are family.
        let mut rel = Relationships::default();
        rel.upsert(k1, RelKind::Kin, 1000, 8);
        world.insert(k2, rel).expect("insert");
        let mut system = SocialDriftSystem::new(tables());
        system
            .run(&mut world, &ctx(), &mut CommandBuffer::new())
            .expect("run");
        let k1_rel = world
            .get::<Relationships>(k1)
            .expect("query")
            .expect("edges")
            .clone();
        assert_eq!(k1_rel.strength(k2, RelKind::Romance), None);
        assert_eq!(k1_rel.strength(k2, RelKind::Friend), Some(30));
    }

    /// Gossip merges at the integer midpoint; a new shop lands only
    /// under the cap (hearsay is droppable — ADR 0010 §3).
    #[test]
    fn gossip_merges_midpoints_and_respects_the_cap() {
        let mut world = World::new(Seed::new(23), 64);
        let shops: Vec<Entity> = (0..3).map(|_| world.spawn()).collect();
        let mut beliefs = Beliefs {
            prices: vec![(shops[0], Money::from_mills(100))],
        };
        merge_belief(&mut beliefs, shops[0], Money::from_mills(51), 2);
        assert_eq!(
            beliefs.prices[0].1,
            Money::from_mills(75),
            "integer midpoint"
        );
        merge_belief(&mut beliefs, shops[1], Money::from_mills(40), 2);
        assert_eq!(beliefs.prices.len(), 2, "under the cap: the row lands");
        merge_belief(&mut beliefs, shops[2], Money::from_mills(10), 2);
        assert_eq!(
            beliefs.prices.len(),
            2,
            "at the cap hearsay is dropped, never evicts"
        );
        assert!(!beliefs.prices.iter().any(|(shop, _)| *shop == shops[2]));
    }

    /// Experience overwrites (never averages) and ALWAYS lands: at the
    /// cap the most-expensive believed shop is forgotten first.
    #[test]
    fn experience_overwrites_and_always_lands() {
        let mut world = World::new(Seed::new(25), 64);
        let shops: Vec<Entity> = (0..3).map(|_| world.spawn()).collect();
        let mut beliefs = Beliefs::default();
        experience_price(&mut beliefs, shops[0], Money::from_mills(100), 2);
        experience_price(&mut beliefs, shops[0], Money::from_mills(60), 2);
        assert_eq!(
            beliefs.prices[0].1,
            Money::from_mills(60),
            "experience overwrites, it never averages"
        );
        experience_price(&mut beliefs, shops[1], Money::from_mills(30), 2);
        // At the cap: the priciest belief (shop 0 at 60) is forgotten so
        // the till's evidence lands.
        experience_price(&mut beliefs, shops[2], Money::from_mills(45), 2);
        assert_eq!(beliefs.prices.len(), 2);
        assert!(!beliefs.prices.iter().any(|(shop, _)| *shop == shops[0]));
        assert_eq!(
            beliefs
                .prices
                .iter()
                .find(|(shop, _)| *shop == shops[2])
                .map(|(_, price)| *price),
            Some(Money::from_mills(45))
        );
        // A zero cap holds no beliefs and must not panic.
        let mut none = Beliefs::default();
        experience_price(&mut none, shops[0], Money::from_mills(9), 0);
        assert!(none.prices.is_empty());
    }

    /// Bonds feed utility (ADR 0010 §2): satisfying the drift need among
    /// present friends gains more — the exact integer bonus, next to a
    /// friendless control in the same world.
    #[test]
    fn performing_among_friends_satisfies_more() {
        let mut world = social_world();
        world.register::<CurrentAction>().expect("register");
        let venue = world.spawn();
        world.insert(venue, Location { kind: 0 }).expect("insert");
        let befriended = citizen(&mut world, venue, false);
        let control = citizen(&mut world, venue, false);
        let friend = citizen(&mut world, venue, false);
        let mut rel = Relationships::default();
        rel.upsert(friend, RelKind::Friend, 1000, 8);
        world.insert(befriended, rel).expect("insert");
        for performer in [befriended, control] {
            world
                .insert(
                    performer,
                    CurrentAction::Perform {
                        at: venue,
                        need_index: 0,
                        remaining: 3,
                    },
                )
                .expect("insert");
        }
        let mut system = crate::systems::ActSystem::new(tables());
        core_ecs::System::run(&mut system, &mut world, &ctx(), &mut CommandBuffer::new())
            .expect("run");
        let level = |world: &World, who: Entity| {
            world
                .get::<Needs>(who)
                .expect("query")
                .expect("needs")
                .levels[0]
                .raw()
        };
        assert_eq!(level(&world, control), 7000, "the data rate, exactly");
        assert_eq!(
            level(&world, befriended),
            7000 + 7000 * 400 / 1000,
            "plus weight/1000 × strength/1000 of the rate per present friend"
        );
    }
}

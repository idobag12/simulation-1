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
            for pair in present.windows(2) {
                meetings.push((pair[0], pair[1]));
            }
            let mut singles: Vec<Entity> = Vec::new();
            for citizen in &present {
                if is_single(world, *citizen)? && world.get::<WorkingAge>(*citizen)?.is_some() {
                    singles.push(*citizen);
                }
            }
            for pair in singles.windows(2) {
                if pair[1].index() != pair[0].index() + 1 {
                    // Only the pairs the general pass did not already meet.
                    meetings.push((pair[0], pair[1]));
                }
            }
        }

        for (a, b) in meetings {
            // Romance or friendship?
            let are_kin = world.get::<Relationships>(a)?.is_some_and(|relationships| {
                relationships
                    .edges
                    .iter()
                    .any(|edge| edge.other == b && matches!(edge.kind, RelKind::Kin))
            });
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
/// beliefs: experience overwrites, it never averages (you saw the tag).
pub(crate) fn experience_price(
    beliefs: &mut Beliefs,
    shop: Entity,
    paid: core_types::Money,
    cap: usize,
) {
    if let Some(row) = beliefs.prices.iter_mut().find(|(other, _)| *other == shop) {
        row.1 = paid;
    } else if beliefs.prices.len() < cap {
        beliefs.prices.push((shop, paid));
        beliefs.prices.sort_by_key(|(other, _)| other.index());
    }
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
        let citizens: Vec<Entity> = world
            .iter::<Relationships>()?
            .map(|(citizen, _)| citizen)
            .collect();
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

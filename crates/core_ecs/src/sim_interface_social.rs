//! The social layer's shared components and events (Phase 7, ADR 0010):
//! skills, the relationship graph, price beliefs, and the lifecycle
//! facts. Split file for the SPEC §3 module-size rule.

use core_types::Money;
use serde::{Deserialize, Serialize};

use crate::entity::Entity;
use crate::store::{Component, StorageKind};

/// Per-skill mastery, per-mille (0 = untrained, 1000 = mastered), in
/// `data/skills.ron` order (ADR 0010 §1). Raised by school attendance
/// and by working; read by the labor market's bid multiplier. Migrated
/// pre-v8 citizens have no row — competence is never invented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skills {
    /// Mastery per skill, data order, each within `0..=1000`.
    pub levels: Vec<u16>,
}

impl Component for Skills {
    const NAME: &'static str = "people.skills";
    // Dense: every post-v8 citizen learns.
    const STORAGE: StorageKind = StorageKind::Dense;
}

/// One social bond (ADR 0010 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelKind {
    /// Parent/child/sibling — written by facts, never decayed.
    Kin,
    /// Married partner — written at the wedding, never decayed.
    Spouse,
    /// Grown by shared leisure, decayed by absence.
    Friend,
    /// The courtship edge; crossing the data threshold marries.
    Romance,
}

/// One edge of the sparse relationship graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    /// The other citizen.
    pub other: Entity,
    /// What kind of bond.
    pub kind: RelKind,
    /// Bond strength, per-mille (kin/spouse conventionally 1000).
    pub strength_per_mille: i32,
}

/// A citizen's bonds (ADR 0010 §2): sparse, data-capped (the weakest
/// non-kin edge evicts on overflow), kept in (other-index, kind) order.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Relationships {
    /// The edges, sorted by (other entity index, kind discriminant).
    pub edges: Vec<Edge>,
}

impl Component for Relationships {
    const NAME: &'static str = "social.relationships";
    // Sparse: edges exist only where life created them.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

impl RelKind {
    /// A stable ordering discriminant (the edge sort key's second term).
    pub const fn order(self) -> u8 {
        match self {
            RelKind::Kin => 0,
            RelKind::Spouse => 1,
            RelKind::Friend => 2,
            RelKind::Romance => 3,
        }
    }
}

impl Relationships {
    /// The strength of the `(other, kind)` edge, if present.
    pub fn strength(&self, other: Entity, kind: RelKind) -> Option<i32> {
        self.edges
            .iter()
            .find(|edge| edge.other == other && edge.kind as u8 == kind as u8)
            .map(|edge| edge.strength_per_mille)
    }

    /// Upserts an edge (clamped to `0..=1000`), keeping the vector in
    /// (other index, kind order) order. The CAP bounds Friend/Romance:
    /// when full, the weakest of them evicts (kin and spouses never
    /// do), and a new drift edge weaker than everything is simply not
    /// recorded. Kin and Spouse edges are PRIVILEGED — they always
    /// record (evicting the weakest drift edge, or exceeding the cap
    /// when only kin remain): a full heart never forgets family, and a
    /// one-sided kin graph would defeat the incest screen and the
    /// bigamy check.
    pub fn upsert(&mut self, other: Entity, kind: RelKind, strength_per_mille: i32, cap: usize) {
        let strength = strength_per_mille.clamp(0, 1000);
        if let Some(edge) = self
            .edges
            .iter_mut()
            .find(|edge| edge.other == other && edge.kind as u8 == kind as u8)
        {
            edge.strength_per_mille = strength;
            return;
        }
        if self.edges.len() >= cap {
            let weakest = self
                .edges
                .iter()
                .enumerate()
                .filter(|(_, edge)| matches!(edge.kind, RelKind::Friend | RelKind::Romance))
                .min_by_key(|(_, edge)| (edge.strength_per_mille, edge.other.index()))
                .map(|(index, edge)| (index, edge.strength_per_mille));
            let privileged = matches!(kind, RelKind::Kin | RelKind::Spouse);
            match weakest {
                Some((index, _)) if privileged => {
                    self.edges.remove(index);
                }
                Some((index, weakest_strength)) if weakest_strength < strength => {
                    self.edges.remove(index);
                }
                _ if privileged => {} // only family left: exceed the cap
                _ => return,          // full of stronger bonds: the drift edge is not recorded
            }
        }
        self.edges.push(Edge {
            other,
            kind,
            strength_per_mille: strength,
        });
        self.edges
            .sort_by_key(|edge| (edge.other.index(), edge.kind.order()));
    }

    /// Removes the `(other, kind)` edge if present.
    pub fn remove(&mut self, other: Entity, kind: RelKind) {
        self.edges
            .retain(|edge| !(edge.other == other && edge.kind as u8 == kind as u8));
    }
}

/// Believed retail prices (ADR 0010 §3): what this citizen THINKS each
/// shop charges — written by purchases, exchanged by gossip, read by
/// purchase scoring. Bounded by data; rows in shop entity-index order.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Beliefs {
    /// `(shop entity, believed unit price)`, shop-index order.
    pub prices: Vec<(Entity, Money)>,
}

impl Component for Beliefs {
    const NAME: &'static str = "social.beliefs";
    // Sparse: beliefs exist only where experience wrote them.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

/// Marks a citizen inside the data school ages (Phase 7, ADR 0010 §1):
/// maintained by `sim_people` on birthdays (age stays in `Identity`),
/// read by the AI's AttendSchool candidate — the same split as
/// `WorkingAge` (SPEC §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchoolAge;

impl Component for SchoolAge {
    const NAME: &'static str = "people.school_age";
    // Sparse: a small subset of citizens.
    const STORAGE: StorageKind = StorageKind::Sparse;
}

/// Two citizens married (a fact; ADR 0010 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Married {
    /// One partner (lower entity index).
    pub partner_a: Entity,
    /// The other.
    pub partner_b: Entity,
}

impl crate::Event for Married {
    const NAME: &'static str = "social.married";
}

/// A child was born (a fact; ADR 0010 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Born {
    /// The newborn.
    pub child: Entity,
    /// One parent (lower entity index).
    pub parent_a: Entity,
    /// The other.
    pub parent_b: Entity,
}

impl crate::Event for Born {
    const NAME: &'static str = "people.born";
}

/// A school day was attended (the skill-mobility measurement hook).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchoolAttended {
    /// The pupil.
    pub pupil: Entity,
    /// The skill raised (data order index).
    pub skill: u32,
    /// The pupil's new level, per-mille.
    pub new_level: u16,
}

impl crate::Event for SchoolAttended {
    const NAME: &'static str = "people.school_attended";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World;
    use core_types::Seed;

    fn entities(count: usize) -> Vec<Entity> {
        let mut world = World::new(Seed::new(9), 64);
        (0..count).map(|_| world.spawn()).collect()
    }

    /// ADR 0010 §2: the cap bounds DRIFT edges — a stronger newcomer
    /// evicts the weakest, a weaker one is simply not recorded — and an
    /// existing edge always updates in place (never duplicated, never
    /// blocked by the cap).
    #[test]
    fn upsert_bounds_drift_edges_at_the_cap() {
        let e = entities(5);
        let mut rel = Relationships::default();
        rel.upsert(e[1], RelKind::Friend, 300, 2);
        rel.upsert(e[2], RelKind::Friend, 500, 2);
        // Weaker than everything at the cap: not recorded.
        rel.upsert(e[3], RelKind::Friend, 200, 2);
        assert_eq!(rel.strength(e[3], RelKind::Friend), None);
        // Stronger: the weakest drift edge gives way.
        rel.upsert(e[3], RelKind::Romance, 400, 2);
        assert_eq!(rel.strength(e[1], RelKind::Friend), None);
        assert_eq!(rel.strength(e[3], RelKind::Romance), Some(400));
        // An EXISTING edge updates in place at the cap (and clamps).
        rel.upsert(e[3], RelKind::Romance, 1500, 2);
        assert_eq!(rel.strength(e[3], RelKind::Romance), Some(1000));
        assert_eq!(rel.edges.len(), 2);
        // The vector stays in (other index, kind order) order.
        let keys: Vec<(u32, u8)> = rel
            .edges
            .iter()
            .map(|edge| (edge.other.index(), edge.kind.order()))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }

    /// ADR 0010 §2: Kin and Spouse are PRIVILEGED — they always record.
    /// At the cap they evict the weakest drift edge regardless of
    /// relative strength (a one-sided kin graph would defeat the incest
    /// screen; a one-sided marriage, the bigamy check), and when only
    /// family remains they exceed the cap outright.
    #[test]
    fn upsert_always_records_kin_and_spouse() {
        let e = entities(6);
        let mut rel = Relationships::default();
        rel.upsert(e[1], RelKind::Friend, 500, 2);
        rel.upsert(e[2], RelKind::Friend, 600, 2);
        // Kin at strength 100 still lands — privilege, not strength.
        rel.upsert(e[3], RelKind::Kin, 100, 2);
        assert_eq!(rel.strength(e[3], RelKind::Kin), Some(100));
        assert_eq!(
            rel.strength(e[1], RelKind::Friend),
            None,
            "the weakest drift edge gave way to family"
        );
        // Spouse evicts the remaining friend the same way.
        rel.upsert(e[4], RelKind::Spouse, 1000, 2);
        assert_eq!(rel.strength(e[4], RelKind::Spouse), Some(1000));
        assert_eq!(rel.strength(e[2], RelKind::Friend), None);
        // Only family left: a newborn's kin edge exceeds the cap rather
        // than vanish.
        rel.upsert(e[5], RelKind::Kin, 1000, 2);
        assert_eq!(rel.strength(e[5], RelKind::Kin), Some(1000));
        assert_eq!(rel.edges.len(), 3, "family may exceed the cap");
        // A drift edge still cannot enter the all-family list.
        rel.upsert(e[1], RelKind::Friend, 900, 2);
        assert_eq!(rel.strength(e[1], RelKind::Friend), None);
    }

    /// `remove` deletes exactly the `(other, kind)` edge — the same
    /// other's other kinds survive (a sibling who was also a friend
    /// stays a sibling).
    #[test]
    fn remove_touches_only_the_exact_edge() {
        let e = entities(2);
        let mut rel = Relationships::default();
        rel.upsert(e[1], RelKind::Kin, 1000, 4);
        rel.upsert(e[1], RelKind::Friend, 400, 4);
        rel.remove(e[1], RelKind::Friend);
        assert_eq!(rel.strength(e[1], RelKind::Friend), None);
        assert_eq!(rel.strength(e[1], RelKind::Kin), Some(1000));
    }
}

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

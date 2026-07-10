//! The Phase 7 lifecycle systems (ADR 0010 §4): marriage (household
//! merge as a housing-supply event) and reproduction (heritable traits
//! with mutation). Split file for the SPEC §3 module-size rule.

use core_ecs::sim_interface::{
    Born, Married, NeedLevel, Needs, Personality, RelKind, Relationships, Residence, SchoolAge,
    Skills, Tenancy, Wallet, WorkingAge,
};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use core_rng::RngCore;
use core_types::Money;

use crate::components::{Household, HouseholdMember, Identity, Sex};
use crate::config::{FertilityConfig, PeopleConfig};

/// RNG stream for fertility draws and trait mutation (ADR 0010 §4).
pub const FERTILITY_STREAM: &str = "people.fertility";

const PER_BILLION: u64 = 1_000_000_000;

/// Day-rate system (ADR 0010 §4): a Romance edge at or above the data
/// threshold, between two single working-age citizens, marries them —
/// spouse edges, the `Married` fact, and a household merge in which the
/// absorbed side's rented home releases and its owned homes go vacant
/// (a housing-supply event, exactly like death).
pub struct MarriageSystem {
    threshold_per_mille: i32,
    edge_cap: usize,
}

impl MarriageSystem {
    /// Builds from the data threshold and edge cap (`balance/social.ron`).
    pub fn new(threshold_per_mille: i32, edge_cap: usize) -> Self {
        MarriageSystem {
            threshold_per_mille,
            edge_cap,
        }
    }
}

fn has_spouse(world: &World, citizen: Entity) -> Result<bool, EcsError> {
    Ok(world
        .get::<Relationships>(citizen)?
        .is_some_and(|relationships| {
            relationships
                .edges
                .iter()
                .any(|edge| matches!(edge.kind, RelKind::Spouse))
        }))
}

impl System for MarriageSystem {
    fn name(&self) -> &'static str {
        "people.marriage"
    }

    fn run(
        &mut self,
        world: &mut World,
        _ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        // Pass 1 (immutable): candidate pairs in entity order, deduped
        // by processing only the lower-index side.
        let mut pairs: Vec<(Entity, Entity)> = Vec::new();
        for (citizen, relationships) in world.iter::<Relationships>()? {
            for edge in &relationships.edges {
                if matches!(edge.kind, RelKind::Romance)
                    && edge.strength_per_mille >= self.threshold_per_mille
                    && citizen.index() < edge.other.index()
                {
                    pairs.push((citizen, edge.other));
                }
            }
        }
        for (a, b) in pairs {
            if !world.is_alive(a)
                || !world.is_alive(b)
                || has_spouse(world, a)?
                || has_spouse(world, b)?
                || world.get::<WorkingAge>(a)?.is_none()
                || world.get::<WorkingAge>(b)?.is_none()
            {
                continue;
            }
            // The bond: romance becomes spousehood, both sides.
            for (this, other) in [(a, b), (b, a)] {
                let mut relationships = world
                    .get::<Relationships>(this)?
                    .cloned()
                    .unwrap_or_default();
                relationships.remove(other, RelKind::Romance);
                relationships.upsert(other, RelKind::Spouse, 1000, self.edge_cap);
                world.insert(this, relationships)?;
            }
            // The register: the couple founds a NEW household — their
            // children will join it; the birth families keep their own
            // registers (merging compounded into sterile mega-
            // households; a family is a couple and its children).
            // Capture the in-laws' roofs BEFORE leaving.
            let mut fallback_home: Option<Entity> = None;
            for partner in [a, b] {
                if fallback_home.is_some() {
                    break;
                }
                if let Some(old) = world.get::<HouseholdMember>(partner)?.map(|m| m.household) {
                    let members: Vec<Entity> = world
                        .get::<Household>(old)?
                        .map(|h| h.members.clone())
                        .unwrap_or_default();
                    for member in members {
                        if member == a || member == b {
                            continue;
                        }
                        if let Some(residence) = world.get::<Residence>(member)? {
                            fallback_home = Some(residence.home);
                            break;
                        }
                    }
                }
            }
            for partner in [a, b] {
                if let Some(old) = world.get::<HouseholdMember>(partner)?.map(|m| m.household) {
                    let now_empty = {
                        if let Some(household) = world.get_mut::<Household>(old)? {
                            household.members.retain(|member| *member != partner);
                            household.members.is_empty()
                        } else {
                            false
                        }
                    };
                    if now_empty {
                        world.despawn(old)?;
                    }
                }
            }
            let new_household = world.spawn();
            let mut members = vec![a, b];
            members.sort_by_key(|member| member.index());
            world.insert(new_household, Household { members })?;
            world.insert(
                a,
                HouseholdMember {
                    household: new_household,
                },
            )?;
            world.insert(
                b,
                HouseholdMember {
                    household: new_household,
                },
            )?;
            // The roof: a partner's own residence wins; a homeless
            // couple (two nest-leavers) starts under the in-laws' roof
            // — the multi-generation HOME, with their own register.
            let kept_home = world
                .get::<Residence>(a)?
                .map(|residence| residence.home)
                .or(world.get::<Residence>(b)?.map(|residence| residence.home))
                .or(fallback_home);
            if let Some(home) = kept_home {
                for partner in [a, b] {
                    let moving = world
                        .get::<Residence>(partner)?
                        .map(|residence| residence.home)
                        != Some(home);
                    if moving {
                        if world.get::<Tenancy>(partner)?.is_some() {
                            world.remove::<Tenancy>(partner)?;
                        }
                        world.insert(partner, Residence { home })?;
                    }
                }
            }
            world.emit(&Married {
                partner_a: a,
                partner_b: b,
            })?;
        }
        Ok(())
    }
}

/// Day-rate system (ADR 0010 §4): married couples sharing a residence
/// face the mother's age band's daily birth chance (fertility stream),
/// capped by data household size. A birth spawns a real child —
/// heritable traits (parents' midpoint ± data mutation), zeroed skills,
/// the mother's home and household, kinship edges — and the `Born` fact.
pub struct FertilitySystem {
    config: FertilityConfig,
    people: PeopleConfig,
    ticks_per_year: u64,
    skill_count: usize,
    edge_cap: usize,
}

impl FertilitySystem {
    /// Builds from the validated configs and calendar.
    pub fn new(
        config: FertilityConfig,
        people: PeopleConfig,
        ticks_per_year: u64,
        skill_count: usize,
        edge_cap: usize,
    ) -> Self {
        FertilitySystem {
            config,
            people,
            ticks_per_year,
            skill_count,
            edge_cap,
        }
    }
}

impl System for FertilitySystem {
    fn name(&self) -> &'static str {
        "people.fertility"
    }

    fn run(
        &mut self,
        world: &mut World,
        ctx: &TickContext,
        _cmd: &mut CommandBuffer,
    ) -> Result<(), EcsError> {
        // Pass 1 (immutable): mothers in entity order — married women
        // inside a fertility band, sharing a roof with the spouse, in a
        // household below the cap.
        let mut mothers: Vec<(Entity, Entity)> = Vec::new();
        for (citizen, identity) in world.iter::<Identity>()? {
            if identity.sex != Sex::Female {
                continue;
            }
            let age = identity.age_years(ctx.tick, self.ticks_per_year);
            if self.config.per_day_chance(age) == 0 {
                continue;
            }
            let Some(spouse) = world.get::<Relationships>(citizen)?.and_then(|r| {
                r.edges
                    .iter()
                    .find(|edge| matches!(edge.kind, RelKind::Spouse))
                    .map(|edge| edge.other)
            }) else {
                continue;
            };
            if !world.is_alive(spouse) {
                continue;
            }
            let shared_roof = match (
                world.get::<Residence>(citizen)?,
                world.get::<Residence>(spouse)?,
            ) {
                (Some(a), Some(b)) => a.home == b.home,
                _ => false,
            };
            if !shared_roof {
                continue;
            }
            let household_size = world
                .get::<HouseholdMember>(citizen)?
                .map(|member| member.household)
                .and_then(|household| {
                    world
                        .get::<Household>(household)
                        .ok()
                        .flatten()
                        .map(|h| h.members.len() as u32)
                })
                .unwrap_or(u32::MAX);
            if household_size >= self.config.max_household_size {
                continue;
            }
            mothers.push((citizen, spouse));
        }

        for (mother, father) in mothers {
            let identity = world
                .get::<Identity>(mother)?
                .cloned()
                .ok_or(EcsError::InternalCorruption("mother lost her identity"))?;
            let age = identity.age_years(ctx.tick, self.ticks_per_year);
            let chance = u64::from(self.config.per_day_chance(age));
            let draw = world.rng(FERTILITY_STREAM).next_u64() % PER_BILLION;
            if draw >= chance {
                continue;
            }
            self.give_birth(world, ctx, mother, father, &identity)?;
        }
        Ok(())
    }
}

impl FertilitySystem {
    fn give_birth(
        &self,
        world: &mut World,
        ctx: &TickContext,
        mother: Entity,
        father: Entity,
        mother_identity: &Identity,
    ) -> Result<(), EcsError> {
        // Sex, name (the mother's family name), traits: midpoint of the
        // parents ± data mutation — all from the fertility stream, in a
        // fixed draw order.
        let sex_draw = world.rng(FERTILITY_STREAM).next_u64();
        let sex = if sex_draw.is_multiple_of(2) {
            Sex::Female
        } else {
            Sex::Male
        };
        let name_draw = world.rng(FERTILITY_STREAM).next_u64();
        let names = match sex {
            Sex::Female => &self.people.given_female.names,
            Sex::Male => &self.people.given_male.names,
        };
        let given_name = names
            .get((name_draw % names.len().max(1) as u64) as usize)
            .cloned()
            .unwrap_or_else(|| "Child".to_owned());

        let mutation = i64::from(self.config.trait_mutation_per_mille);
        let father_weights = world
            .get::<Personality>(father)?
            .map(|p| p.weights.clone())
            .unwrap_or_default();
        let mother_weights = world
            .get::<Personality>(mother)?
            .map(|p| p.weights.clone())
            .unwrap_or_default();
        let mut weights = Vec::with_capacity(self.people.traits.traits.len());
        for index in 0..self.people.traits.traits.len() {
            let base = (i64::from(mother_weights.get(index).copied().unwrap_or(0))
                + i64::from(father_weights.get(index).copied().unwrap_or(0)))
                / 2;
            let draw = world.rng(FERTILITY_STREAM).next_u64();
            let offset = (draw % (2 * mutation.max(1) as u64 + 1)) as i64 - mutation;
            weights.push((base + offset).clamp(0, 1000) as i16);
        }
        // Needs: full at birth (a newborn starts satisfied; decay is
        // life's problem).
        let levels = vec![NeedLevel::MAX; self.people.needs.needs.len()];

        let child = world.spawn();
        world.insert(
            child,
            Identity {
                given_name,
                family_name: mother_identity.family_name.clone(),
                sex,
                birth_tick: ctx.tick.raw() as i64,
            },
        )?;
        world.insert(child, Needs { levels })?;
        world.insert(child, Personality { weights })?;
        world.insert(child, Wallet { cash: Money::ZERO })?;
        world.insert(
            child,
            Skills {
                levels: vec![0; self.skill_count],
            },
        )?;
        // The roof and the register.
        if let Some(residence) = world.get::<Residence>(mother)?.copied() {
            world.insert(child, residence)?;
            world.insert(
                child,
                core_ecs::sim_interface::Position { at: residence.home },
            )?;
        }
        let household = world.get::<HouseholdMember>(mother)?.map(|m| m.household);
        if let Some(household) = household {
            world.insert(child, HouseholdMember { household })?;
            let siblings: Vec<Entity> = world
                .get::<Household>(household)?
                .map(|h| {
                    h.members
                        .iter()
                        .copied()
                        .filter(|member| *member != mother && *member != father)
                        .collect()
                })
                .unwrap_or_default();
            if let Some(h) = world.get_mut::<Household>(household)? {
                h.members.push(child);
                h.members.sort_by_key(|member| member.index());
            }
            // Kinship: parents both ways, siblings both ways.
            let mut kin: Vec<(Entity, Entity)> = vec![
                (child, mother),
                (child, father),
                (mother, child),
                (father, child),
            ];
            for sibling in siblings {
                if world.get::<Identity>(sibling)?.is_some() {
                    kin.push((child, sibling));
                    kin.push((sibling, child));
                }
            }
            for (from, to) in kin {
                let mut relationships = world
                    .get::<Relationships>(from)?
                    .cloned()
                    .unwrap_or_default();
                relationships.upsert(to, RelKind::Kin, 1000, self.edge_cap);
                world.insert(from, relationships)?;
            }
        }
        // A newborn is not yet school-age; the birthday system stamps
        // SchoolAge and WorkingAge as the years arrive.
        let _ = SchoolAge;
        world.emit(&Born {
            child,
            parent_a: if mother.index() < father.index() {
                mother
            } else {
                father
            },
            parent_b: if mother.index() < father.index() {
                father
            } else {
                mother
            },
        })?;
        Ok(())
    }
}

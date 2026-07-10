//! The Phase 7 lifecycle systems (ADR 0010 §4): marriage (the couple
//! founds a NEW household; the vacated tenancy is a housing-supply
//! event) and reproduction (heritable traits with mutation). Split file
//! for the SPEC §3 module-size rule.

use core_ecs::sim_interface::{
    Born, Married, NeedLevel, Needs, Personality, RelKind, Relationships, Residence, Skills,
    Tenancy, Wallet, WorkingAge,
};
use core_ecs::{CommandBuffer, EcsError, Entity, System, TickContext, World};
use core_rng::RngCore;
use core_types::Money;

use crate::components::{Household, HouseholdMember, Identity, Sex};
use crate::config::{FertilityConfig, PeopleConfig};

/// RNG stream for fertility draws and trait mutation (ADR 0010 §4).
pub const FERTILITY_STREAM: &str = "people.fertility";

const PER_BILLION: u64 = 1_000_000_000;

/// Day-rate system (ADR 0010 §4, as amended): a Romance edge at or
/// above the data threshold, between two single working-age citizens,
/// marries them — spouse edges, the `Married` fact, and a NEW household
/// of exactly the couple (the birth families keep their registers). A
/// partner's own home houses the pair; homeless nest-leavers start
/// under the in-laws' roof; a vacated tenancy releases to the market.
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

fn are_kin(world: &World, from: Entity, to: Entity) -> Result<bool, EcsError> {
    Ok(world.get::<Relationships>(from)?.is_some_and(|r| {
        r.edges
            .iter()
            .any(|edge| edge.other == to && matches!(edge.kind, RelKind::Kin))
    }))
}

/// How many of the mother's household members actually live under her
/// roof. The fertility cap is ROOF crowding (ADR 0010 §4, amended):
/// nest-left adult children keep their register entry but not the
/// bedroom, so they must not sterilize their parents. `u32::MAX` when
/// the mother has no roof or register — no birth without either.
fn roof_crowding(world: &World, mother: Entity) -> Result<u32, EcsError> {
    let Some(home) = world.get::<Residence>(mother)?.map(|r| r.home) else {
        return Ok(u32::MAX);
    };
    let Some(household) = world.get::<HouseholdMember>(mother)?.map(|m| m.household) else {
        return Ok(u32::MAX);
    };
    let members = world
        .get::<Household>(household)?
        .map(|h| h.members.clone())
        .unwrap_or_default();
    let mut under_roof = 0u32;
    for member in members {
        if world.get::<Residence>(member)?.map(|r| r.home) == Some(home) {
            under_roof += 1;
        }
    }
    Ok(under_roof)
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
                // Defense in depth behind the drift screen: family does
                // not marry, even if a kin-edged Romance edge somehow
                // exists (checked BOTH sides — one list may be imperfect).
                || are_kin(world, a, b)?
                || are_kin(world, b, a)?
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
            // Fidelity resets courtship (ADR 0010 §4, amended): EVERY
            // Romance edge touching either newlywed clears — their own
            // old flames and third parties' edges toward them. A stale
            // above-threshold flame must not remarry a widow the day
            // after the funeral; widows love again by NEW courtship.
            let flames: Vec<Entity> = world
                .iter::<Relationships>()?
                .filter(|(holder, relationships)| {
                    relationships.edges.iter().any(|edge| {
                        matches!(edge.kind, RelKind::Romance)
                            && (*holder == a || *holder == b || edge.other == a || edge.other == b)
                    })
                })
                .map(|(holder, _)| holder)
                .collect();
            for holder in flames {
                if let Some(relationships) = world.get_mut::<Relationships>(holder)? {
                    relationships.edges.retain(|edge| {
                        !(matches!(edge.kind, RelKind::Romance)
                            && (holder == a || holder == b || edge.other == a || edge.other == b))
                    });
                }
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
            // One draw per COUPLE (ADR 0010 §4): the father must be male
            // — a same-sex marriage is a household, not a birth chance.
            if world.get::<Identity>(spouse)?.map(|identity| identity.sex) != Some(Sex::Male) {
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
            if roof_crowding(world, citizen)? >= self.config.max_household_size {
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
            // Re-check the cap against LIVE state: an earlier birth this
            // same day may have filled the roof.
            if roof_crowding(world, mother)? >= self.config.max_household_size {
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
            // `2·m + 1` outcomes centred on zero — with mutation 0 the
            // span is 1 and the offset is exactly 0 (zero mutation means
            // zero drift, not an upward bias), still one draw per trait.
            let draw = world.rng(FERTILITY_STREAM).next_u64();
            let offset = (draw % (2 * mutation as u64 + 1)) as i64 - mutation;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AgeBand, DemographicsConfig, FertilityBand, MortalityConfig, NameList, NeedDef,
        NeedsConfig, TraitDef, TraitsConfig,
    };
    use core_ecs::sim_interface::Position;
    use core_ecs::{CommandBuffer, TickContext};
    use core_types::{CalendarTime, Seed, Ticks};

    const YEAR: u64 = 120 * 1440;
    const EDGE_CAP: usize = 12;

    fn ctx() -> TickContext {
        TickContext {
            tick: Ticks::new(0),
            time: CalendarTime::START,
        }
    }

    fn social_world() -> World {
        let mut world = World::new(Seed::new(31), 64);
        world.register::<Identity>().expect("register");
        world.register::<Household>().expect("register");
        world.register::<HouseholdMember>().expect("register");
        world.register::<Relationships>().expect("register");
        world.register::<Residence>().expect("register");
        world.register::<Tenancy>().expect("register");
        world.register::<WorkingAge>().expect("register");
        world.register::<Needs>().expect("register");
        world.register::<Personality>().expect("register");
        world.register::<Wallet>().expect("register");
        world.register::<Skills>().expect("register");
        world.register::<Position>().expect("register");
        world.register_event::<Married>().expect("register");
        world.register_event::<Born>().expect("register");
        world
    }

    /// A working-age adult aged 30, in their own one-member household.
    fn adult(world: &mut World, name: &str, sex: Sex) -> Entity {
        let entity = world.spawn();
        world
            .insert(
                entity,
                Identity {
                    given_name: name.to_owned(),
                    family_name: "Test".to_owned(),
                    sex,
                    birth_tick: -((30 * YEAR) as i64),
                },
            )
            .expect("insert");
        world.insert(entity, WorkingAge).expect("insert");
        let household = world.spawn();
        world
            .insert(
                household,
                Household {
                    members: vec![entity],
                },
            )
            .expect("insert");
        world
            .insert(entity, HouseholdMember { household })
            .expect("insert");
        entity
    }

    fn romance(world: &mut World, a: Entity, b: Entity, strength: i32) {
        for (this, other) in [(a, b), (b, a)] {
            let mut rel = world
                .get::<Relationships>(this)
                .expect("query")
                .cloned()
                .unwrap_or_default();
            rel.upsert(other, RelKind::Romance, strength, EDGE_CAP);
            world.insert(this, rel).expect("insert");
        }
    }

    fn strength(world: &World, from: Entity, to: Entity, kind: RelKind) -> Option<i32> {
        world
            .get::<Relationships>(from)
            .expect("query")
            .and_then(|rel| rel.strength(to, kind))
    }

    /// ADR 0010 §4 (as amended): the wedding founds a NEW household of
    /// exactly the couple, houses them under a partner's roof (releasing
    /// the mover's tenancy), and clears EVERY Romance edge touching
    /// either newlywed — the old flame's edge included.
    #[test]
    fn a_wedding_founds_a_household_and_clears_every_flame() {
        let mut world = social_world();
        let a = adult(&mut world, "Ada", Sex::Female);
        let b = adult(&mut world, "Bram", Sex::Male);
        let flame = adult(&mut world, "Cass", Sex::Female);
        let home_a = world.spawn();
        let home_b = world.spawn();
        world.insert(a, Residence { home: home_a }).expect("insert");
        world.insert(b, Residence { home: home_b }).expect("insert");
        world
            .insert(
                b,
                Tenancy {
                    home: home_b,
                    rent_per_day: core_types::Money::from_mills(60),
                },
            )
            .expect("insert");
        romance(&mut world, a, b, 800);
        // The old flame holds a one-sided, above-threshold edge to b.
        let mut rel = world
            .get::<Relationships>(flame)
            .expect("query")
            .cloned()
            .unwrap_or_default();
        rel.upsert(b, RelKind::Romance, 900, EDGE_CAP);
        world.insert(flame, rel).expect("insert");

        let old_household_b = world
            .get::<HouseholdMember>(b)
            .expect("query")
            .expect("member")
            .household;
        MarriageSystem::new(700, EDGE_CAP)
            .run(&mut world, &ctx(), &mut CommandBuffer::new())
            .expect("run");

        assert_eq!(strength(&world, a, b, RelKind::Spouse), Some(1000));
        assert_eq!(strength(&world, b, a, RelKind::Spouse), Some(1000));
        assert_eq!(strength(&world, a, b, RelKind::Romance), None);
        assert_eq!(
            strength(&world, flame, b, RelKind::Romance),
            None,
            "fidelity resets courtship — the flame's edge cleared too"
        );
        // The register: a NEW household of exactly the couple; the
        // vacated one-member registers despawned.
        let household = world
            .get::<HouseholdMember>(a)
            .expect("query")
            .expect("member")
            .household;
        assert_eq!(
            world
                .get::<HouseholdMember>(b)
                .expect("query")
                .expect("member")
                .household,
            household
        );
        let mut expected = vec![a, b];
        expected.sort_by_key(|entity| entity.index());
        assert_eq!(
            world
                .get::<Household>(household)
                .expect("query")
                .expect("household")
                .members,
            expected
        );
        assert!(
            world
                .get::<Household>(old_household_b)
                .expect("query")
                .is_none(),
            "the emptied birth register despawned"
        );
        // The roof: a's home wins; the mover's tenancy released.
        assert_eq!(
            world
                .get::<Residence>(b)
                .expect("query")
                .expect("housed")
                .home,
            home_a
        );
        assert!(world.get::<Tenancy>(b).expect("query").is_none());
    }

    /// ADR 0010 §4 (as amended): family does not marry — the kin bar
    /// holds in MarriageSystem itself, even one-sided.
    #[test]
    fn kin_never_marry() {
        let mut world = social_world();
        let a = adult(&mut world, "Dane", Sex::Male);
        let b = adult(&mut world, "Etta", Sex::Female);
        romance(&mut world, a, b, 900);
        // One-sided on purpose: only b's list knows they are family.
        let mut rel = world
            .get::<Relationships>(b)
            .expect("query")
            .cloned()
            .unwrap_or_default();
        rel.upsert(a, RelKind::Kin, 1000, EDGE_CAP);
        world.insert(b, rel).expect("insert");

        MarriageSystem::new(700, EDGE_CAP)
            .run(&mut world, &ctx(), &mut CommandBuffer::new())
            .expect("run");
        assert_eq!(strength(&world, a, b, RelKind::Spouse), None);
        assert_eq!(strength(&world, b, a, RelKind::Spouse), None);
    }

    fn people_config() -> PeopleConfig {
        PeopleConfig {
            needs: NeedsConfig {
                needs: vec![NeedDef {
                    id: "hunger".into(),
                    decay_per_hour: 0,
                    initial_min: 0,
                    initial_max: 0,
                }],
            },
            traits: TraitsConfig {
                traits: vec![TraitDef {
                    id: "ambition".into(),
                    min: 0,
                    max: 1000,
                }],
            },
            mortality: MortalityConfig {
                bands: vec![],
                terminal_per_day_chance_per_billion: 0,
            },
            demographics: DemographicsConfig {
                age_bands: vec![AgeBand {
                    min_age_years: 20,
                    max_age_years: 40,
                    weight_per_mille: 1000,
                }],
                male_per_mille: 500,
                household_min: 1,
                household_max: 1,
                annual_death_rate_min_per_mille: 0,
                annual_death_rate_max_per_mille: 1000,
                wealth_min_mills: 0,
                wealth_max_mills: 0,
            },
            given_female: NameList {
                names: vec!["Wren".into()],
            },
            given_male: NameList {
                names: vec!["Ash".into()],
            },
            family: NameList {
                names: vec!["Vale".into()],
            },
        }
    }

    fn fertility_config(max_household_size: u32, mutation: u16) -> FertilityConfig {
        FertilityConfig {
            bands: vec![FertilityBand {
                min_age_years: 16,
                max_age_years: 60,
                // Certainty: every eligible mother draws a birth.
                per_day_chance_per_billion: 1_000_000_000,
            }],
            max_household_size,
            trait_mutation_per_mille: mutation,
        }
    }

    /// Marries `a` and `b` by hand (spouse edges + shared register +
    /// shared roof) without running MarriageSystem.
    fn wed(world: &mut World, a: Entity, b: Entity, home: Entity) {
        for (this, other) in [(a, b), (b, a)] {
            let mut rel = world
                .get::<Relationships>(this)
                .expect("query")
                .cloned()
                .unwrap_or_default();
            rel.upsert(other, RelKind::Spouse, 1000, EDGE_CAP);
            world.insert(this, rel).expect("insert");
            world.insert(this, Residence { home }).expect("insert");
        }
        let household = world
            .get::<HouseholdMember>(a)
            .expect("query")
            .expect("member")
            .household;
        world
            .insert(b, HouseholdMember { household })
            .expect("insert");
        let mut members = vec![a, b];
        members.sort_by_key(|entity| entity.index());
        world
            .insert(household, Household { members })
            .expect("insert");
    }

    fn population(world: &World) -> usize {
        world.iter::<Identity>().expect("query").count()
    }

    /// ADR 0010 §4: one draw per COUPLE, and the father must be male —
    /// a same-sex marriage is a household, not a birth chance (and never
    /// a double one).
    #[test]
    fn births_require_a_male_father() {
        let mut world = social_world();
        let a = adult(&mut world, "Fay", Sex::Female);
        let b = adult(&mut world, "Gwen", Sex::Female);
        let home = world.spawn();
        wed(&mut world, a, b, home);
        let mut system =
            FertilitySystem::new(fertility_config(6, 120), people_config(), YEAR, 1, EDGE_CAP);
        let before = population(&world);
        system
            .run(&mut world, &ctx(), &mut CommandBuffer::new())
            .expect("run");
        assert_eq!(population(&world), before, "no father, no birth");

        // The control couple proves the harness: a birth lands, with the
        // full newborn kit.
        let c = adult(&mut world, "Hale", Sex::Male);
        let d = adult(&mut world, "Ivy", Sex::Female);
        let home2 = world.spawn();
        wed(&mut world, c, d, home2);
        let before = population(&world);
        system
            .run(&mut world, &ctx(), &mut CommandBuffer::new())
            .expect("run");
        assert_eq!(population(&world), before + 1, "the control couple bore");
        let child = world
            .iter::<Identity>()
            .expect("query")
            .find(|(_, identity)| identity.birth_tick == 0)
            .map(|(entity, _)| entity)
            .expect("newborn");
        assert_eq!(strength(&world, child, d, RelKind::Kin), Some(1000));
        assert_eq!(strength(&world, c, child, RelKind::Kin), Some(1000));
        assert_eq!(
            world
                .get::<Residence>(child)
                .expect("query")
                .expect("housed")
                .home,
            home2,
            "the child is born under the mother's roof"
        );
    }

    /// ADR 0010 §4 (as amended): the cap counts household members UNDER
    /// THE ROOF — a nest-left adult child on the register must not
    /// sterilize the parents; a resident one at the cap must.
    #[test]
    fn the_fertility_cap_counts_only_those_under_the_roof() {
        for (grown_child_resident, expected_births) in [(false, 1usize), (true, 0usize)] {
            let mut world = social_world();
            let a = adult(&mut world, "Jules", Sex::Male);
            let b = adult(&mut world, "Kira", Sex::Female);
            let home = world.spawn();
            wed(&mut world, a, b, home);
            // A grown child on the register (cap 3: couple + one).
            let grown = adult(&mut world, "Lark", Sex::Male);
            let household = world
                .get::<HouseholdMember>(a)
                .expect("query")
                .expect("member")
                .household;
            world
                .insert(grown, HouseholdMember { household })
                .expect("insert");
            let mut members = vec![a, b, grown];
            members.sort_by_key(|entity| entity.index());
            world
                .insert(household, Household { members })
                .expect("insert");
            if grown_child_resident {
                world.insert(grown, Residence { home }).expect("insert");
            }
            let before = population(&world);
            FertilitySystem::new(fertility_config(3, 120), people_config(), YEAR, 1, EDGE_CAP)
                .run(&mut world, &ctx(), &mut CommandBuffer::new())
                .expect("run");
            assert_eq!(
                population(&world) - before,
                expected_births,
                "resident={grown_child_resident}"
            );
        }
    }

    /// The live cap re-check: two eligible mothers under ONE roof at
    /// cap−1 cannot both give birth the same day — the first birth fills
    /// the roof.
    #[test]
    fn a_same_day_double_birth_cannot_breach_the_cap() {
        let mut world = social_world();
        let a = adult(&mut world, "Moss", Sex::Male);
        let b = adult(&mut world, "Nell", Sex::Female);
        let c = adult(&mut world, "Orin", Sex::Male);
        let d = adult(&mut world, "Prue", Sex::Female);
        let home = world.spawn();
        wed(&mut world, a, b, home);
        wed(&mut world, c, d, home);
        // One register, one roof: all four in a's household.
        let household = world
            .get::<HouseholdMember>(a)
            .expect("query")
            .expect("member")
            .household;
        for member in [c, d] {
            world
                .insert(member, HouseholdMember { household })
                .expect("insert");
        }
        let mut members = vec![a, b, c, d];
        members.sort_by_key(|entity| entity.index());
        world
            .insert(household, Household { members })
            .expect("insert");
        let before = population(&world);
        FertilitySystem::new(fertility_config(5, 120), people_config(), YEAR, 1, EDGE_CAP)
            .run(&mut world, &ctx(), &mut CommandBuffer::new())
            .expect("run");
        assert_eq!(
            population(&world) - before,
            1,
            "the first birth filled the roof; the second draw respected it"
        );
    }

    /// ADR 0010 §4: heritable traits are the parents' midpoint ± the
    /// data mutation, clamped — and ZERO mutation means exactly the
    /// midpoint, not an upward drift.
    #[test]
    fn newborn_traits_stay_within_the_mutation_band() {
        for (mutation, low, high) in [(0u16, 500i16, 500i16), (120, 380, 620)] {
            let mut world = social_world();
            let a = adult(&mut world, "Quin", Sex::Male);
            let b = adult(&mut world, "Rue", Sex::Female);
            world
                .insert(a, Personality { weights: vec![400] })
                .expect("insert");
            world
                .insert(b, Personality { weights: vec![600] })
                .expect("insert");
            let home = world.spawn();
            wed(&mut world, a, b, home);
            FertilitySystem::new(
                fertility_config(6, mutation),
                people_config(),
                YEAR,
                1,
                EDGE_CAP,
            )
            .run(&mut world, &ctx(), &mut CommandBuffer::new())
            .expect("run");
            let child = world
                .iter::<Identity>()
                .expect("query")
                .find(|(_, identity)| identity.birth_tick == 0)
                .map(|(entity, _)| entity)
                .expect("newborn");
            let weight = world
                .get::<Personality>(child)
                .expect("query")
                .expect("traits")
                .weights[0];
            assert!(
                (low..=high).contains(&weight),
                "mutation {mutation}: trait {weight} outside [{low}, {high}]"
            );
        }
    }
}

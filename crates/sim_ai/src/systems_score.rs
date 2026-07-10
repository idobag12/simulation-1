//! The decision scoring kernel (SPEC §11): floats live HERE and only
//! here — pure `+ − × ÷` over exact integer inputs. Split from
//! `systems.rs` for the SPEC §3 module-size rule.

use core_ecs::Entity;

use super::{Decider, MICRO, NEED_MAX, OfferSnapshot};
use crate::systems::DecideSystem;

impl DecideSystem {
    /// Scores one candidate. Float discipline: `+ − × ÷` only (ADR 0006 §4).
    #[allow(
        clippy::too_many_arguments,
        reason = "pure scoring kernel; a params struct would only rename the locals"
    )]
    pub(crate) fn score(
        &self,
        level: i64,
        rate: i64,
        traveling: bool,
        traits: &[i16],
        need_index: u32,
        is_own_home_rest: bool,
        asleep_window: bool,
    ) -> f64 {
        let deficit = (NEED_MAX - level).max(0);
        // Recoverable amount is bounded by the longest performance.
        let recoverable =
            deficit.min(rate.saturating_mul(i64::from(self.tables.max_perform_ticks)));
        if recoverable <= 0 {
            return f64::MIN; // nothing to gain: never chosen over Idle (0.0)
        }
        let perform_ticks = recoverable.div_euclid(rate) + i64::from(recoverable % rate != 0);
        let travel_ticks = if traveling {
            i64::from(self.tables.travel_ticks)
        } else {
            0
        };

        let gain = recoverable as f64 / MICRO;
        let time_cost = (travel_ticks + perform_ticks) as f64
            * self.tables.time_cost_micro_per_tick as f64
            / MICRO;
        let sleep_bias = if is_own_home_rest && asleep_window {
            self.tables.sleep_home_bias_micro as f64 / MICRO
        } else {
            0.0
        };

        gain * self.urgency(deficit) * self.trait_factor(traits, need_index) - time_cost
            + sleep_bias
    }

    /// `deficit^exponent` in tank fractions (the SPEC §11 nonlinearity).
    pub(crate) fn urgency(&self, deficit: i64) -> f64 {
        let deficit_frac = deficit as f64 / MICRO;
        let mut urgency = 1.0f64;
        for _ in 0..self.tables.urgency_exponent {
            urgency *= deficit_frac;
        }
        urgency
    }

    /// The personality amplifier for `need_index` (1.0 when unmapped).
    pub(crate) fn trait_factor(&self, traits: &[i16], need_index: u32) -> f64 {
        match self
            .tables
            .need_trait
            .get(need_index as usize)
            .copied()
            .flatten()
        {
            Some((trait_index, weight_per_mille)) => {
                let trait_value = traits
                    .get(trait_index as usize)
                    .copied()
                    .unwrap_or(0)
                    .max(0) as f64
                    / 1000.0;
                1.0 + (f64::from(weight_per_mille) / 1000.0) * trait_value
            }
            None => 1.0,
        }
    }

    /// Scores buying one unit from `snapshot` (ADR 0007 §6): the gain a
    /// unit's satisfaction offers against time cost AND money cost —
    /// `price × mu`, where the marginal utility of wealth
    /// `mu = mu_scale / (1 + wallet/half_wealth)` makes the same price
    /// weigh more on a thin wallet. Float discipline: `+ − × ÷` only.
    pub(crate) fn score_buy(&self, decider: &Decider, snapshot: &OfferSnapshot) -> f64 {
        let offer = &snapshot.offer;
        let level = decider
            .needs
            .get(offer.need_index as usize)
            .copied()
            .unwrap_or(NEED_MAX);
        let deficit = (NEED_MAX - level).max(0);
        let recoverable = deficit.min(offer.gain_per_unit.max(0));
        if recoverable <= 0 {
            return f64::MIN;
        }
        let travel_ticks = if decider.at == Some(snapshot.seller) {
            0
        } else {
            i64::from(self.tables.travel_ticks)
        };
        let gain = recoverable as f64 / MICRO;
        let time_cost = (travel_ticks + i64::from(offer.use_ticks)) as f64
            * self.tables.time_cost_micro_per_tick as f64
            / MICRO;
        let mu = (self.tables.mu_scale_micro as f64 / MICRO)
            / (1.0 + decider.wealth_mills as f64 / self.tables.half_wealth_mills as f64);
        // The BELIEVED price weighs the choice when one exists (Phase 7,
        // ADR 0010 §3) — the till still charges the posted price, and
        // experience corrects the belief there.
        let believed = decider
            .believed
            .iter()
            .find(|(shop, _)| *shop == snapshot.seller.index())
            .map(|(_, mills)| *mills)
            .unwrap_or(offer.unit_price.mills());
        let money_cost = believed as f64 * mu;

        gain * self.urgency(deficit) * self.trait_factor(&decider.traits, offer.need_index)
            - time_cost
            - money_cost
    }

    /// Scores going to work (Phase 5, ADR 0008 §2): a flat data-defined
    /// bias during the shift, less time cost — strong enough to shape the
    /// day, weak enough that urgent needs still win (obligations bias,
    /// never dictate).
    pub(crate) fn score_work(&self, decider: &Decider, workplace: Entity) -> f64 {
        let travel_ticks = if decider.at == Some(workplace) {
            0
        } else {
            i64::from(self.tables.travel_ticks)
        };
        let time_cost = (travel_ticks + i64::from(self.tables.work_ticks)) as f64
            * self.tables.time_cost_micro_per_tick as f64
            / MICRO;
        self.tables.work_bias_micro as f64 / MICRO - time_cost
    }

    /// Scores attending school (Phase 7, ADR 0010 §1): the same shape
    /// as the work bias — obligations bias, never dictate.
    pub(crate) fn score_school(&self, decider: &Decider, school: Entity) -> f64 {
        let travel_ticks = if decider.at == Some(school) {
            0
        } else {
            i64::from(self.tables.travel_ticks)
        };
        let time_cost = (travel_ticks + i64::from(self.tables.school_attend_ticks)) as f64
            * self.tables.time_cost_micro_per_tick as f64
            / MICRO;
        self.tables.school_bias_micro as f64 / MICRO - time_cost
    }
}

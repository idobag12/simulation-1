//! AI-owned components: what a citizen is doing, their compiled plan, and
//! the inspectable dump of their last decision (ADR 0006 §§3, 5–6).

use core_ecs::{Component, Entity, StorageKind};
use core_types::Ticks;
use serde::{Deserialize, Serialize};

/// What a citizen is currently doing. Absent ⇒ the citizen decides on the
/// next `DecideSystem` run.
///
/// Invariants: `remaining` counts down by exactly 1 per tick in
/// `ActSystem`; a `Perform` at a location implies the citizen's
/// `Position` is that location (set on arrival).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CurrentAction {
    /// En route to `target` to satisfy `need_index`.
    Travel {
        /// Destination location entity.
        target: Entity,
        /// Need (data order index) to satisfy on arrival.
        need_index: u32,
        /// Ticks of travel left.
        remaining: u32,
    },
    /// At `at`, gaining need satisfaction each tick.
    Perform {
        /// The location being used.
        at: Entity,
        /// Need (data order index) being satisfied.
        need_index: u32,
        /// Ticks of performance left (ends early when the need fills).
        remaining: u32,
    },
    /// Doing nothing for a bounded time (re-decides afterwards).
    Idle {
        /// Ticks of idling left.
        remaining: u32,
    },
}

impl Component for CurrentAction {
    const NAME: &'static str = "ai.current_action";
    // Dense: most citizens are doing something most ticks.
    const STORAGE: StorageKind = StorageKind::Dense;
}

/// A citizen's compiled next-day plan (ADR 0006 §5): currently the one
/// block reality affords — the sleep window. Minutes of day (0..1440);
/// the window may cross midnight (start > end).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyPlan {
    /// Sleep window start, minute of day.
    pub sleep_start_minute: u16,
    /// Sleep window end, minute of day.
    pub sleep_end_minute: u16,
}

impl DailyPlan {
    /// True iff `minute_of_day` falls inside the (possibly
    /// midnight-crossing) sleep window.
    pub fn in_sleep_window(&self, minute_of_day: u16) -> bool {
        if self.sleep_start_minute <= self.sleep_end_minute {
            (self.sleep_start_minute..self.sleep_end_minute).contains(&minute_of_day)
        } else {
            minute_of_day >= self.sleep_start_minute || minute_of_day < self.sleep_end_minute
        }
    }
}

impl Component for DailyPlan {
    const NAME: &'static str = "ai.daily_plan";
    // Dense: every citizen carries one once the first compile has run.
    const STORAGE: StorageKind = StorageKind::Dense;
}

/// One scored option from a decision (SPEC §11 inspectability).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoredCandidate {
    /// What the option was.
    pub action: CandidateAction,
    /// The utility score, quantized to micro units for storage (floats
    /// are never persisted, SPEC §2; quantization of a deterministic
    /// float is deterministic).
    pub score_micro: i64,
}

/// A candidate action in a decision dump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandidateAction {
    /// Go to `location` and satisfy `need_index`.
    Satisfy {
        /// Target location entity.
        location: Entity,
        /// Need (data order index).
        need_index: u32,
    },
    /// Do nothing for a while.
    Idle,
}

/// The full scored candidate list of a citizen's most recent decision —
/// "why did Mara skip work?" answerable in one click (SPEC §11/§13).
///
/// Invariant: `chosen` indexes `candidates`; candidates appear in
/// enumeration order (own home first, then public locations in entity
/// order, each in satisfier data order, `Idle` last).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastDecision {
    /// Tick the decision was made.
    pub tick: Ticks,
    /// Index of the chosen candidate.
    pub chosen: u32,
    /// Every candidate that was scored.
    pub candidates: Vec<ScoredCandidate>,
}

impl Component for LastDecision {
    const NAME: &'static str = "ai.last_decision";
    // Dense: every deciding citizen carries one.
    const STORAGE: StorageKind = StorageKind::Dense;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_window_handles_midnight_crossing() {
        let plan = DailyPlan {
            sleep_start_minute: 22 * 60,
            sleep_end_minute: 6 * 60,
        };
        assert!(plan.in_sleep_window(23 * 60));
        assert!(plan.in_sleep_window(0));
        assert!(plan.in_sleep_window(5 * 60 + 59));
        assert!(!plan.in_sleep_window(6 * 60));
        assert!(!plan.in_sleep_window(12 * 60));
        assert!(plan.in_sleep_window(22 * 60));
        assert!(!plan.in_sleep_window(21 * 60 + 59));

        let daytime_nap = DailyPlan {
            sleep_start_minute: 13 * 60,
            sleep_end_minute: 15 * 60,
        };
        assert!(daytime_nap.in_sleep_window(14 * 60));
        assert!(!daytime_nap.in_sleep_window(16 * 60));
    }
}

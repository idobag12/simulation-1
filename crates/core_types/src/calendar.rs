//! Calendar value types (ADR 0004 §6).
//!
//! The *structure* of simulated time is definitional and fixed by SPEC §5:
//! 1 tick = 1 minute, 60 minutes per hour, 24 hours per day (1440
//! ticks/day), 4 seasons per year. The one tunable — days per season —
//! lives in `data/balance/calendar.ron` and is owned by `sim_time`'s
//! `Calendar`, which produces these values. This module holds only the
//! plain value types so `TickContext` can carry them without upward
//! dependencies.

use serde::{Deserialize, Serialize};

/// Ticks per simulated minute. Definitional (SPEC §5), not a tunable.
pub const TICKS_PER_MINUTE: u64 = 1;
/// Minutes per hour. Definitional (SPEC §5), not a tunable.
pub const MINUTES_PER_HOUR: u64 = 60;
/// Hours per day. Definitional (SPEC §5), not a tunable.
pub const HOURS_PER_DAY: u64 = 24;
/// Ticks per day (1440, SPEC §5). Definitional, not a tunable.
pub const TICKS_PER_DAY: u64 = TICKS_PER_MINUTE * MINUTES_PER_HOUR * HOURS_PER_DAY;
/// Seasons per year. Definitional (four named seasons), not a tunable.
pub const SEASONS_PER_YEAR: u64 = 4;

/// A season of the simulated year, in calendar order.
///
/// Invariant: the discriminant order (Spring → Winter) is the in-year
/// order; `Season::from_index` and `Season::index` are inverse bijections
/// on 0..4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Season {
    /// First season of the year.
    Spring,
    /// Second season.
    Summer,
    /// Third season.
    Autumn,
    /// Fourth season.
    Winter,
}

impl Season {
    /// Season for an in-year index 0..4 (values ≥ 4 wrap; callers pass
    /// `season_ordinal % SEASONS_PER_YEAR`).
    pub const fn from_index(index: u64) -> Season {
        match index % SEASONS_PER_YEAR {
            0 => Season::Spring,
            1 => Season::Summer,
            2 => Season::Autumn,
            _ => Season::Winter,
        }
    }

    /// In-year index (0..4).
    pub const fn index(self) -> u64 {
        match self {
            Season::Spring => 0,
            Season::Summer => 1,
            Season::Autumn => 2,
            Season::Winter => 3,
        }
    }
}

/// A tick decomposed into calendar coordinates.
///
/// Invariants: `minute < 60`, `hour < 24`, `season`/`day_of_season` are
/// consistent with the `days_per_season` of the `Calendar` that produced
/// this value; all fields are 0-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CalendarTime {
    /// Year, counted from 0.
    pub year: u64,
    /// Season within the year.
    pub season: Season,
    /// Day within the season, 0-based.
    pub day_of_season: u32,
    /// Hour of day, 0..24.
    pub hour: u8,
    /// Minute of hour, 0..60.
    pub minute: u8,
}

impl CalendarTime {
    /// The very first instant: year 0, Spring, day 0, 00:00 (tick 0).
    pub const START: CalendarTime = CalendarTime {
        year: 0,
        season: Season::Spring,
        day_of_season: 0,
        hour: 0,
        minute: 0,
    };

    /// True iff this tick begins an hour (drives hour-rate systems).
    pub const fn starts_hour(&self) -> bool {
        self.minute == 0
    }

    /// True iff this tick begins a day.
    pub const fn starts_day(&self) -> bool {
        self.starts_hour() && self.hour == 0
    }

    /// True iff this tick begins a season.
    pub const fn starts_season(&self) -> bool {
        self.starts_day() && self.day_of_season == 0
    }

    /// True iff this tick begins a year.
    pub const fn starts_year(&self) -> bool {
        self.starts_season() && matches!(self.season, Season::Spring)
    }
}

impl std::fmt::Display for CalendarTime {
    /// E.g. `Y3 Autumn D12 08:30`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Y{} {:?} D{} {:02}:{:02}",
            self.year, self.season, self.day_of_season, self.hour, self.minute
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn season_index_round_trips() {
        for i in 0..4 {
            assert_eq!(Season::from_index(i).index(), i);
        }
        assert_eq!(Season::from_index(7), Season::Winter);
    }

    #[test]
    fn boundary_flags_nest() {
        assert!(CalendarTime::START.starts_year());
        assert!(CalendarTime::START.starts_season());
        assert!(CalendarTime::START.starts_day());
        assert!(CalendarTime::START.starts_hour());

        let noon = CalendarTime {
            hour: 12,
            ..CalendarTime::START
        };
        assert!(noon.starts_hour());
        assert!(!noon.starts_day());

        let winter_start = CalendarTime {
            season: Season::Winter,
            ..CalendarTime::START
        };
        assert!(winter_start.starts_season());
        assert!(!winter_start.starts_year());
    }
}

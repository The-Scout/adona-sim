//! Deterministic simulation clock.
//!
//! Primary strategic time advances day by day. Quarters are the typed seam
//! for later sub-day scheduling (convoy contacts, interception windows,
//! rumor movement); `World::tick` currently advances whole days only.

use serde::{Deserialize, Serialize};

/// Sub-day quarter marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DayQuarter {
    Q1,
    Q2,
    Q3,
    Q4,
}

/// The simulation clock. Day 0 is the seed day; the first `tick` moves to
/// day 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimClock {
    pub day: u64,
    pub quarter: DayQuarter,
}

impl SimClock {
    pub fn start() -> Self {
        SimClock {
            day: 0,
            quarter: DayQuarter::Q1,
        }
    }

    pub(crate) fn advance_day(&mut self) {
        self.day += 1;
        self.quarter = DayQuarter::Q1;
    }
}

use std::sync::{Arc, Mutex};

use crate::support::sync::lock_unpoisoned;
use crate::support::{Clock, DateTime, Timezone};

/// Controllable clock for application code that reads time through `AppContext::clock()`.
#[derive(Clone, Debug)]
pub struct ClockFake {
    clock: Clock,
    control: Arc<Mutex<DateTime>>,
}

impl ClockFake {
    pub fn new(now: DateTime, timezone: Timezone) -> Self {
        let (clock, control) = Clock::controlled(timezone, now);
        Self { clock, control }
    }

    pub fn utc(now: DateTime) -> Self {
        Self::new(now, Timezone::utc())
    }

    pub fn now(&self) -> DateTime {
        *lock_unpoisoned(&self.control, "clock fake")
    }

    pub fn set(&self, now: DateTime) -> &Self {
        *lock_unpoisoned(&self.control, "clock fake") = now;
        self
    }

    pub fn advance_seconds(&self, seconds: i64) -> &Self {
        let mut now = lock_unpoisoned(&self.control, "clock fake");
        *now = now.add_seconds(seconds);
        self
    }

    pub fn rewind_seconds(&self, seconds: i64) -> &Self {
        let mut now = lock_unpoisoned(&self.control, "clock fake");
        *now = now.sub_seconds(seconds);
        self
    }

    pub(crate) fn clock(&self) -> Clock {
        self.clock.clone()
    }

    #[track_caller]
    pub fn assert_now(&self, expected: DateTime) -> &Self {
        assert_eq!(self.now(), expected, "unexpected fake clock time");
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_controls_every_clock_clone() {
        let start = DateTime::parse("2026-07-11T00:00:00Z").unwrap();
        let fake = ClockFake::utc(start);
        let clock = fake.clock();

        fake.advance_seconds(90);

        assert_eq!(clock.now(), start.add_seconds(90));
        fake.assert_now(start.add_seconds(90));
    }
}

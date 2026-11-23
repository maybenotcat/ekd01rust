//! Distance-counter continuity and durable distance checkpoint scheduling.
use crate::preferences::Preferences;

const TICKS_PER_SECOND: u32 = 128;
const COUNTER_UNIT_METRES: u32 = 10;
const CONTINUITY_TICKS: u32 = 5 * TICKS_PER_SECOND;
// A corruption guard, not an engine speed limit: 360 km/h leaves ample margin
// above bicycle speeds. One counter unit accounts for 10 m quantization.
const MAX_PLAUSIBLE_METRES_PER_SECOND: u32 = 100;
pub const CHECKPOINT_TICKS: u32 = 60 * TICKS_PER_SECOND;

#[derive(Clone, Copy)]
struct Sample {
    counter: u16,
    tick: u32,
}
impl Sample {
    fn delta_to(self, next: Self) -> Option<u32> {
        let elapsed = next.tick.wrapping_sub(self.tick);
        if elapsed > CONTINUITY_TICKS {
            return None;
        }
        let units = next.counter.wrapping_sub(self.counter) as u32;
        let maximum = elapsed * MAX_PLAUSIBLE_METRES_PER_SECOND
            / (TICKS_PER_SECOND * COUNTER_UNIT_METRES)
            + 1;
        (units <= maximum).then_some(units * COUNTER_UNIT_METRES)
    }
}

#[derive(Clone, Copy, Default)]
pub struct DistanceCounter {
    anchor: Option<Sample>,
    candidate: Option<Sample>,
    last_received: u32,
    pub discontinuities: u32,
}
impl DistanceCounter {
    pub fn observe(&mut self, counter: u16, now: u32) -> u32 {
        let current = Sample { counter, tick: now };
        let previous_receive = self.last_received;
        self.last_received = now;
        let Some(anchor) = self.anchor else {
            self.anchor = Some(current);
            return 0;
        };
        if now.wrapping_sub(previous_receive) > CONTINUITY_TICKS {
            // Missing distance is unknown after a long outage. Rebaseline rather
            // than converting a possible motor restart into a large forward wrap.
            self.anchor = Some(current);
            self.candidate = None;
            self.discontinuities = self.discontinuities.saturating_add(1);
            return 0;
        }
        if let Some(metres) = anchor.delta_to(current) {
            self.anchor = Some(current);
            self.candidate = None;
            return metres;
        }
        if let Some(metres) = self.candidate.and_then(|sample| sample.delta_to(current)) {
            // Two coherent samples establish a new motor-counter baseline. Count
            // only their small increment, never the discontinuity from the old one.
            self.anchor = Some(current);
            self.candidate = None;
            return metres;
        }
        // Retain the good anchor so one corrupt/out-of-order packet cannot make
        // the next normal packet add a fictitious distance.
        self.candidate = Some(current);
        self.discontinuities = self.discontinuities.saturating_add(1);
        0
    }
}

pub struct DistanceCheckpoint {
    saved: [u32; 3],
    last_attempt: u32,
}
impl DistanceCheckpoint {
    fn totals(preferences: &Preferences) -> [u32; 3] {
        [
            preferences.odometer_m,
            preferences.trip_m,
            preferences.since_service_m,
        ]
    }
    pub fn new(now: u32, preferences: &Preferences) -> Self {
        Self {
            saved: Self::totals(preferences),
            last_attempt: now,
        }
    }
    pub fn due(&self, now: u32, preferences: &Preferences) -> bool {
        now.wrapping_sub(self.last_attempt) >= CHECKPOINT_TICKS
            && self.saved != Self::totals(preferences)
    }
    pub fn finished(&mut self, now: u32, preferences: &Preferences, success: bool) {
        // Failed writes remain dirty, but retry no faster than once per minute.
        self.last_attempt = now;
        if success {
            self.saved = Self::totals(preferences);
        }
    }
}

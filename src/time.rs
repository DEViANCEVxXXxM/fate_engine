use std::time::Instant;

/// Frame clock plus a fixed-timestep accumulator.
///
/// `delta` is clamped so that a paused or minimized window cannot produce a
/// giant simulation step when it comes back.
pub struct Time {
    last: Instant,
    delta: f32,
    elapsed: f32,
    fixed_delta: f32,
    accumulator: f32,
    max_delta: f32,
}

#[allow(dead_code)] // timing helpers for later milestones (interpolation, profiling)
impl Time {
    pub fn new(fixed_hz: f32) -> Self {
        Self {
            last: Instant::now(),
            delta: 0.0,
            elapsed: 0.0,
            fixed_delta: 1.0 / fixed_hz,
            accumulator: 0.0,
            max_delta: 0.1,
        }
    }

    /// Advance the clock and return the (clamped) frame delta in seconds.
    pub fn tick(&mut self) -> f32 {
        let now = Instant::now();
        let raw = (now - self.last).as_secs_f32();
        self.last = now;

        self.delta = raw.min(self.max_delta);
        self.elapsed += self.delta;
        self.accumulator += self.delta;
        self.delta
    }

    /// Pop one fixed step if enough time accumulated. Run simulation in this loop.
    pub fn next_fixed_step(&mut self) -> Option<f32> {
        if self.accumulator >= self.fixed_delta {
            self.accumulator -= self.fixed_delta;
            Some(self.fixed_delta)
        } else {
            None
        }
    }

    /// Remaining fraction of a fixed step, for render interpolation.
    pub fn alpha(&self) -> f32 {
        self.accumulator / self.fixed_delta
    }

    pub fn delta(&self) -> f32 {
        self.delta
    }

    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }

    pub fn fixed_delta(&self) -> f32 {
        self.fixed_delta
    }

    /// Instantaneous frames per second, based on the last frame delta.
    pub fn fps(&self) -> f32 {
        if self.delta > 0.0 { 1.0 / self.delta } else { 0.0 }
    }
}

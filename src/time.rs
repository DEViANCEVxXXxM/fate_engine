use std::time::Instant;

pub const TARGET_FPS: f32 = 120.0;

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
        let safe_hz = fixed_hz.max(1.0);
        Self {
            last: Instant::now(),
            delta: 0.0,
            elapsed: 0.0,
            fixed_delta: 1.0 / safe_hz,
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
        if self.fixed_delta <= 0.0 {
            0.0
        } else {
            self.accumulator / self.fixed_delta
        }
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
        if self.delta > 0.0 {
            1.0 / self.delta
        } else {
            0.0
        }
    }

    /// The ideal sleep budget between redraws for a stable 120 FPS target.
    pub fn frame_budget_ms() -> u64 {
        ((1000.0 / TARGET_FPS).floor() as u64).max(1)
    }

    /// Wall-clock frame time in milliseconds for the latest tick.
    pub fn frame_time_ms(&self) -> f32 {
        self.delta * 1000.0
    }

    /// Remaining margin within the target frame budget.
    pub fn budget_headroom_ms(&self) -> f32 {
        Self::frame_budget_ms() as f32 - self.frame_time_ms()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{TARGET_FPS, Time};

    #[test]
    fn zero_or_negative_fixed_hz_is_clamped() {
        let clock = Time::new(0.0);
        assert!(clock.fixed_delta() > 0.0);
        assert_eq!(clock.alpha(), 0.0);
    }

    #[test]
    fn large_deltas_are_clamped() {
        let mut clock = Time::new(60.0);
        clock.last = Instant::now() - Duration::from_secs(2);

        let delta = clock.tick();

        assert!(delta <= 0.1);
        assert!(delta > 0.0);
    }

    #[test]
    fn fixed_step_accumulator_progresses_without_overflow() {
        let mut clock = Time::new(60.0);
        clock.accumulator = 0.25;

        let step = clock.next_fixed_step();

        assert_eq!(step, Some(1.0 / 60.0));
        assert!(clock.accumulator > 0.0);
        assert!(clock.accumulator < 0.25);
    }

    #[test]
    fn target_frame_budget_matches_120_fps_policy() {
        assert_eq!(Time::frame_budget_ms(), 8);
        assert!((TARGET_FPS - 120.0).abs() < 1e-6);
    }

    #[test]
    fn budget_headroom_tracks_latest_frame_time() {
        let mut clock = Time::new(120.0);
        clock.delta = 0.016;

        let headroom = clock.budget_headroom_ms();
        assert!((headroom - 8.0 + 16.0).abs() < 1e-5);
        assert!((clock.frame_time_ms() - 16.0).abs() < 1e-5);
    }
}

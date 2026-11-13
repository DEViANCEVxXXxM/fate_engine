use std::time::Instant;

pub const TARGET_FPS: f32 = 120.0;
pub const MAX_FIXED_STEPS_PER_FRAME: usize = 8;

/// Maximum wall-clock delta applied to presentation/frame work, in seconds.
///
/// The fixed-step accumulator still receives the unclamped delta, so this only
/// bounds per-frame presentation work and never discards simulation time.
pub const MAX_FRAME_DELTA_SECONDS: f32 = 0.1;

/// Frame clock plus a fixed-timestep accumulator.
///
/// `delta` is clamped to [`MAX_FRAME_DELTA_SECONDS`] so that a paused or
/// minimized window cannot produce a giant presentation step when it comes
/// back. The fixed accumulator receives the unclamped wall-clock delta.
pub struct Time {
    last: Instant,
    delta: f32,
    elapsed: f32,
    fixed_delta: f64,
    accumulator: f64,
    steps_this_frame: usize,
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
            fixed_delta: 1.0 / safe_hz as f64,
            accumulator: 0.0,
            steps_this_frame: 0,
            max_delta: MAX_FRAME_DELTA_SECONDS,
        }
    }

    /// Advance the clock and return a clamped delta for frame/presentation work.
    ///
    /// `delta` is limited to [`MAX_FRAME_DELTA_SECONDS`]. The fixed-step
    /// accumulator receives the full wall-clock delta, so excess simulation
    /// time is retained as backlog and processed in later frames rather than
    /// discarded.
    pub fn tick(&mut self) -> f32 {
        let now = Instant::now();
        let raw = (now - self.last).as_secs_f64();
        self.last = now;

        self.steps_this_frame = 0;
        self.delta = (raw as f32).min(self.max_delta);
        self.elapsed += raw as f32;
        self.accumulator += raw;
        self.delta
    }

    /// Pop one fixed step, up to the per-frame safety limit.
    ///
    /// When the limit is reached, pending steps remain in the accumulator and
    /// can be consumed after the next call to `tick()`.
    pub fn next_fixed_step(&mut self) -> Option<f32> {
        if self.accumulator >= self.fixed_delta && self.steps_this_frame < MAX_FIXED_STEPS_PER_FRAME
        {
            self.accumulator -= self.fixed_delta;
            self.steps_this_frame += 1;
            Some(self.fixed_delta as f32)
        } else {
            None
        }
    }

    /// Fractional remainder for interpolation; pending whole ticks are excluded.
    pub fn alpha(&self) -> f32 {
        if self.fixed_delta <= 0.0 {
            0.0
        } else {
            (self.accumulator % self.fixed_delta / self.fixed_delta) as f32
        }
    }

    /// Whether whole fixed steps remain queued after the per-frame step limit.
    pub fn is_catching_up(&self) -> bool {
        self.accumulator >= self.fixed_delta
    }

    /// Number of complete fixed steps still queued after this frame's work.
    pub fn pending_fixed_steps(&self) -> u64 {
        (self.accumulator / self.fixed_delta).floor() as u64
    }

    pub fn delta(&self) -> f32 {
        self.delta
    }

    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }

    pub fn fixed_delta(&self) -> f32 {
        self.fixed_delta as f32
    }

    /// Instantaneous frames per second, based on the last frame delta.
    pub fn fps(&self) -> f32 {
        if self.delta > 0.0 {
            1.0 / self.delta
        } else {
            0.0
        }
    }

    /// Exact target frame period in milliseconds (approximately 8.333 ms at
    /// 120 FPS).
    ///
    /// Use this for analysis and reporting. Scheduling and budget checks use
    /// [`Self::frame_budget_ms`], which is deliberately conservative.
    pub fn frame_period_ms() -> f32 {
        1000.0 / TARGET_FPS
    }

    /// Conservative integer sleep/budget value for the 120 FPS target.
    ///
    /// This is the floor of [`Self::frame_period_ms`] (currently 8 ms). It is
    /// not the exact tick period and does not guarantee 120 presented frames per
    /// second.
    pub fn frame_budget_ms() -> u64 {
        (Self::frame_period_ms().floor() as u64).max(1)
    }

    /// Wall-clock frame time in milliseconds for the latest tick.
    pub fn frame_time_ms(&self) -> f32 {
        self.delta * 1000.0
    }

    /// Remaining margin within the conservative integer frame budget.
    ///
    /// Uses [`Self::frame_budget_ms`], not the exact 120 Hz period. A positive
    /// value means the latest clamped frame delta fits the scheduling budget; it
    /// is not a GPU-completion measurement.
    pub fn budget_headroom_ms(&self) -> f32 {
        Self::frame_budget_ms() as f32 - self.frame_time_ms()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{MAX_FIXED_STEPS_PER_FRAME, MAX_FRAME_DELTA_SECONDS, TARGET_FPS, Time};

    #[test]
    fn zero_or_negative_fixed_hz_is_clamped() {
        let clock = Time::new(0.0);
        assert!(clock.fixed_delta() > 0.0);
        assert_eq!(clock.alpha(), 0.0);
    }

    #[test]
    fn large_deltas_keep_simulation_time_and_bound_steps_per_frame() {
        let mut clock = Time::new(60.0);
        clock.last = Instant::now() - Duration::from_secs(2);

        let delta = clock.tick();

        assert_eq!(delta, 0.1);
        assert!(clock.is_catching_up());
        let mut steps = 0;
        while clock.next_fixed_step().is_some() {
            steps += 1;
        }
        assert_eq!(steps, MAX_FIXED_STEPS_PER_FRAME);
        assert!(clock.is_catching_up());
        assert!(clock.pending_fixed_steps() >= 100);

        clock.tick();
        let mut next_frame_steps = 0;
        while clock.next_fixed_step().is_some() {
            next_frame_steps += 1;
        }
        assert_eq!(next_frame_steps, MAX_FIXED_STEPS_PER_FRAME);
        assert!(clock.is_catching_up());
    }

    #[test]
    fn interpolation_alpha_stays_within_one_step_during_catch_up() {
        let mut clock = Time::new(60.0);
        clock.accumulator = clock.fixed_delta * 10.5;

        for _ in 0..MAX_FIXED_STEPS_PER_FRAME {
            assert!(clock.next_fixed_step().is_some());
        }

        assert!(clock.is_catching_up());
        assert!((clock.alpha() - 0.5).abs() < 1e-5);
        assert!(clock.alpha() < 1.0);
    }

    #[test]
    fn next_fixed_step_stops_at_per_frame_limit_without_discarding_backlog() {
        let mut clock = Time::new(60.0);
        clock.accumulator = clock.fixed_delta * 9.0;

        for _ in 0..MAX_FIXED_STEPS_PER_FRAME {
            assert!(clock.next_fixed_step().is_some());
        }
        assert!(clock.next_fixed_step().is_none());
        assert!(clock.is_catching_up());
        assert_eq!(clock.pending_fixed_steps(), 1);
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
        assert!((Time::frame_period_ms() - (1000.0 / 120.0)).abs() < 1e-5);
        assert!(Time::frame_period_ms() > Time::frame_budget_ms() as f32);
    }

    #[test]
    fn presentation_delta_is_clamped_while_simulation_backlog_is_retained() {
        let mut clock = Time::new(120.0);
        clock.last = Instant::now() - Duration::from_secs(1);

        let delta = clock.tick();

        assert!((delta - MAX_FRAME_DELTA_SECONDS).abs() < 1e-6);
        assert!(clock.elapsed() >= 1.0);
        assert!(clock.is_catching_up());
        assert!(clock.pending_fixed_steps() >= 100);
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

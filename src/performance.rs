use crate::time::Time;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FrameProfile {
    pub(crate) sim_ms: f32,
    pub(crate) render_ms: f32,
    pub(crate) drawn_instances: usize,
}

impl FrameProfile {
    pub(crate) fn total_ms(self) -> f32 {
        self.sim_ms + self.render_ms
    }

    pub(crate) fn over_budget(self) -> bool {
        self.total_ms() > Time::frame_budget_ms() as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameBudgetPolicy {
    Healthy,
    Warning,
    Critical,
}

impl FrameBudgetPolicy {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }

    pub(crate) fn for_frame(total_ms: f32, draw_pressure: f32) -> Self {
        let target_ms = Time::frame_budget_ms() as f32;

        if total_ms <= target_ms && draw_pressure < 0.75 {
            Self::Healthy
        } else if total_ms <= target_ms * 1.25 && draw_pressure < 0.9 {
            Self::Warning
        } else {
            Self::Critical
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PerformanceStats {
    pub(crate) recent_frame_times: [f32; 60],
    pub(crate) recent_count: usize,
    pub(crate) healthy_frames: usize,
    pub(crate) warning_frames: usize,
    pub(crate) critical_frames: usize,
    pub(crate) peak_draw_pressure: f32,
}

impl Default for PerformanceStats {
    fn default() -> Self {
        Self {
            recent_frame_times: [0.0; 60],
            recent_count: 0,
            healthy_frames: 0,
            warning_frames: 0,
            critical_frames: 0,
            peak_draw_pressure: 0.0,
        }
    }
}

impl PerformanceStats {
    pub(crate) fn add(&mut self, frame_ms: f32) {
        self.recent_frame_times[self.recent_count % self.recent_frame_times.len()] = frame_ms;
        self.recent_count = (self.recent_count + 1).min(self.recent_frame_times.len());
    }

    pub(crate) fn record_budget(
        &mut self,
        frame_ms: f32,
        policy: FrameBudgetPolicy,
        draw_pressure: f32,
    ) {
        self.add(frame_ms);
        self.peak_draw_pressure = self.peak_draw_pressure.max(draw_pressure);

        match policy {
            FrameBudgetPolicy::Healthy => self.healthy_frames += 1,
            FrameBudgetPolicy::Warning => self.warning_frames += 1,
            FrameBudgetPolicy::Critical => self.critical_frames += 1,
        }
    }

    pub(crate) fn average_ms(&self) -> f32 {
        if self.recent_count == 0 {
            return 0.0;
        }

        let count = self.recent_count.min(self.recent_frame_times.len());
        let sum: f32 = self.recent_frame_times.iter().take(count).sum();
        sum / count as f32
    }

    pub(crate) fn worst_ms(&self) -> f32 {
        if self.recent_count == 0 {
            return 0.0;
        }

        let count = self.recent_count.min(self.recent_frame_times.len());
        self.recent_frame_times
            .iter()
            .take(count)
            .fold(0.0, |max, value| max.max(*value))
    }

    pub(crate) fn snapshot(&self) -> (f32, f32) {
        (self.average_ms(), self.worst_ms())
    }

    pub(crate) fn budget_breakdown(&self) -> (usize, usize, usize) {
        (
            self.healthy_frames,
            self.warning_frames,
            self.critical_frames,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn peak_draw_pressure_pct(&self) -> f32 {
        self.peak_draw_pressure * 100.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FrameTelemetry {
    pub(crate) sim_ms: f32,
    pub(crate) render_ms: f32,
    pub(crate) total_ms: f32,
    pub(crate) avg_ms: f32,
    pub(crate) worst_ms: f32,
    pub(crate) target_ms: f32,
    pub(crate) headroom_ms: f32,
    pub(crate) draw_count: usize,
    pub(crate) draw_limit: usize,
    pub(crate) draw_pressure_pct: f32,
    pub(crate) healthy_frames: usize,
    pub(crate) warning_frames: usize,
    pub(crate) critical_frames: usize,
    pub(crate) policy: FrameBudgetPolicy,
}

impl FrameTelemetry {
    pub(crate) fn summary(&self) -> String {
        format!(
            "sim={:.1}ms render={:.1}ms drawn={}/{} load={:.1}% total={:.1}ms avg={:.1}ms worst={:.1}ms target={:.1}ms headroom={:.1}ms budget=healthy:{}/warning:{}/critical:{} tier={}",
            self.sim_ms,
            self.render_ms,
            self.draw_count,
            self.draw_limit,
            self.draw_pressure_pct,
            self.total_ms,
            self.avg_ms,
            self.worst_ms,
            self.target_ms,
            self.headroom_ms,
            self.healthy_frames,
            self.warning_frames,
            self.critical_frames,
            self.policy.label(),
        )
    }
}

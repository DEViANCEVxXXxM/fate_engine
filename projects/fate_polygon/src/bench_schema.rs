use crate::settings::Settings;

pub const META: &[&str] = &["case", "preset", "sweep_setting", "sweep_value"];
pub const CPU_PREFIX: &[&str] = &["warmup_ticks", "measured_ticks", "physics", "ai"];
pub const CPU_TIMES: &[&str] = &[
    "tick_p50_ms",
    "tick_p95_ms",
    "physics_p50_ms",
    "physics_p95_ms",
    "ai_p50_ms",
    "ai_p95_ms",
];
pub const CPU_WORK: &[&str] = &[
    "pair_tests",
    "contacts",
    "decisions",
    "neighbor_tests",
    "route_searches",
    "route_cell_visits",
    "route_moves",
    "state_hash",
];
const ECS_TIMES: &[&str] = &[
    "ecs_p50_ms",
    "ecs_p95_ms",
    "snapshot_p50_ms",
    "snapshot_p95_ms",
];
const ECS_WORK: &[&str] = &[
    "ecs_spawned",
    "ecs_despawned",
    "ecs_updates",
    "snapshots",
    "snapshot_bytes",
];
pub const GPU_INPUTS: &[&str] = &[
    "width",
    "height",
    "adapter",
    "backend",
    "driver",
    "warmup_frames",
    "measured_frames",
    "candidates",
    "detail",
    "batch",
    "cull",
];
pub const GPU_WORK: &[&str] = &[
    "after_cull",
    "instances",
    "draw_calls",
    "submitted_triangles",
];
pub const GPU_TIMES: &[&str] = &[
    "cpu_prepare_p50_ms",
    "cpu_prepare_p95_ms",
    "cpu_submit_p50_ms",
    "cpu_submit_p95_ms",
    "gpu_scene_p50_ms",
    "gpu_scene_p95_ms",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Cpu,
    Ecs,
    Offscreen,
}

impl Kind {
    pub fn cpu_numbers(self) -> impl Iterator<Item = &'static str> {
        Settings::NUMBERS
            .iter()
            .filter(move |spec| {
                spec.cpu && (self == Self::Ecs || !Settings::ECS_NUMBERS.contains(&spec.name))
            })
            .map(|spec| spec.name)
    }

    pub fn inputs(self) -> Vec<&'static str> {
        match self {
            Self::Cpu | Self::Ecs => {
                let mut inputs: Vec<_> = CPU_PREFIX
                    .iter()
                    .copied()
                    .chain(self.cpu_numbers())
                    .collect();
                if self == Self::Ecs {
                    inputs.push("ecs");
                }
                inputs
            }
            Self::Offscreen => GPU_INPUTS.to_vec(),
        }
    }

    pub fn timings(self) -> Vec<&'static str> {
        match self {
            Self::Cpu => CPU_TIMES.to_vec(),
            Self::Ecs => [CPU_TIMES, ECS_TIMES].concat(),
            Self::Offscreen => GPU_TIMES.to_vec(),
        }
    }

    pub fn work(self) -> Vec<&'static str> {
        match self {
            Self::Cpu => CPU_WORK.to_vec(),
            Self::Ecs => [CPU_WORK, ECS_WORK].concat(),
            Self::Offscreen => GPU_WORK.to_vec(),
        }
    }

    pub fn columns(self) -> Vec<&'static str> {
        let mut columns = META.to_vec();
        columns.extend(self.inputs());
        match self {
            Self::Cpu | Self::Ecs => {
                columns.extend(self.timings());
                columns.extend(self.work());
            }
            Self::Offscreen => {
                columns.extend(GPU_WORK);
                columns.extend(GPU_TIMES);
                columns.push("gpu_samples");
            }
        }
        columns
    }
}

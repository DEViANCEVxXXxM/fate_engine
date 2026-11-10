use std::time::Instant;

use fate_sim::ecs::{Position, Rotation, Spin, Velocity};
use fate_sim::math::{Fixed, FixedVec2};
use fate_sim::simulation::{EngineCore, SIMULATION_HZ, SimulationInput};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BenchmarkError {
    ZeroTicks,
}

impl fmt::Display for BenchmarkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroTicks => {
                formatter.write_str("benchmark tick count must be greater than zero")
            }
        }
    }
}

impl std::error::Error for BenchmarkError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BenchmarkScenario {
    Idle,
    Movement,
    EntityLoad { entities: usize },
}

impl BenchmarkScenario {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Movement => "movement",
            Self::EntityLoad { .. } => "entity_load",
        }
    }

    fn setup(self, core: &mut EngineCore) -> SimulationInput {
        match self {
            Self::Idle => SimulationInput::default(),
            Self::Movement => SimulationInput {
                move_x: 1,
                move_y: 1,
                boost: false,
            },
            Self::EntityLoad { entities } => {
                for index in 0..entities {
                    let entity = core.world.spawn();
                    core.world.positions.insert(
                        entity,
                        Position(FixedVec2::new(
                            Fixed::from_ratio(index as i64, 4),
                            Fixed::from_ratio((index % 16) as i64, 4),
                        )),
                    );
                    core.world.velocities.insert(
                        entity,
                        Velocity(FixedVec2::new(Fixed::from_int(2), Fixed::from_ratio(3, 2))),
                    );
                    core.world
                        .rotations
                        .insert(entity, Rotation(Fixed::from_ratio(index as i64, 10)));
                    core.world
                        .spins
                        .insert(entity, Spin(Fixed::from_ratio(1, 2)));
                }
                SimulationInput {
                    move_x: -1,
                    move_y: 0,
                    boost: true,
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BudgetBenchmarkReport {
    pub scenario: String,
    pub entities: usize,
    pub ticks: usize,
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub worst_ms: f64,
    pub target_ms: f64,
    pub passes_budget: bool,
    pub final_state_hash: u64,
}

impl BudgetBenchmarkReport {
    pub fn summary(&self) -> String {
        format!(
            "scope=simulation_only scenario={} entities={} ticks={} avg={:.4}ms p50={:.4}ms p95={:.4}ms p99={:.4}ms worst={:.4}ms target={:.4}ms passes_budget={} state_hash={:016x}",
            self.scenario,
            self.entities,
            self.ticks,
            self.avg_ms,
            self.p50_ms,
            self.p95_ms,
            self.p99_ms,
            self.worst_ms,
            self.target_ms,
            self.passes_budget,
            self.final_state_hash,
        )
    }
}

pub fn benchmark_scenario(
    scenario: BenchmarkScenario,
    ticks: usize,
) -> Result<BudgetBenchmarkReport, BenchmarkError> {
    if ticks == 0 {
        return Err(BenchmarkError::ZeroTicks);
    }
    let mut core = EngineCore::new();
    let input = scenario.setup(&mut core);
    let warmup_ticks = ticks.min(100);
    for _ in 0..warmup_ticks {
        core.fixed_update(input);
    }

    let mut samples_ms = Vec::with_capacity(ticks);
    let mut total_ms = 0.0_f64;
    let target_ms = 1000.0 / SIMULATION_HZ as f64;

    for _ in 0..ticks {
        let start = Instant::now();
        core.fixed_update(input);
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        total_ms += elapsed_ms;
        samples_ms.push(elapsed_ms);
    }

    let measured_ticks = ticks;
    samples_ms.sort_by(f64::total_cmp);
    let percentile = |percentage: usize| {
        let rank = samples_ms.len().saturating_mul(percentage).div_ceil(100);
        samples_ms[rank.saturating_sub(1)]
    };
    let worst_ms = samples_ms.last().copied().unwrap_or_default();
    let passes_budget = percentile(99) <= target_ms;
    Ok(BudgetBenchmarkReport {
        scenario: scenario.label().to_owned(),
        entities: match scenario {
            BenchmarkScenario::EntityLoad { entities } => entities + 1,
            _ => 1,
        },
        ticks: measured_ticks,
        avg_ms: total_ms / measured_ticks as f64,
        p50_ms: percentile(50),
        p95_ms: percentile(95),
        p99_ms: percentile(99),
        worst_ms,
        target_ms,
        passes_budget,
        final_state_hash: core.state_hash(),
    })
}

pub fn benchmark_scenarios(ticks: usize) -> Result<Vec<BudgetBenchmarkReport>, BenchmarkError> {
    let scenarios = [
        BenchmarkScenario::Idle,
        BenchmarkScenario::Movement,
        BenchmarkScenario::EntityLoad { entities: 64 },
        BenchmarkScenario::EntityLoad { entities: 256 },
        BenchmarkScenario::EntityLoad { entities: 1024 },
    ];
    scenarios
        .into_iter()
        .map(|scenario| benchmark_scenario(scenario, ticks))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_simulation_scope_and_fixed_rate_target() {
        let report = benchmark_scenario(BenchmarkScenario::Idle, 8).unwrap();

        assert_eq!(report.ticks, 8);
        assert_eq!(report.scenario, "idle");
        assert_eq!(report.entities, 1);
        assert!(report.avg_ms >= 0.0);
        assert!(report.p50_ms <= report.p95_ms);
        assert!(report.p95_ms <= report.p99_ms);
        assert!(report.p99_ms <= report.worst_ms);
        assert!(report.worst_ms >= 0.0);
        assert_eq!(report.target_ms, 1000.0 / SIMULATION_HZ as f64);
        assert!(report.final_state_hash != 0);
        assert!(report.summary().contains("scope=simulation_only"));
        assert!(report.summary().contains("ticks=8"));
    }

    #[test]
    fn scenarios_cover_idle_movement_and_entity_load() {
        let reports = benchmark_scenarios(4).unwrap();

        assert_eq!(reports.len(), 5);
        assert_eq!(reports[0].scenario, "idle");
        assert_eq!(reports[1].scenario, "movement");
        assert_eq!(reports[2].entities, 65);
        assert_eq!(reports[3].entities, 257);
        assert_eq!(reports[4].entities, 1025);
    }

    #[test]
    fn zero_measured_ticks_are_rejected() {
        assert_eq!(
            benchmark_scenario(BenchmarkScenario::Idle, 0),
            Err(BenchmarkError::ZeroTicks)
        );
    }
}

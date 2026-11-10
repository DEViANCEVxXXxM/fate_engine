use std::time::Instant;

use crate::ecs::{Position, Rotation, Spin, Velocity};
use crate::math::{Fixed, FixedVec2};
use crate::simulation::{EngineCore, SIMULATION_HZ, SimulationInput};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BenchmarkScenario {
    Idle,
    Movement,
    EntityLoad,
}

impl BenchmarkScenario {
    fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Movement => "movement",
            Self::EntityLoad => "entity_load",
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
            Self::EntityLoad => {
                for index in 0..64 {
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BudgetBenchmarkReport {
    pub scenario: &'static str,
    pub ticks: usize,
    pub avg_ms: f32,
    pub worst_ms: f32,
    pub target_ms: f32,
    pub passes_budget: bool,
}

impl BudgetBenchmarkReport {
    pub fn summary(&self) -> String {
        format!(
            "scenario={} ticks={} avg={:.2}ms worst={:.2}ms target={:.2}ms passes_budget={}",
            self.scenario,
            self.ticks,
            self.avg_ms,
            self.worst_ms,
            self.target_ms,
            self.passes_budget,
        )
    }
}

pub fn benchmark_scenario(scenario: BenchmarkScenario, ticks: usize) -> BudgetBenchmarkReport {
    let mut core = EngineCore::new();
    let input = scenario.setup(&mut core);
    let mut total_ms = 0.0f32;
    let mut worst_ms = 0.0f32;
    let target_ms = (1000.0 / SIMULATION_HZ as f32).floor();
    let mut passes_budget = true;

    for _ in 0..ticks {
        let start = Instant::now();
        core.fixed_update(input);
        let elapsed_ms = start.elapsed().as_secs_f32() * 1000.0;
        total_ms += elapsed_ms;
        worst_ms = worst_ms.max(elapsed_ms);
        passes_budget &= elapsed_ms <= target_ms;
    }

    let measured_ticks = ticks.max(1);
    BudgetBenchmarkReport {
        scenario: scenario.label(),
        ticks: measured_ticks,
        avg_ms: total_ms / measured_ticks as f32,
        worst_ms,
        target_ms,
        passes_budget,
    }
}

pub fn benchmark_scenarios(ticks: usize) -> Vec<BudgetBenchmarkReport> {
    [
        BenchmarkScenario::Idle,
        BenchmarkScenario::Movement,
        BenchmarkScenario::EntityLoad,
    ]
    .into_iter()
    .map(|scenario| benchmark_scenario(scenario, ticks))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_reports_simulation_tick_budget() {
        let report = benchmark_scenario(BenchmarkScenario::Idle, 8);

        assert_eq!(report.ticks, 8);
        assert_eq!(report.scenario, "idle");
        assert!(report.avg_ms >= 0.0);
        assert!(report.worst_ms >= 0.0);
        assert_eq!(report.target_ms, (1000.0 / SIMULATION_HZ as f32).floor());
        assert!(report.summary().contains("scenario=idle"));
        assert!(report.summary().contains("ticks=8"));
    }

    #[test]
    fn benchmark_scenarios_cover_idle_movement_and_entity_load() {
        let reports = benchmark_scenarios(4);

        assert_eq!(reports.len(), 3);
        assert_eq!(reports[0].scenario, "idle");
        assert_eq!(reports[1].scenario, "movement");
        assert_eq!(reports[2].scenario, "entity_load");
    }
}

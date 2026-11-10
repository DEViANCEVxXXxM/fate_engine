use std::hash::Hash;

use fate_engine::input::Input;
use fate_sim::ecs::{Position, Velocity, movement_system};
use fate_sim::math::{Fixed, FixedVec2};
use fate_sim::simulation::SimulationState;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum GameAction {
    MoveRight,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum GameButton {
    Right,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input: Input<GameAction, GameButton> = Input::new();
    input.bind(GameAction::MoveRight, GameButton::Right);
    input.press(GameButton::Right);

    let mut simulation = SimulationState::with_seed(0x534d_4f4b_4554_4553);
    let player = simulation.world.spawn();
    simulation
        .world
        .positions
        .insert(player, Position(FixedVec2::ZERO));
    simulation.world.velocities.insert(
        player,
        Velocity(FixedVec2::new(
            Fixed::from_int(i64::from(input.action_down(GameAction::MoveRight))),
            Fixed::ZERO,
        )),
    );
    movement_system(&mut simulation.world, Fixed::from_ratio(1, 120));
    simulation.advance_tick()?;
    input.end_frame();

    let snapshot = simulation.state_bytes();
    let restored = SimulationState::from_state_bytes(&snapshot)?;
    assert_eq!(restored.tick(), 1);
    assert_eq!(restored.state_bytes(), snapshot);

    println!(
        "consumer smoke test passed at simulation tick {}",
        restored.tick()
    );
    Ok(())
}

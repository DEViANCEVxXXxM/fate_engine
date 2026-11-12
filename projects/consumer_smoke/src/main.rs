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
    assert!(input.action_just_pressed(GameAction::MoveRight));

    let mut simulation = SimulationState::with_seed(0x534d_4f4b_4554_4553);
    let player = simulation.world.spawn();
    simulation
        .world
        .set_position(player, Position(FixedVec2::ZERO))?;
    simulation.world.set_velocity(
        player,
        Velocity(FixedVec2::new(
            Fixed::from_int(i64::from(input.action_down(GameAction::MoveRight))),
            Fixed::ZERO,
        )),
    )?;
    movement_system(&mut simulation.world, Fixed::from_ratio(1, 120));
    simulation.advance_tick()?;
    input.end_frame();
    assert!(input.action_down(GameAction::MoveRight));
    assert!(!input.action_just_pressed(GameAction::MoveRight));
    input.release(GameButton::Right);
    assert!(input.action_just_released(GameAction::MoveRight));

    simulation.world.velocity_mut(player).unwrap().0 = FixedVec2::new(
        Fixed::from_int(i64::from(input.action_down(GameAction::MoveRight))),
        Fixed::ZERO,
    );
    movement_system(&mut simulation.world, Fixed::from_ratio(1, 120));
    simulation.advance_tick()?;

    let _ = simulation.random_u64();
    let snapshot = simulation.state_bytes();
    let expected_next_random = simulation.random_u64();
    let mut restored = SimulationState::from_state_bytes(&snapshot)?;
    assert_eq!(restored.tick(), 2);
    assert_eq!(restored.state_bytes(), snapshot);
    assert_eq!(restored.random_u64(), expected_next_random);
    assert!(restored.world.is_alive(player));
    assert_eq!(restored.world.entity_count(), 1);
    assert_eq!(
        restored.world.positions().get(player).unwrap().0,
        FixedVec2::new(Fixed::from_ratio(1, 120), Fixed::ZERO)
    );

    restored.world.despawn(player);
    let despawned = SimulationState::from_state_bytes(&restored.state_bytes())?;
    assert!(!despawned.world.is_alive(player));
    assert_eq!(despawned.world.entity_count(), 0);
    assert!(despawned.world.positions().get(player).is_none());

    println!(
        "consumer smoke test passed at simulation tick {}",
        despawned.tick()
    );
    Ok(())
}

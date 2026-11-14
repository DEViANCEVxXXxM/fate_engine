//! Render-only interpolation between the previous and current fixed states.

use fate_engine::math::Vec3;
use fate_sim::math::FixedVec2;

use crate::sim::{ENEMY_CENTER_Y, PLAYER_EYE_HEIGHT, Sim};

/// Tracks the last two fixed-state positions so rendering can blend them.
///
/// This state is intentionally not part of the simulation snapshot: it affects
/// presentation only and is rebuilt on restart.
pub struct Interpolation {
    previous_player: FixedVec2,
    current_player: FixedVec2,
    previous_enemies: Vec<FixedVec2>,
    current_enemies: Vec<FixedVec2>,
}

impl Interpolation {
    pub fn new(sim: &Sim) -> Self {
        let positions: Vec<FixedVec2> = sim.enemies.iter().map(|enemy| enemy.position).collect();
        Self {
            previous_player: sim.player.position,
            current_player: sim.player.position,
            previous_enemies: positions.clone(),
            current_enemies: positions,
        }
    }

    /// Shift the current positions into the previous slot before a fixed step.
    pub fn begin_step(&mut self) {
        self.previous_player = self.current_player;
        self.previous_enemies.clone_from(&self.current_enemies);
    }

    /// Capture the positions produced by the completed fixed step.
    pub fn end_step(&mut self, sim: &Sim) {
        if sim.enemies.len() != self.current_enemies.len() {
            *self = Self::new(sim);
            return;
        }
        self.current_player = sim.player.position;
        for (index, enemy) in sim.enemies.iter().enumerate() {
            self.current_enemies[index] = enemy.position;
        }
    }

    /// Interpolated eye position for the first-person camera.
    pub fn player_position(&self, alpha: f32) -> Vec3 {
        let alpha = sanitize_alpha(alpha);
        let horizontal = lerp(self.previous_player, self.current_player, alpha);
        Vec3::new(horizontal.x, PLAYER_EYE_HEIGHT.to_f32(), horizontal.z)
    }

    /// Interpolated enemy center for rendering.
    pub fn enemy_position(&self, index: usize, alpha: f32) -> Vec3 {
        let alpha = sanitize_alpha(alpha);
        let previous = self
            .previous_enemies
            .get(index)
            .copied()
            .unwrap_or(FixedVec2::ZERO);
        let current = self.current_enemies.get(index).copied().unwrap_or(previous);
        let horizontal = lerp(previous, current, alpha);
        Vec3::new(horizontal.x, ENEMY_CENTER_Y.to_f32(), horizontal.z)
    }
}

fn sanitize_alpha(alpha: f32) -> f32 {
    if alpha.is_finite() {
        alpha.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

fn lerp(previous: FixedVec2, current: FixedVec2, alpha: f32) -> Vec3 {
    let previous_x = previous.x.to_f32();
    let previous_z = previous.y.to_f32();
    let current_x = current.x.to_f32();
    let current_z = current.y.to_f32();
    Vec3::new(
        previous_x + (current_x - previous_x) * alpha,
        0.0,
        previous_z + (current_z - previous_z) * alpha,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_blends_between_previous_and_current_positions() {
        let mut sim = Sim::new();
        let mut interpolation = Interpolation::new(&sim);
        interpolation.begin_step();
        sim.step(crate::sim::SimInput {
            move_y: 1,
            ..crate::sim::SimInput::default()
        });
        interpolation.end_step(&sim);

        let start = interpolation.player_position(0.0);
        let end = interpolation.player_position(1.0);
        let middle = interpolation.player_position(0.5);

        assert!((start - end).length() > 1e-4);
        assert!(((start + end) * 0.5 - middle).length() < 1e-4);
    }

    #[test]
    fn invalid_alpha_falls_back_to_the_latest_state() {
        let mut sim = Sim::new();
        let mut interpolation = Interpolation::new(&sim);
        interpolation.begin_step();
        sim.step(crate::sim::SimInput {
            move_y: 1,
            ..crate::sim::SimInput::default()
        });
        interpolation.end_step(&sim);

        assert_eq!(
            interpolation.player_position(f32::NAN),
            interpolation.player_position(1.0)
        );
        assert_eq!(
            interpolation.enemy_position(0, f32::INFINITY),
            interpolation.enemy_position(0, 1.0)
        );
    }
}

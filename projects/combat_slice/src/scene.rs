//! Cube arena, dynamic slice objects, and the culled draw list.

use fate_engine::math::{Mat4, Vec3};
use fate_engine::renderer::{Camera3d, Draw3d, Mesh3d};

use crate::interpolation::Interpolation;
use crate::sim::{ENEMY_HEALTH, PLAYER_EYE_HEIGHT, Sim};

/// One cube to place in the slice scene.
#[derive(Clone, Copy, Debug)]
pub struct CubeInstance {
    pub position: Vec3,
    pub scale: Vec3,
    pub rotation: f32,
    pub tint: [f32; 3],
}

/// Static arena: floor and four walls around the playable bounds.
pub const ARENA: [CubeInstance; 5] = [
    // Floor.
    CubeInstance {
        position: Vec3::new(0.0, -0.5, 0.0),
        scale: Vec3::new(26.0, 1.0, 26.0),
        rotation: 0.0,
        tint: [0.07, 0.09, 0.12],
    },
    // Back wall.
    CubeInstance {
        position: Vec3::new(0.0, 3.0, -12.5),
        scale: Vec3::new(26.0, 6.0, 1.0),
        rotation: 0.0,
        tint: [0.15, 0.19, 0.26],
    },
    // Front wall.
    CubeInstance {
        position: Vec3::new(0.0, 3.0, 12.5),
        scale: Vec3::new(26.0, 6.0, 1.0),
        rotation: 0.0,
        tint: [0.15, 0.19, 0.26],
    },
    // Left wall.
    CubeInstance {
        position: Vec3::new(-12.5, 3.0, 0.0),
        scale: Vec3::new(1.0, 6.0, 26.0),
        rotation: 0.0,
        tint: [0.15, 0.19, 0.26],
    },
    // Right wall.
    CubeInstance {
        position: Vec3::new(12.5, 3.0, 0.0),
        scale: Vec3::new(1.0, 6.0, 26.0),
        rotation: 0.0,
        tint: [0.15, 0.19, 0.26],
    },
];

/// First-person camera matching the simulation's aim at an explicit eye point.
pub fn camera_at(position: Vec3, sim: &Sim) -> Camera3d {
    Camera3d {
        position,
        yaw: sim.player.look.yaw_angle(),
        pitch: sim.player.look.pitch_angle(),
        ..Camera3d::default()
    }
}

/// First-person camera at the simulation's current eye position.
pub fn camera_for(sim: &Sim) -> Camera3d {
    camera_at(
        Vec3::new(
            sim.player.position.x.to_f32(),
            PLAYER_EYE_HEIGHT.to_f32(),
            sim.player.position.y.to_f32(),
        ),
        sim,
    )
}

/// Build the world-space dynamic cubes: living enemies.
pub fn dynamic_instances(
    sim: &Sim,
    interpolation: &Interpolation,
    alpha: f32,
) -> Vec<CubeInstance> {
    let mut instances = Vec::with_capacity(sim.enemies.len());

    for (index, enemy) in sim.enemies.iter().enumerate() {
        if !enemy.alive() {
            continue;
        }
        let damaged = enemy.health < ENEMY_HEALTH;
        instances.push(CubeInstance {
            position: interpolation.enemy_position(index, alpha),
            scale: Vec3::new(1.2, 2.0, 1.2),
            rotation: 0.0,
            tint: if damaged {
                [1.0, 0.55, 0.2]
            } else {
                [0.82, 0.24, 0.2]
            },
        });
    }

    instances
}

/// A camera-locked viewmodel cube with a full model matrix.
///
/// Unlike [`CubeInstance`], this is not a world object: its matrix is rebuilt
/// from the camera basis every frame so it stays fixed relative to the view.
#[derive(Clone, Copy, Debug)]
pub struct ViewmodelInstance {
    pub model: Mat4,
    pub tint: [f32; 3],
}

const WEAPON_FORWARD: f32 = 0.70;
const WEAPON_RIGHT: f32 = 0.28;
const WEAPON_DOWN: f32 = 0.20;
const MUZZLE_FORWARD: f32 = 1.10;
const MUZZLE_RIGHT: f32 = 0.28;
const MUZZLE_DOWN: f32 = 0.18;

/// Camera forward, right, and up vectors, matching `Camera3d::forward`.
fn camera_basis(camera: &Camera3d) -> (Vec3, Vec3, Vec3) {
    let forward = camera.forward();
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward);
    (forward, right, up)
}

/// Model matrix for a cube at a fixed offset in camera space.
fn viewmodel_model(
    camera: &Camera3d,
    forward_offset: f32,
    right_offset: f32,
    down_offset: f32,
    scale: Vec3,
) -> Mat4 {
    let (forward, right, up) = camera_basis(camera);
    let position =
        camera.position + forward * forward_offset + right * right_offset + up * (-down_offset);
    let rotation = Mat4::from_cols_array([
        right.x, right.y, right.z, 0.0, //
        up.x, up.y, up.z, 0.0, //
        -forward.x, -forward.y, -forward.z, 0.0, //
        0.0, 0.0, 0.0, 1.0, //
    ]);
    Mat4::translation(position)
        .mul(rotation)
        .mul(Mat4::scale(scale))
}

/// Build the camera-locked weapon and its muzzle flash.
pub fn weapon_instances(camera: &Camera3d, sim: &Sim) -> Vec<ViewmodelInstance> {
    let mut instances = Vec::with_capacity(2);
    instances.push(ViewmodelInstance {
        model: viewmodel_model(
            camera,
            WEAPON_FORWARD,
            WEAPON_RIGHT,
            WEAPON_DOWN,
            Vec3::new(0.16, 0.18, 0.60),
        ),
        tint: [0.62, 0.66, 0.74],
    });

    if sim
        .last_shot_tick()
        .is_some_and(|last| sim.tick().saturating_sub(last) <= 3)
    {
        instances.push(ViewmodelInstance {
            model: viewmodel_model(
                camera,
                MUZZLE_FORWARD,
                MUZZLE_RIGHT,
                MUZZLE_DOWN,
                Vec3::new(0.16, 0.16, 0.16),
            ),
            tint: [1.0, 0.86, 0.36],
        });
    }

    instances
}

/// Build draws for the camera-locked viewmodel.
pub fn weapon_draws<'a>(mesh: &'a Mesh3d, camera: &Camera3d, sim: &Sim) -> Vec<Draw3d<'a>> {
    weapon_instances(camera, sim)
        .into_iter()
        .map(|instance| Draw3d {
            mesh,
            model: instance.model,
            tint: instance.tint,
        })
        .collect()
}

/// Indices of instances whose bounding sphere intersects the camera frustum.
///
/// The renderer does not cull automatically, so the slice owns this decision.
pub fn visible_indices(camera: &Camera3d, aspect: f32, instances: &[CubeInstance]) -> Vec<usize> {
    let frustum = camera.frustum(aspect);
    instances
        .iter()
        .enumerate()
        .filter_map(|(index, instance)| {
            let radius = instance.scale.length() * 0.5;
            frustum
                .intersects_sphere(instance.position, radius)
                .then_some(index)
        })
        .collect()
}

/// Build a draw list from the visible instances and return its visible count.
pub fn build_draws<'a>(
    mesh: &'a Mesh3d,
    camera: &Camera3d,
    aspect: f32,
    instances: &[CubeInstance],
) -> (Vec<Draw3d<'a>>, usize) {
    let visible = visible_indices(camera, aspect, instances);
    let mut draws = Vec::with_capacity(visible.len());
    for &index in &visible {
        let instance = &instances[index];
        let model = Mat4::translation(instance.position)
            .mul(Mat4::rotation_y(instance.rotation))
            .mul(Mat4::scale(instance.scale));
        draws.push(Draw3d {
            mesh,
            model,
            tint: instance.tint,
        });
    }
    (draws, visible.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::SimInput;

    fn cube(x: f32, y: f32, z: f32) -> CubeInstance {
        CubeInstance {
            position: Vec3::new(x, y, z),
            scale: Vec3::ONE,
            rotation: 0.0,
            tint: [1.0; 3],
        }
    }

    #[test]
    fn culling_keeps_in_front_and_drops_behind_and_side() {
        let camera = Camera3d::default();
        let instances = [
            cube(0.0, 1.6, 0.0),
            cube(0.0, 1.6, 20.0),
            cube(100.0, 1.6, 0.0),
        ];

        assert_eq!(visible_indices(&camera, 16.0 / 9.0, &instances), vec![0]);
    }

    #[test]
    fn arena_contains_ground_and_boundaries() {
        assert!(ARENA.len() >= 5);
        assert!(
            ARENA
                .iter()
                .any(|instance| instance.scale.x > 20.0 && instance.scale.z > 20.0)
        );
    }

    #[test]
    fn dynamic_instances_track_living_enemies() {
        let mut sim = Sim::new();
        let interpolation = Interpolation::new(&sim);
        let instances = dynamic_instances(&sim, &interpolation, 1.0);
        assert_eq!(instances.len(), sim.enemies.len());

        sim.enemies[0].health = 0;
        let instances = dynamic_instances(&sim, &interpolation, 1.0);
        assert_eq!(instances.len(), sim.enemies.len() - 1);
    }

    #[test]
    fn firing_adds_a_muzzle_flash_for_a_few_ticks() {
        let mut sim = Sim::new();
        let camera = camera_for(&sim);
        assert_eq!(weapon_instances(&camera, &sim).len(), 1);

        sim.step(SimInput {
            fire: true,
            ..SimInput::default()
        });

        let camera = camera_for(&sim);
        assert_eq!(weapon_instances(&camera, &sim).len(), 2);
    }

    fn viewmodel_offset(camera: &Camera3d, model: Mat4) -> Vec3 {
        let (forward, right, up) = camera_basis(camera);
        let cols = model.to_cols_array();
        let offset = Vec3::new(cols[12], cols[13], cols[14]) - camera.position;
        Vec3::new(offset.dot(right), offset.dot(up), offset.dot(forward))
    }

    #[test]
    fn weapon_stays_locked_to_the_view_when_looking_around() {
        let sim = Sim::new();
        let level = camera_for(&sim);
        let mut turned = camera_for(&sim);
        turned.yaw = 0.9;
        turned.pitch = -0.6;

        let level_weapon = weapon_instances(&level, &sim)[0];
        let turned_weapon = weapon_instances(&turned, &sim)[0];

        let level_offset = viewmodel_offset(&level, level_weapon.model);
        let turned_offset = viewmodel_offset(&turned, turned_weapon.model);
        assert!((level_offset - turned_offset).length() < 1e-4);

        let forward = turned.forward();
        let cols = turned_weapon.model.to_cols_array();
        let local_forward = Vec3::new(cols[8], cols[9], cols[10]).normalize();
        assert!((local_forward + forward).length() < 1e-4);
    }

    #[test]
    fn camera_matches_the_simulation_eye_and_aim() {
        let mut sim = Sim::new();
        sim.step(SimInput {
            look_yaw: 100,
            ..SimInput::default()
        });

        let camera = camera_for(&sim);
        assert!((camera.position.y - PLAYER_EYE_HEIGHT.to_f32()).abs() < 1e-6);
        assert!((camera.yaw - sim.player.look.yaw_angle()).abs() < 1e-6);
        assert!((camera.pitch - sim.player.look.pitch_angle()).abs() < 1e-6);
    }
}

//! Static arena geometry and the culled cube draw list for the showcase.

use fate_engine::math::{Mat4, Vec3};
use fate_engine::renderer::{Camera3d, Draw3d, Mesh3d};

/// One cube to place in the showcase scene.
#[derive(Clone, Copy, Debug)]
pub struct CubeInstance {
    pub position: Vec3,
    pub scale: Vec3,
    pub rotation: f32,
    pub tint: [f32; 3],
}

/// Static arena: floor, three walls, two pillars, and a low plinth.
pub const ARENA: [CubeInstance; 7] = [
    // Floor.
    CubeInstance {
        position: Vec3::new(0.0, -0.5, 0.0),
        scale: Vec3::new(26.0, 1.0, 24.0),
        rotation: 0.0,
        tint: [0.08, 0.10, 0.14],
    },
    // Back wall.
    CubeInstance {
        position: Vec3::new(0.0, 3.0, -12.0),
        scale: Vec3::new(26.0, 7.0, 0.6),
        rotation: 0.0,
        tint: [0.16, 0.20, 0.28],
    },
    // Left wall.
    CubeInstance {
        position: Vec3::new(-13.0, 3.0, 0.0),
        scale: Vec3::new(0.6, 7.0, 24.0),
        rotation: 0.0,
        tint: [0.16, 0.20, 0.28],
    },
    // Right wall.
    CubeInstance {
        position: Vec3::new(13.0, 3.0, 0.0),
        scale: Vec3::new(0.6, 7.0, 24.0),
        rotation: 0.0,
        tint: [0.16, 0.20, 0.28],
    },
    // Left pillar.
    CubeInstance {
        position: Vec3::new(-9.0, 3.0, -9.0),
        scale: Vec3::new(0.9, 7.0, 0.9),
        rotation: 0.0,
        tint: [0.22, 0.28, 0.38],
    },
    // Right pillar.
    CubeInstance {
        position: Vec3::new(9.0, 3.0, -9.0),
        scale: Vec3::new(0.9, 7.0, 0.9),
        rotation: 0.0,
        tint: [0.22, 0.28, 0.38],
    },
    // Central plinth.
    CubeInstance {
        position: Vec3::new(0.0, 0.1, 0.0),
        scale: Vec3::new(6.0, 0.2, 6.0),
        rotation: 0.0,
        tint: [0.18, 0.22, 0.30],
    },
];

/// Indices of instances whose bounding sphere intersects the camera frustum.
///
/// The renderer does not cull automatically, so the demo owns this decision.
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
    fn culling_is_deterministic() {
        let camera = Camera3d::default();
        let instances = [cube(0.0, 1.6, 0.0), cube(2.0, 1.6, -2.0)];

        let first = visible_indices(&camera, 1.0, &instances);
        let second = visible_indices(&camera, 1.0, &instances);

        assert_eq!(first, second);
    }

    #[test]
    fn arena_contains_ground_and_boundaries() {
        assert!(ARENA.len() >= 6);
        assert!(
            ARENA
                .iter()
                .any(|instance| instance.scale.x > 20.0 && instance.scale.z > 20.0)
        );
    }
}

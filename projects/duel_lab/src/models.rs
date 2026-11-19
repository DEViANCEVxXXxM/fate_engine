//! Prototype geometry owned by the project, not the renderer or simulation.

use fate_engine::math::Vec3;
use fate_engine::renderer::{Mesh3d, MeshVertex, Renderer};

pub struct Models {
    pub floor: Mesh3d,
    pub player: Mesh3d,
    pub opponent: Mesh3d,
    pub bolt: Mesh3d,
    pub shot: Mesh3d,
}

impl Models {
    pub fn upload(renderer: &Renderer) -> Self {
        let upload = |label, data: Geometry| {
            renderer
                .create_mesh(label, &data.vertices, &data.indices)
                .expect("upload duel lab model")
        };
        Self {
            floor: renderer.cube_mesh().expect("upload lab floor"),
            player: upload("duel lab player", player()),
            opponent: upload("duel lab opponent", opponent()),
            bolt: upload("duel lab bolt", bolt()),
            shot: upload("duel lab shot", shot()),
        }
    }
}

#[derive(Default)]
struct Geometry {
    vertices: Vec<MeshVertex>,
    indices: Vec<u32>,
}

impl Geometry {
    fn face<const N: usize>(&mut self, positions: [Vec3; N], color: [f32; 3]) {
        let first = self.vertices.len() as u32;
        self.vertices.extend(positions.map(|point| MeshVertex {
            position: [point.x, point.y, point.z],
            color,
        }));
        for corner in 1..N as u32 - 1 {
            self.indices
                .extend([first, first + corner, first + corner + 1]);
        }
    }

    /// A closed bevelled body part between two octagonal sections. Dimensions
    /// are half-extents in the ground plane; the model faces local +Z.
    fn hull(
        &mut self,
        center: [f32; 2],
        height: [f32; 2],
        lower: [f32; 2],
        upper: [f32; 2],
        color: [f32; 3],
    ) {
        const PROFILE: [[f32; 2]; 8] = [
            [-0.62, -1.0],
            [0.62, -1.0],
            [1.0, -0.62],
            [1.0, 0.62],
            [0.62, 1.0],
            [-0.62, 1.0],
            [-1.0, 0.62],
            [-1.0, -0.62],
        ];
        let ring = |y, [width, depth]: [f32; 2]| {
            PROFILE.map(|[x, z]| Vec3::new(center[0] + x * width, y, center[1] + z * depth))
        };
        let bottom = ring(height[0], lower);
        let mut top = ring(height[1], upper);
        let levels = [0.58, 0.67, 0.74, 0.86, 1.0, 0.87, 0.81, 0.68];
        for (i, level) in levels.into_iter().enumerate() {
            let j = (i + 1) % bottom.len();
            self.face(
                [bottom[i], top[i], top[j], bottom[j]],
                color.map(|c| c * level),
            );
        }
        self.face(bottom, color.map(|c| c * 0.5));
        top.reverse();
        self.face(top, color);
    }
}

fn player() -> Geometry {
    let mut model = Geometry::default();
    let dark = [0.09, 0.16, 0.22];
    let armor = [0.35, 0.82, 0.96];
    let accent = [0.90, 0.98, 1.0];
    for x in [-0.15, 0.15] {
        model.hull([x, 0.05], [0.0, 0.21], [0.11, 0.17], [0.09, 0.11], dark);
        model.hull([x, 0.0], [0.19, 0.52], [0.085, 0.09], [0.12, 0.12], armor);
    }
    model.hull([0.0, 0.0], [0.46, 0.91], [0.21, 0.14], [0.30, 0.18], armor);
    model.hull(
        [0.0, 0.172],
        [0.63, 0.83],
        [0.10, 0.032],
        [0.17, 0.032],
        accent,
    );
    for x in [-0.34, 0.34] {
        model.hull([x, 0.0], [0.54, 0.88], [0.065, 0.075], [0.10, 0.10], dark);
        model.hull(
            [x, 0.0],
            [0.87, 0.96],
            [0.115, 0.115],
            [0.075, 0.085],
            armor,
        );
    }
    model.hull(
        [0.0, 0.0],
        [0.90, 1.19],
        [0.15, 0.135],
        [0.11, 0.115],
        armor,
    );
    model.hull(
        [0.0, 0.126],
        [1.01, 1.08],
        [0.129, 0.030],
        [0.113, 0.030],
        dark,
    );
    model.hull(
        [0.0, 0.153],
        [1.028, 1.057],
        [0.083, 0.013],
        [0.083, 0.013],
        accent,
    );
    model
}

fn opponent() -> Geometry {
    let mut model = Geometry::default();
    // Neutral plates keep warm/cool attack and recovery tints readable.
    let shadow = [0.13, 0.14, 0.16];
    let metal = [0.55, 0.56, 0.60];
    let mask = [0.93, 0.94, 1.0];
    for x in [-0.22, 0.22] {
        model.hull([x, 0.059], [0.0, 0.20], [0.16, 0.22], [0.13, 0.16], shadow);
    }
    model.hull(
        [0.0, -0.13],
        [0.11, 1.02],
        [0.49, 0.24],
        [0.42, 0.23],
        shadow,
    );
    model.hull([0.0, 0.0], [0.54, 1.21], [0.35, 0.24], [0.52, 0.32], metal);
    model.hull([0.0, 0.29], [0.77, 1.13], [0.19, 0.035], [0.30, 0.07], mask);
    for x in [-0.56, 0.56] {
        model.hull([x, 0.0], [0.62, 1.30], [0.105, 0.12], [0.23, 0.28], metal);
        model.hull([x, 0.0], [1.28, 1.47], [0.23, 0.28], [0.12, 0.14], mask);
    }
    model.hull([0.0, 0.0], [1.19, 1.59], [0.22, 0.19], [0.20, 0.17], shadow);
    model.hull(
        [0.0, 0.16],
        [1.26, 1.56],
        [0.13, 0.082],
        [0.18, 0.079],
        mask,
    );
    for x in [-0.18, 0.18] {
        model.hull(
            [x, -0.04],
            [1.50, 1.86],
            [0.092, 0.105],
            [0.024, 0.040],
            metal,
        );
    }
    model
}

fn bolt() -> Geometry {
    let mut model = Geometry::default();
    let top = Vec3::new(0.0, 0.21, 0.0);
    let bottom = Vec3::new(0.0, -0.21, 0.0);
    let ring = [
        Vec3::new(0.0, 0.0, -0.18),
        Vec3::new(0.18, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.18),
        Vec3::new(-0.18, 0.0, 0.0),
    ];
    for (i, color) in [
        [0.50, 0.14, 0.035],
        [1.0, 0.38, 0.055],
        [1.0, 0.73, 0.16],
        [0.79, 0.24, 0.04],
    ]
    .into_iter()
    .enumerate()
    {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        model.face([a, top, b], color);
        model.face([a, b, bottom], color.map(|channel| channel * 0.65));
    }
    model
}

fn shot() -> Geometry {
    let mut model = bolt();
    for vertex in &mut model.vertices {
        let brightness = vertex.color[0];
        vertex.color = [0.16 * brightness, 0.82 * brightness, brightness];
    }
    model
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_have_only_finite_non_degenerate_triangles() {
        for model in [player(), opponent(), bolt(), shot()] {
            let vertex = |index: u32| {
                let data = model.vertices[index as usize];
                assert!(
                    data.position
                        .into_iter()
                        .chain(data.color)
                        .all(f32::is_finite)
                );
                let [x, y, z] = data.position;
                Vec3::new(x, y, z)
            };
            let (triangles, remainder) = model.indices.as_chunks::<3>();
            assert!(!triangles.is_empty() && remainder.is_empty());
            for &[a, b, c] in triangles {
                let origin = vertex(a);
                let area = (vertex(b) - origin)
                    .cross(vertex(c) - origin)
                    .length_squared();
                assert!(area > 0.0);
            }
        }
    }
}

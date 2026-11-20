//! CPU picking for indexed meshes; render resources do not retain pick data.

use std::ops::RangeInclusive;

use crate::math::{Mat4, Vec3};

use super::{MeshCreateError, mesh::validate_indices};

mod scene;
pub use scene::{PickInstance3d, PickScene3d, PickSceneHit3d, PickSceneStats};

type Point = [f64; 3];
const LEAF_TRIANGLES: usize = 8;
const EDGE_EPSILON: f64 = 1e-12;

/// A world-space ray with an inclusive distance interval in world units.
#[derive(Clone, Copy, Debug)]
pub struct PickRay3d {
    origin: Point,
    direction: Point,
    near: f64,
    far: f64,
}

impl PickRay3d {
    /// Normalizes `direction`. Rejects non-finite geometry, zero directions,
    /// and invalid distance intervals. The upper bound may be `f64::INFINITY`.
    pub fn new(origin: Vec3, direction: Vec3, distances: RangeInclusive<f64>) -> Option<Self> {
        if distances.is_empty() {
            return None;
        }
        let origin = wide(origin);
        let direction = wide(direction);
        let length = dot(direction, direction).sqrt();
        let (near, far) = distances.into_inner();
        if !origin.into_iter().all(f64::is_finite)
            || !length.is_finite()
            || length == 0.0
            || !near.is_finite()
            || near < 0.0
            || !(near..=f64::INFINITY).contains(&far)
        {
            return None;
        }
        Some(Self {
            origin,
            direction: direction.map(|component| component / length),
            near,
            far,
        })
    }

    pub(super) fn with_view_depth(mut self, forward: Vec3, near: f32, far: f32) -> Option<Self> {
        let depth_per_unit = dot(self.direction, wide(forward));
        if depth_per_unit <= 0.0 || !depth_per_unit.is_finite() {
            return None;
        }
        self.near = f64::from(near) / depth_per_unit;
        self.far = f64::from(far) / depth_per_unit;
        Some(self)
    }
}

/// The nearest geometric hit on an indexed triangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshHit3d {
    pub distance: f64,
    pub position: Vec3,
    /// Zero-based triangle number in the source index list.
    pub triangle: usize,
    /// Weights in the triangle's source index order, summing to one.
    pub barycentric: [f64; 3],
}

/// CPU work performed by one mesh instance query.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PickQueryStats {
    /// Number of hierarchy bounds checked, including rejected bounds.
    pub bounds_tests: usize,
    /// Number of exact triangle tests in the leaves reached by the ray.
    pub triangle_tests: usize,
}

/// Optional CPU copy of mesh positions and indices, reusable across instances.
///
/// Builds a local bounds hierarchy once, so queries can skip whole groups
/// of triangles. Queries do not allocate.
/// No window, GPU adapter, or render resource is required.
pub struct PickMesh3d {
    positions: Box<[[f32; 3]]>,
    indices: Box<[[u32; 3]]>,
    hierarchy: Hierarchy,
}

#[derive(Clone, Copy)]
struct Bounds {
    min: Point,
    max: Point,
}

impl Bounds {
    const EMPTY: Self = Self {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
    };

    fn include(&mut self, other: Self) {
        for i in 0..3 {
            self.min[i] = self.min[i].min(other.min[i]);
            self.max[i] = self.max[i].max(other.max[i]);
        }
    }

    fn include_point(&mut self, point: Point) {
        self.include(Self {
            min: point,
            max: point,
        });
    }
}

struct Node {
    bounds: Bounds,
    kind: NodeKind,
}

enum NodeKind {
    Leaf { start: usize, len: usize },
    Branch { left: usize, right: usize },
}

struct BuildItem {
    index: usize,
    bounds: Bounds,
    centroid: Point,
}

fn build_node(
    nodes: &mut Vec<Node>,
    items: &mut [BuildItem],
    start: usize,
    leaf_size: usize,
) -> usize {
    let mut bounds = Bounds::EMPTY;
    let mut centers = Bounds::EMPTY;
    for item in items.iter() {
        bounds.include(item.bounds);
        centers.include_point(item.centroid);
    }
    let node = nodes.len();
    nodes.push(Node {
        bounds,
        kind: NodeKind::Leaf {
            start,
            len: items.len(),
        },
    });
    if items.len() > leaf_size {
        let extent = sub(centers.max, centers.min);
        let axis = if extent[0] >= extent[1] && extent[0] >= extent[2] {
            0
        } else if extent[1] >= extent[2] {
            1
        } else {
            2
        };
        let mid = items.len() / 2;
        items.select_nth_unstable_by(mid, |a, b| {
            a.centroid[axis]
                .total_cmp(&b.centroid[axis])
                .then(a.index.cmp(&b.index))
        });
        let (left, right) = items.split_at_mut(mid);
        let left = build_node(nodes, left, start, leaf_size);
        let right = build_node(nodes, right, start + mid, leaf_size);
        nodes[node].kind = NodeKind::Branch { left, right };
    }
    node
}

struct Hierarchy {
    nodes: Box<[Node]>,
    order: Box<[usize]>,
}

trait Traversal {
    fn entry(&mut self, bounds: Bounds) -> Option<f64>;
    fn limit(&self) -> f64;
    fn visit_item(&mut self, index: usize);
}

impl Hierarchy {
    fn build(mut items: Vec<BuildItem>, leaf_size: usize) -> Self {
        let mut nodes = Vec::new();
        if !items.is_empty() {
            build_node(&mut nodes, &mut items, 0, leaf_size);
        }
        Self {
            nodes: nodes.into_boxed_slice(),
            order: items.into_iter().map(|item| item.index).collect(),
        }
    }

    fn traverse(&self, query: &mut impl Traversal) {
        if let Some(root) = self.nodes.first()
            && query.entry(root.bounds).is_some()
        {
            self.visit(0, query);
        }
    }

    fn visit(&self, node: usize, query: &mut impl Traversal) {
        match self.nodes[node].kind {
            NodeKind::Leaf { start, len } => {
                for &index in &self.order[start..start + len] {
                    query.visit_item(index);
                }
            }
            NodeKind::Branch { left, right } => {
                let left_entry = query.entry(self.nodes[left].bounds);
                let right_entry = query.entry(self.nodes[right].bounds);
                match (left_entry, right_entry) {
                    (Some(a), Some(b)) => {
                        let (first, second, entry) = if a <= b {
                            (left, right, b)
                        } else {
                            (right, left, a)
                        };
                        self.visit(first, query);
                        if entry <= query.limit() {
                            self.visit(second, query);
                        }
                    }
                    (Some(_), None) => self.visit(left, query),
                    (None, Some(_)) => self.visit(right, query),
                    (None, None) => {}
                }
            }
        }
    }
}

impl PickMesh3d {
    /// Copies the same object-space positions and triangle indices used for
    /// mesh upload. Non-finite positions are rejected.
    pub fn new(positions: &[[f32; 3]], indices: &[u32]) -> Result<Self, MeshCreateError> {
        validate_indices(positions.len(), indices)?;
        for (i, position) in positions.iter().enumerate() {
            if !position.iter().all(|p| p.is_finite()) {
                return Err(MeshCreateError::NonFinitePosition(i));
            }
        }
        let indices = indices.as_chunks::<3>().0;
        let triangles = indices
            .iter()
            .enumerate()
            .map(|(index, indices)| {
                let [a, b, c] = indices.map(|i| positions[i as usize].map(f64::from));
                let mut bounds = Bounds::EMPTY;
                for point in [a, b, c] {
                    bounds.include_point(point);
                }
                // Match the triangle test's edge tolerance in object space.
                for i in 0..3 {
                    let extent = bounds.max[i] - bounds.min[i];
                    let magnitude = bounds.min[i].abs().max(bounds.max[i].abs());
                    let margin = 2.0 * EDGE_EPSILON * extent + 8.0 * f64::EPSILON * magnitude;
                    bounds.min[i] -= margin;
                    bounds.max[i] += margin;
                }
                BuildItem {
                    index,
                    bounds,
                    centroid: std::array::from_fn(|i| (a[i] + b[i] + c[i]) / 3.0),
                }
            })
            .collect();
        Ok(Self {
            positions: positions.into(),
            indices: indices.into(),
            hierarchy: Hierarchy::build(triangles, LEAF_TRIANGLES),
        })
    }

    /// Returns the nearest two-sided triangle hit within the ray interval.
    /// Degenerate triangles and singular, non-affine, or non-finite models are
    /// skipped. Exact ties keep the first triangle. Materials do not affect hits.
    pub fn intersect(&self, ray: PickRay3d, model: Mat4) -> Option<MeshHit3d> {
        self.intersect_with_stats(ray, model).0
    }

    /// Like [`Self::intersect`], also reporting the amount of CPU work for
    /// this instance. Invalid transforms produce no work.
    pub fn intersect_with_stats(
        &self,
        ray: PickRay3d,
        model: Mat4,
    ) -> (Option<MeshHit3d>, PickQueryStats) {
        let Some((_, transform)) = affine_parts(model) else {
            return (None, PickQueryStats::default());
        };
        self.intersect_local(ray, &transform)
    }

    fn intersect_local(
        &self,
        ray: PickRay3d,
        transform: &LocalTransform,
    ) -> (Option<MeshHit3d>, PickQueryStats) {
        let Some((origin, direction)) = transform.ray(ray) else {
            return (None, PickQueryStats::default());
        };
        let mut query = Query {
            ray,
            origin,
            direction,
            hit: None,
            stats: PickQueryStats::default(),
        };
        self.hierarchy.traverse(&mut MeshTraversal {
            mesh: self,
            query: &mut query,
        });
        (query.hit, query.stats)
    }
}

struct MeshTraversal<'a> {
    mesh: &'a PickMesh3d,
    query: &'a mut Query,
}

impl Traversal for MeshTraversal<'_> {
    fn entry(&mut self, bounds: Bounds) -> Option<f64> {
        self.query.entry(bounds)
    }

    fn limit(&self) -> f64 {
        self.query.limit()
    }

    fn visit_item(&mut self, triangle: usize) {
        let [a, b, c] = self.mesh.indices[triangle]
            .map(|index| self.mesh.positions[index as usize].map(f64::from));
        self.query.test_triangle(triangle, a, b, c);
    }
}

struct Query {
    ray: PickRay3d,
    origin: Point,
    direction: Point,
    hit: Option<MeshHit3d>,
    stats: PickQueryStats,
}

impl Query {
    fn limit(&self) -> f64 {
        self.hit.map_or(self.ray.far, |hit| hit.distance)
    }

    fn entry(&mut self, bounds: Bounds) -> Option<f64> {
        self.stats.bounds_tests += 1;
        bounds_entry(
            self.origin,
            self.direction,
            bounds,
            self.ray.near,
            self.limit(),
        )
    }

    fn test_triangle(&mut self, triangle: usize, a: Point, b: Point, c: Point) {
        self.stats.triangle_tests += 1;
        let Some((distance, barycentric)) = hit_triangle(self.origin, self.direction, a, b, c)
        else {
            return;
        };
        if !distance.is_finite()
            || distance < self.ray.near
            || distance > self.limit()
            || self
                .hit
                .is_some_and(|hit| distance == hit.distance && triangle >= hit.triangle)
        {
            return;
        }
        let [x, y, z] =
            std::array::from_fn(|i| (self.ray.origin[i] + self.ray.direction[i] * distance) as f32);
        if [x, y, z].into_iter().all(f32::is_finite) {
            self.hit = Some(MeshHit3d {
                distance,
                position: Vec3::new(x, y, z),
                triangle,
                barycentric,
            });
        }
    }
}

fn wide(v: Vec3) -> Point {
    v.to_array().map(f64::from)
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

struct LocalTransform {
    rows: [Point; 3],
    determinant: f64,
    translation: Point,
}

fn affine_parts(model: Mat4) -> Option<([Point; 3], LocalTransform)> {
    let m = model.to_cols_array().map(f64::from);
    if !m.into_iter().all(f64::is_finite) || [m[3], m[7], m[11], m[15]] != [0.0, 0.0, 0.0, 1.0] {
        return None;
    }
    let (a, b, c) = ([m[0], m[1], m[2]], [m[4], m[5], m[6]], [m[8], m[9], m[10]]);
    let rows = [cross(b, c), cross(c, a), cross(a, b)];
    let determinant = dot(a, rows[0]);
    if determinant == 0.0 {
        return None;
    }
    Some((
        [a, b, c],
        LocalTransform {
            rows,
            determinant,
            translation: [m[12], m[13], m[14]],
        },
    ))
}

impl LocalTransform {
    fn ray(&self, ray: PickRay3d) -> Option<(Point, Point)> {
        let inverse_linear = |v| self.rows.map(|row| dot(row, v) / self.determinant);
        // The local direction stays unnormalized, so the ray parameter
        // remains a world distance even with shear or nonuniform scale.
        let origin = inverse_linear(sub(ray.origin, self.translation));
        let direction = inverse_linear(ray.direction);
        origin
            .into_iter()
            .chain(direction)
            .all(f64::is_finite)
            .then_some((origin, direction))
    }
}

fn bounds_entry(
    origin: Point,
    direction: Point,
    bounds: Bounds,
    mut near: f64,
    mut far: f64,
) -> Option<f64> {
    for axis in 0..3 {
        if direction[axis] == 0.0 {
            if !(bounds.min[axis]..=bounds.max[axis]).contains(&origin[axis]) {
                return None;
            }
        } else {
            let a = (bounds.min[axis] - origin[axis]) / direction[axis];
            let b = (bounds.max[axis] - origin[axis]) / direction[axis];
            let lower = a.min(b);
            let upper = a.max(b);
            // Round outward for distance pruning, so equal-distance hits in
            // separate branches can still resolve by source triangle index.
            let pad = |t: f64| {
                if t.is_finite() {
                    16.0 * f64::EPSILON * t.abs()
                } else {
                    0.0
                }
            };
            near = near.max(lower - pad(lower));
            far = far.min(upper + pad(upper));
            if near > far {
                return None;
            }
        }
    }
    Some(near)
}

fn hit_triangle(
    origin: Point,
    direction: Point,
    a: Point,
    b: Point,
    c: Point,
) -> Option<(f64, [f64; 3])> {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let p = cross(direction, ac);
    let det = dot(ab, p);
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let t = sub(origin, a);
    let u = dot(t, p) / det;
    let q = cross(t, ab);
    let v = dot(direction, q) / det;
    let w = 1.0 - u - v;
    // Allow roundoff on shared edges.
    if ![w, u, v]
        .into_iter()
        .all(|weight| (-EDGE_EPSILON..=1.0 + EDGE_EPSILON).contains(&weight))
    {
        return None;
    }
    let weights = [w, u, v].map(|weight| weight.max(0.0));
    let total: f64 = weights.into_iter().sum();
    Some((dot(ac, q) / det, weights.map(|weight| weight / total)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec2;
    use crate::renderer::Camera3d;

    const TRIANGLE: [[f32; 3]; 3] = [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]];

    fn scan(mesh: &PickMesh3d, ray: PickRay3d, model: Mat4) -> Option<MeshHit3d> {
        let (origin, direction) = affine_parts(model)?.1.ray(ray)?;
        let mut query = Query {
            ray,
            origin,
            direction,
            hit: None,
            stats: PickQueryStats::default(),
        };
        for (triangle, indices) in mesh.indices.iter().enumerate() {
            let [a, b, c] = indices.map(|index| mesh.positions[index as usize].map(f64::from));
            query.test_triangle(triangle, a, b, c);
        }
        query.hit
    }

    #[test]
    fn hierarchy_agrees_with_direct_scan_and_skips_dense_geometry() {
        const SIDE: u32 = 48;
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for z in [-12.0, -4.0] {
            let offset = vertices.len() as u32;
            for y in 0..=SIDE {
                for x in 0..=SIDE {
                    vertices.push([x as f32 - 24.0, y as f32 - 24.0, z]);
                    if x < SIDE && y < SIDE {
                        let base = offset + y * (SIDE + 1) + x;
                        indices.extend([
                            base,
                            base + 1,
                            base + SIDE + 1,
                            base + 1,
                            base + SIDE + 2,
                            base + SIDE + 1,
                        ]);
                    }
                }
            }
        }
        let mesh = PickMesh3d::new(&vertices, &indices).unwrap();
        let ray = PickRay3d::new(Vec3::new(3.25, 2.7, 0.0), -Vec3::Z, 0.0..=30.0).unwrap();
        let (hit, stats) = mesh.intersect_with_stats(ray, Mat4::IDENTITY);
        assert_eq!(hit, scan(&mesh, ray, Mat4::IDENTITY));
        assert_eq!(hit.unwrap().distance, 4.0);
        assert!(
            stats.triangle_tests <= 32 && stats.bounds_tests < 128,
            "{stats:?}"
        );

        // Exercise hit and miss rays from both sides, through shared edges,
        // and under model transforms, against the reference scan.
        for model in [
            Mat4::IDENTITY,
            Mat4::rotation_y(0.37).mul(Mat4::scale(Vec3::new(-2.0, 0.75, 1.5))),
        ] {
            for z in [0.0, -25.0] {
                for i in -7..=7 {
                    let ray = PickRay3d::new(
                        Vec3::new(i as f32 * 4.3, i as f32 * 3.0, z),
                        Vec3::new(0.12, 0.02, if z == 0.0 { -1.0 } else { 1.0 }),
                        1.0..=30.0,
                    )
                    .unwrap();
                    assert_eq!(mesh.intersect(ray, model), scan(&mesh, ray, model));
                }
            }
        }
        let miss = PickRay3d::new(Vec3::new(50.0, 0.0, 0.0), -Vec3::Z, 0.0..=30.0).unwrap();
        assert_eq!(
            mesh.intersect_with_stats(miss, Mat4::IDENTITY),
            (
                None,
                PickQueryStats {
                    bounds_tests: 1,
                    triangle_tests: 0
                }
            )
        );
    }

    #[test]
    fn ties_across_branches_keep_the_lowest_source_triangle() {
        let vertices = [
            [0.0, -1.0, -4.0],
            [2.0, -1.0, -4.0],
            [0.0, 1.0, -4.0],
            [-2.0, -1.0, -4.0],
        ];
        let mut indices = [0, 1, 2].repeat(LEAF_TRIANGLES);
        indices.extend([0, 2, 3].repeat(LEAF_TRIANGLES));
        let mesh = PickMesh3d::new(&vertices, &indices).unwrap();
        let ray = PickRay3d::new(Vec3::ZERO, -Vec3::Z, 0.0..=4.0).unwrap();
        let (hit, stats) = mesh.intersect_with_stats(ray, Mat4::IDENTITY);
        assert_eq!(stats.triangle_tests, 2 * LEAF_TRIANGLES);
        assert_eq!(hit.unwrap().triangle, 0);
    }

    #[test]
    fn selects_nearest_triangle_and_preserves_exact_ties() {
        let positions = [
            TRIANGLE.map(|[x, y, z]| [x, y, z - 5.0]),
            TRIANGLE.map(|[x, y, z]| [x, y, z - 3.0]),
        ]
        .concat();
        let mesh = PickMesh3d::new(&positions, &[0, 1, 2, 3, 5, 4, 3, 4, 5]).unwrap();
        let ray = PickRay3d::new(Vec3::ZERO, -Vec3::Z * 5.0, 0.0..=10.0).unwrap();
        let hit = mesh.intersect(ray, Mat4::IDENTITY).unwrap();
        assert_eq!((hit.triangle, hit.distance), (1, 3.0));
        assert_eq!(hit.position, Vec3::new(0.0, 0.0, -3.0));
        assert_eq!(hit.barycentric, [0.25, 0.5, 0.25]);
        let clipped = PickRay3d::new(Vec3::ZERO, -Vec3::Z, 4.0..=5.0).unwrap();
        assert_eq!(mesh.intersect(clipped, Mat4::IDENTITY).unwrap().triangle, 0);
    }

    #[test]
    fn returns_world_distance_for_rotated_scaled_and_mirrored_models() {
        let mesh = PickMesh3d::new(&TRIANGLE, &[0, 1, 2]).unwrap();
        let model = Mat4::translation(Vec3::new(2.0, 1.0, -7.0))
            .mul(Mat4::rotation_y(0.7))
            .mul(Mat4::scale(Vec3::new(-2.0, 0.5, 3.0)));
        let ray = PickRay3d::new(Vec3::new(2.0, 1.0, 0.0), -Vec3::Z, 5.0..=10.0).unwrap();
        let hit = mesh.intersect(ray, model).unwrap();
        assert!((hit.distance - 7.0).abs() < 1e-11);
        assert_eq!(hit.position, Vec3::new(2.0, 1.0, -7.0));
        assert!(
            mesh.intersect(ray, Mat4::scale(Vec3::new(1.0, 1.0, 0.0)))
                .is_none()
        );
        let mut non_affine = model;
        non_affine.m[3] = 0.5;
        assert!(mesh.intersect(ray, non_affine).is_none());
        let mut invalid = model;
        invalid.m[0] = f32::NAN;
        assert!(mesh.intersect(ray, invalid).is_none());
    }

    #[test]
    fn misses_inside_bounds_do_not_count_as_triangle_hits() {
        let mesh = PickMesh3d::new(&TRIANGLE, &[0, 0, 1, 0, 1, 2]).unwrap();
        let model = Mat4::translation(-Vec3::Z);
        let cast = |x, y| PickRay3d::new(Vec3::new(x, y, 0.0), -Vec3::Z, 0.0..=10.0).unwrap();
        assert!(mesh.intersect(cast(2.0, 0.0), model).is_none());
        assert!(mesh.intersect(cast(0.9, 0.9), model).is_none());
        assert_eq!(mesh.intersect(cast(0.0, 1.0), model).unwrap().triangle, 1);
    }

    #[test]
    fn camera_picking_limits_off_axis_rays_by_view_depth_not_radial_distance() {
        let camera = Camera3d {
            position: Vec3::ZERO,
            near: 1.0,
            far: 5.0,
            ..Default::default()
        };
        let mesh = PickMesh3d::new(&TRIANGLE, &[0, 1, 2]).unwrap();
        let ray = camera.pick_ray(Vec2::new(90.0, 50.0), (100, 100)).unwrap();
        let model = |depth: f32| {
            Mat4::translation(Vec3::new(0.0, 0.0, -depth))
                .mul(Mat4::scale(Vec3::new(20.0, 20.0, 1.0)))
        };
        let hit = mesh.intersect(ray, model(4.9)).unwrap();
        assert!(hit.distance > 5.0);
        assert_eq!(hit.position.z, -4.9);
        assert!(mesh.intersect(ray, model(0.9)).is_none());
        assert!(mesh.intersect(ray, model(5.1)).is_none());
        assert!(camera.pick_ray(Vec2::new(-1.0, 50.0), (100, 100)).is_none());
    }

    #[test]
    fn rejects_invalid_cpu_geometry_and_ray_inputs() {
        let mut bad = TRIANGLE;
        bad[1][0] = f32::INFINITY;
        assert!(matches!(
            PickMesh3d::new(&bad, &[0, 1, 2]),
            Err(MeshCreateError::NonFinitePosition(1))
        ));
        assert!(matches!(
            PickMesh3d::new(&TRIANGLE, &[0, 1, 4]),
            Err(MeshCreateError::IndexOutOfBounds { index: 4, .. })
        ));
        assert!(PickRay3d::new(Vec3::ZERO, Vec3::ZERO, 0.0..=10.0).is_none());
        assert!(PickRay3d::new(Vec3::ZERO, Vec3::X, -1.0..=10.0).is_none());
        assert!(PickRay3d::new(Vec3::ZERO, Vec3::X, 0.0..=f64::NAN).is_none());
        assert!(PickRay3d::new(Vec3::ZERO, Vec3::X * f32::MAX, 0.0..=f64::INFINITY).is_some());
    }
}

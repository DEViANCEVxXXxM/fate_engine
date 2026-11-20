//! Broad-phase picking over a snapshot of consumer-owned instances.

use std::sync::Arc;

use crate::math::Mat4;

use super::{
    Bounds, BuildItem, Hierarchy, LocalTransform, MeshHit3d, PickMesh3d, PickQueryStats, PickRay3d,
    Point, Traversal, affine_parts, bounds_entry,
};

/// One mesh and model transform to index for CPU picking.
#[derive(Clone)]
pub struct PickInstance3d {
    pub mesh: Arc<PickMesh3d>,
    pub model: Mat4,
}

/// A geometric hit and its original instance index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickSceneHit3d {
    /// Zero-based position in the input to [`PickScene3d::new`].
    pub instance: usize,
    pub mesh_hit: MeshHit3d,
}

/// CPU work performed by a single scene ray. These counts are not timings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PickSceneStats {
    /// Scene hierarchy bounds checked, including rejected groups.
    pub bounds_tests: usize,
    /// Number of instances whose local mesh hierarchy was queried.
    pub instances_tested: usize,
    /// Sum of local mesh bounds and triangle tests across those instances.
    pub mesh: PickQueryStats,
}

/// A CPU-only snapshot of pickable instances, with a world-space bounds tree.
///
/// Rebuild it if transforms, ordering, or membership change. Mesh trees
/// are shared and are not rebuilt or copied. Queries do not allocate.
pub struct PickScene3d {
    instances: Box<[IndexedInstance]>,
    hierarchy: Hierarchy,
}

struct IndexedInstance {
    source_index: usize,
    mesh: Arc<PickMesh3d>,
    transform: LocalTransform,
}

impl PickScene3d {
    /// Index the supplied instances. Invalid transforms are skipped without
    /// renumbering later inputs. An empty scene is valid.
    pub fn new(instances: impl IntoIterator<Item = PickInstance3d>) -> Self {
        let mut valid = Vec::new();
        let mut items = Vec::new();
        for (source_index, PickInstance3d { mesh, model }) in instances.into_iter().enumerate() {
            let Some((columns, transform)) = affine_parts(model) else {
                continue;
            };
            let bounds = world_bounds(
                mesh.hierarchy.nodes[0].bounds,
                columns,
                transform.translation,
            );
            items.push(BuildItem {
                index: valid.len(),
                bounds,
                centroid: std::array::from_fn(|i| (bounds.min[i] + bounds.max[i]) * 0.5),
            });
            valid.push(IndexedInstance {
                source_index,
                mesh,
                transform,
            });
        }
        Self {
            instances: valid.into_boxed_slice(),
            hierarchy: Hierarchy::build(items, 1),
        }
    }

    /// Returns the nearest hit. Exact distance ties favor the earlier instance.
    pub fn intersect(&self, ray: PickRay3d) -> Option<PickSceneHit3d> {
        self.intersect_with_stats(ray).0
    }

    /// Also reports how much work the scene and local mesh queries performed.
    pub fn intersect_with_stats(&self, ray: PickRay3d) -> (Option<PickSceneHit3d>, PickSceneStats) {
        let mut query = SceneTraversal {
            scene: self,
            ray,
            hit: None,
            stats: PickSceneStats::default(),
        };
        self.hierarchy.traverse(&mut query);
        (query.hit, query.stats)
    }
}

fn world_bounds(local: Bounds, columns: [Point; 3], translation: Point) -> Bounds {
    let mut world = Bounds {
        min: translation,
        max: translation,
    };
    for axis in 0..3 {
        let mut magnitude = translation[axis].abs();
        for (i, column) in columns.iter().enumerate() {
            let a = column[axis] * local.min[i];
            let b = column[axis] * local.max[i];
            world.min[axis] += a.min(b);
            world.max[axis] += a.max(b);
            magnitude += a.abs().max(b.abs());
        }
        // Round outwards, including cancellation between opposing terms.
        let pad = 16.0 * f64::EPSILON * magnitude;
        world.min[axis] -= pad;
        world.max[axis] += pad;
    }
    world
}

struct SceneTraversal<'a> {
    scene: &'a PickScene3d,
    ray: PickRay3d,
    hit: Option<PickSceneHit3d>,
    stats: PickSceneStats,
}

impl Traversal for SceneTraversal<'_> {
    fn entry(&mut self, bounds: Bounds) -> Option<f64> {
        self.stats.bounds_tests += 1;
        bounds_entry(
            self.ray.origin,
            self.ray.direction,
            bounds,
            self.ray.near,
            self.limit(),
        )
    }

    fn limit(&self) -> f64 {
        self.hit.map_or(self.ray.far, |hit| hit.mesh_hit.distance)
    }

    fn visit_item(&mut self, index: usize) {
        let instance = &self.scene.instances[index];
        let ray = PickRay3d {
            far: self.limit(),
            ..self.ray
        };
        self.stats.instances_tested += 1;
        let (hit, stats) = instance.mesh.intersect_local(ray, &instance.transform);
        self.stats.mesh.bounds_tests += stats.bounds_tests;
        self.stats.mesh.triangle_tests += stats.triangle_tests;
        if let Some(mesh_hit) = hit
            && self.hit.is_none_or(|best| {
                mesh_hit.distance < best.mesh_hit.distance
                    || (mesh_hit.distance == best.mesh_hit.distance
                        && instance.source_index < best.instance)
            })
        {
            self.hit = Some(PickSceneHit3d {
                instance: instance.source_index,
                mesh_hit,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;

    fn triangle() -> Arc<PickMesh3d> {
        Arc::new(
            PickMesh3d::new(
                &[[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0], [0.0, 0.5, 0.0]],
                &[0, 1, 2],
            )
            .unwrap(),
        )
    }

    fn scan(instances: &[PickInstance3d], ray: PickRay3d) -> Option<PickSceneHit3d> {
        instances
            .iter()
            .enumerate()
            .filter_map(|(instance, draw)| {
                Some(PickSceneHit3d {
                    instance,
                    mesh_hit: draw.mesh.intersect(ray, draw.model)?,
                })
            })
            .min_by(|a, b| {
                a.mesh_hit
                    .distance
                    .total_cmp(&b.mesh_hit.distance)
                    .then(a.instance.cmp(&b.instance))
            })
    }

    #[test]
    fn scene_hierarchy_matches_instance_scan_on_a_transformed_grid() {
        let mesh = triangle();
        let instances: Vec<_> = (0..1024)
            .map(|index| {
                let pos = Vec3::new((index % 32) as f32, (index / 32) as f32, -8.0) * 2.0;
                let mut model = Mat4::translation(pos)
                    .mul(Mat4::rotation_y(0.31))
                    .mul(Mat4::scale(Vec3::new(-1.4, 0.75, 1.0)));
                model.m[4] += 0.13; // world-space shear
                PickInstance3d {
                    mesh: mesh.clone(),
                    model,
                }
            })
            .collect();
        let scene = PickScene3d::new(instances.iter().cloned());
        for i in [-1, 0, 1, 3, 5, 7, 15, 16, 30, 31, 32] {
            for x_offset in [0.0, 0.7] {
                let ray = PickRay3d::new(
                    Vec3::new(i as f32 * 2.0 + x_offset, 12.0, 0.0),
                    -Vec3::Z,
                    0.0..=30.0,
                )
                .unwrap();
                assert_eq!(scene.intersect(ray), scan(&instances, ray));
            }
        }
        let ray = PickRay3d::new(Vec3::new(30.0, 12.0, 0.0), -Vec3::Z, 0.0..=30.0).unwrap();
        let (hit, stats) = scene.intersect_with_stats(ray);
        assert_eq!(hit.unwrap().instance, 6 * 32 + 15);
        assert_eq!(stats.instances_tested, 1);
        assert_eq!(stats.mesh.triangle_tests, 1);
        assert!(stats.bounds_tests < 32, "{stats:?}");
    }

    #[test]
    fn overlapping_instances_keep_nearest_and_source_order_on_ties() {
        let mesh = triangle();
        let instance = |model| PickInstance3d {
            mesh: mesh.clone(),
            model,
        };
        let model = |x, z| Mat4::translation(Vec3::new(x, 0.0, z));
        let instances = [
            instance(Mat4::scale(Vec3::new(1.0, 1.0, 0.0))),
            instance(model(0.0, -15.0)),
            instance(model(0.1, -5.0)),
            instance(model(-0.1, -5.0)),
        ];
        let scene = PickScene3d::new(instances.iter().cloned());
        let ray = PickRay3d::new(Vec3::ZERO, -Vec3::Z, 0.0..=50.0).unwrap();
        let (hit, stats) = scene.intersect_with_stats(ray);
        assert_eq!(hit, scan(&instances, ray));
        assert_eq!(hit.unwrap().instance, 2);
        assert_eq!(stats.instances_tested, 2); // farther instance pruned
        let clipped = PickRay3d::new(Vec3::ZERO, -Vec3::Z, 6.0..=50.0).unwrap();
        assert_eq!(scene.intersect(clipped).unwrap().instance, 1);

        let empty = PickScene3d::new([instances[0].clone()]);
        assert_eq!(
            empty.intersect_with_stats(ray),
            (None, PickSceneStats::default())
        );
    }
}

//! Headless build and query timings for the CPU scene picker.

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fate_engine::math::{Mat4, Vec3};
use fate_engine::renderer::{PickInstance3d, PickMesh3d, PickRay3d, PickScene3d, PickSceneHit3d};

const SIDE: usize = 32;
const INSTANCES: usize = SIDE * SIDE;
const RAYS: usize = 64;
const QUERY_SAMPLES: usize = RAYS * 16;
const BUILD_SAMPLES: usize = 40;
const WARMUP: usize = 4;

fn instances(mesh: &Arc<PickMesh3d>, overlap: bool) -> Vec<PickInstance3d> {
    (0..INSTANCES)
        .map(|i| {
            let (x, y) = ((i % SIDE) as f32, (i / SIDE) as f32);
            let spacing = if overlap { 0.001 } else { 2.0 };
            PickInstance3d {
                mesh: mesh.clone(),
                model: Mat4::translation(Vec3::new(x * spacing, y * spacing, -8.0)),
            }
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Workload {
    GridHits,
    GridMisses,
    Grazing,
    Overlap,
}

impl Workload {
    fn label(self) -> &'static str {
        match self {
            Self::GridHits => "grid hits",
            Self::GridMisses => "grid misses",
            Self::Grazing => "grazing edges",
            Self::Overlap => "heavy overlap",
        }
    }

    fn rays(self) -> [PickRay3d; RAYS] {
        std::array::from_fn(|i| {
            let x = ((i * 13) % SIDE) as f32 * 2.0;
            let y = ((i * 7 + i / SIDE) % SIDE) as f32 * 2.0;
            let even = i.is_multiple_of(2);
            let (x, y) = match self {
                Self::GridHits => (x + 0.14, y - 0.22),
                Self::GridMisses if even => (x + 0.75, y),
                Self::GridMisses => (-2.0, y),
                Self::Grazing => (x + if even { 0.499 } else { 0.501 }, y + 0.12),
                // All 1,024 bounds overlap these rays at the same depth.
                Self::Overlap => (i as f32 * 0.002 - 0.06, 0.13),
            };
            PickRay3d::new(Vec3::new(x, y, 0.0), -Vec3::Z, 0.0..=20.0).unwrap()
        })
    }

    fn expects_hit(self, i: usize) -> bool {
        match self {
            Self::GridHits | Self::Overlap => true,
            Self::GridMisses => false,
            Self::Grazing => i.is_multiple_of(2),
        }
    }
}

fn shared_quad() -> Arc<PickMesh3d> {
    Arc::new(
        PickMesh3d::new(
            &[
                [-0.5, -0.5, 0.0],
                [0.5, -0.5, 0.0],
                [0.5, 0.5, 0.0],
                [-0.5, 0.5, 0.0],
            ],
            &[0, 1, 2, 0, 2, 3],
        )
        .unwrap(),
    )
}

fn scan(instances: &[PickInstance3d], ray: PickRay3d) -> Option<PickSceneHit3d> {
    instances
        .iter()
        .enumerate()
        .filter_map(|(instance, item)| {
            Some(PickSceneHit3d {
                instance,
                mesh_hit: item.mesh.intersect(ray, item.model)?,
            })
        })
        .min_by(|a, b| {
            a.mesh_hit
                .distance
                .total_cmp(&b.mesh_hit.distance)
                .then(a.instance.cmp(&b.instance))
        })
}

fn verify(
    scene: &PickScene3d,
    instances: &[PickInstance3d],
    workload: Workload,
    rays: &[PickRay3d],
) {
    for (i, &ray) in rays.iter().enumerate() {
        let hit = scene.intersect(ray);
        assert_eq!(hit, scan(instances, ray), "{} ray {i}", workload.label());
        assert_eq!(
            hit.is_some(),
            workload.expects_hit(i),
            "{} ray {i}",
            workload.label()
        );
    }
}

fn sample(f: impl FnOnce()) -> Duration {
    let start = Instant::now();
    f();
    start.elapsed()
}

fn print_percentiles(label: &str, samples: &mut [Duration], scale: f64, unit: &str) {
    samples.sort_unstable();
    let quantile =
        |percent: usize| samples[(samples.len() * percent).div_ceil(100) - 1].as_secs_f64() * scale;
    println!(
        "{label:<28} p50 {:>10.3}  p95 {:>10.3} {unit}",
        quantile(50),
        quantile(95)
    );
}

fn build_bench(label: &str, instances: &[PickInstance3d]) -> PickScene3d {
    let build = || PickScene3d::new(black_box(instances).iter().cloned());
    for _ in 0..WARMUP {
        black_box(build());
    }
    let mut samples = [Duration::ZERO; BUILD_SAMPLES];
    for elapsed in &mut samples {
        let start = Instant::now();
        let scene = black_box(build());
        *elapsed = start.elapsed();
        drop(scene);
    }
    print_percentiles(label, &mut samples, 1e3, "ms/build");
    build()
}

fn query_bench(scene: &PickScene3d, instances: &[PickInstance3d], workload: Workload) {
    let rays = workload.rays();
    verify(scene, instances, workload, &rays);
    let indexed = |ray| {
        black_box(scene.intersect(black_box(ray)));
    };
    let linear = |ray| {
        black_box(scan(instances, black_box(ray)));
    };
    for &ray in rays.iter().cycle().take(RAYS * WARMUP) {
        indexed(ray);
        linear(ray);
    }
    let mut index_samples = [Duration::ZERO; QUERY_SAMPLES];
    let mut scan_samples = [Duration::ZERO; QUERY_SAMPLES];
    for (i, &ray) in rays.iter().cycle().take(QUERY_SAMPLES).enumerate() {
        // Alternate timing order on successive passes over the same rays.
        if (i / RAYS).is_multiple_of(2) {
            index_samples[i] = sample(|| indexed(ray));
            scan_samples[i] = sample(|| linear(ray));
        } else {
            scan_samples[i] = sample(|| linear(ray));
            index_samples[i] = sample(|| indexed(ray));
        }
    }
    let total: usize = rays
        .iter()
        .map(|&ray| scene.intersect_with_stats(ray).1.instances_tested)
        .sum();
    println!(
        "\n{}: indexed {:.1} of {INSTANCES} instances queried per ray on average",
        workload.label(),
        total as f64 / RAYS as f64
    );
    print_percentiles("scene index", &mut index_samples, 1e6, "us/ray");
    print_percentiles("linear instance scan", &mut scan_samples, 1e6, "us/ray");
}

fn main() {
    println!("CPU picking only: no window or GPU, {INSTANCES} instances per scene");
    println!(
        "{BUILD_SAMPLES} build samples, {QUERY_SAMPLES} individual ray samples per workload (includes timer overhead)"
    );
    println!(
        "Both query paths use the same shared mesh picker; build timings exclude mesh creation and scene drop.\n"
    );
    let mesh = shared_quad();
    let grid = instances(&mesh, false);
    let overlap = instances(&mesh, true);
    let grid_scene = build_bench("grid scene build", &grid);
    let overlap_scene = build_bench("overlap scene build", &overlap);
    for workload in [Workload::GridHits, Workload::GridMisses, Workload::Grazing] {
        query_bench(&grid_scene, &grid, workload);
    }
    query_bench(&overlap_scene, &overlap, Workload::Overlap);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workloads_have_the_intended_hits_and_match_the_reference() {
        let mesh = shared_quad();
        let grid = instances(&mesh, false);
        let scene = PickScene3d::new(grid.iter().cloned());
        for workload in [Workload::GridHits, Workload::GridMisses, Workload::Grazing] {
            verify(&scene, &grid, workload, &workload.rays());
        }
        let overlap = instances(&mesh, true);
        let scene = PickScene3d::new(overlap.iter().cloned());
        let workload = Workload::Overlap;
        verify(&scene, &overlap, workload, &workload.rays());
    }
}

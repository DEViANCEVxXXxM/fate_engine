//! Headless ordered visibility queries on a retained grid, as used by Polygon.

use std::hint::black_box;
use std::time::{Duration, Instant};

use fate_engine::math::Vec3;
use fate_engine::renderer::{Bounds3d, Camera3d, VisibilitySet3d, VisibilityStats3d};

const SIDE: usize = 64;
const SAMPLES: usize = 512;
const WARMUP: usize = 32;

fn main() {
    let bounds: Vec<_> = (0..SIDE * SIDE)
        .map(|i| {
            let half = (SIDE - 1) as f32 * 0.5;
            let x = (i % SIDE) as f32 - half;
            let y = (i / SIDE) as f32 - half;
            Bounds3d::new(Vec3::new(x * 1.26, y * 1.26, -3.0), Vec3::ONE)
        })
        .collect();
    let start = Instant::now();
    let set = VisibilitySet3d::new(bounds.iter().copied());
    println!(
        "{} retained bounds | one-time build: {:.1} us | no window/GPU",
        bounds.len(),
        start.elapsed().as_secs_f64() * 1e6
    );
    println!("{WARMUP} warmup + {SAMPLES} queries per view | p50/p95 us");
    for (label, depth, fov) in [
        ("wide view", 140.0, 75.0_f32),
        ("Polygon view", 30.0, 75.0),
        ("narrow view", 30.0, 15.0),
    ] {
        let mut scan = Vec::with_capacity(bounds.len());
        let mut indexed = Vec::with_capacity(bounds.len());
        let mut scan_times = Vec::with_capacity(SAMPLES);
        let mut indexed_times = Vec::with_capacity(SAMPLES);
        let mut totals = VisibilityStats3d::default();
        let mut total_visible = 0;
        for frame in 0..WARMUP + SAMPLES {
            let camera = Camera3d {
                position: Vec3::new((frame as f32 * 0.04).sin() * 8.0, 0.0, depth),
                fov_y_radians: fov.to_radians(),
                ..Default::default()
            };
            let frustum = camera.frustum(16.0 / 9.0);
            let mut run_scan = || {
                timed(|| {
                    scan.clear();
                    scan.extend(
                        black_box(&bounds)
                            .iter()
                            .enumerate()
                            .filter_map(|(i, &b)| frustum.intersects_bounds(b).then_some(i)),
                    );
                })
                .0
            };
            let mut run_index = || timed(|| set.query(black_box(frustum), &mut indexed));
            let (scan_time, (index_time, work)) = if frame % 2 == 0 {
                (run_scan(), run_index())
            } else {
                let result = run_index();
                (run_scan(), result)
            };
            assert_eq!(scan, indexed);
            if frame >= WARMUP {
                scan_times.push(scan_time);
                indexed_times.push(index_time);
                totals.group_tests += work.group_tests;
                totals.bounds_tests += work.bounds_tests;
                total_visible += indexed.len();
            }
        }
        println!(
            "{label}: {:.0} visible | {:.0} groups + {:.0} individual tests/query (scan: {})",
            total_visible as f64 / SAMPLES as f64,
            totals.group_tests as f64 / SAMPLES as f64,
            totals.bounds_tests as f64 / SAMPLES as f64,
            bounds.len(),
        );
        print_times("scan", &mut scan_times);
        print_times("grouped", &mut indexed_times);
    }
}

fn timed<T>(run: impl FnOnce() -> T) -> (Duration, T) {
    let start = Instant::now();
    let result = run();
    (start.elapsed(), result)
}

fn print_times(label: &str, times: &mut [Duration]) {
    times.sort_unstable();
    let us = |percent: usize| times[(times.len() * percent).div_ceil(100) - 1].as_secs_f64() * 1e6;
    println!("  {label:7} {:7.2}/{:.2}", us(50), us(95));
}

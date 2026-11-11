//! CPU-side baseline for preparing an indexed-mesh draw list.
//!
//! Measures frustum culling and draw-record assembly for a fixed box grid.
//! It never initializes a GPU, so driver-side submission cost is not included.
//!
//! Run with:
//!
//! ```powershell
//! cargo run --release -p fate_engine --example draw_list_bench
//! ```

use std::hint::black_box;
use std::time::Instant;

use fate_engine::math::Vec3;
use fate_engine::renderer::{Bounds3d, Camera3d};

const GRID: usize = 16;
const FRAMES: usize = 200;
const WARMUP: usize = 20;

fn main() {
    let camera = Camera3d {
        position: Vec3::new(0.0, 12.0, 40.0),
        pitch: -0.25,
        ..Default::default()
    };
    let frustum = camera.frustum(16.0 / 9.0);

    let spacing = 4.0;
    let offset = (GRID as f32 - 1.0) * 0.5 * spacing;
    let candidates: Vec<Bounds3d> = (0..GRID * GRID * GRID)
        .map(|index| {
            let x = (index % GRID) as f32 * spacing - offset;
            let y = ((index / GRID) % GRID) as f32 * spacing - offset;
            let z = (index / (GRID * GRID)) as f32 * spacing - offset;
            Bounds3d::new(Vec3::new(x, y, z), Vec3::new(0.5, 0.5, 0.5))
        })
        .collect();

    let mut visible = Vec::with_capacity(candidates.len());
    let mut all = Vec::with_capacity(candidates.len());

    for _ in 0..WARMUP {
        visible.clear();
        for bounds in &candidates {
            if frustum.intersects_bounds(*bounds) {
                visible.push(*bounds);
            }
        }
    }

    let start = Instant::now();
    let mut visible_total = 0usize;
    for _ in 0..FRAMES {
        visible.clear();
        for bounds in &candidates {
            if frustum.intersects_bounds(*bounds) {
                visible.push(*bounds);
            }
        }
        visible_total += visible.len();
    }
    let cull_elapsed = start.elapsed();

    let start = Instant::now();
    for _ in 0..FRAMES {
        all.clear();
        all.extend_from_slice(&candidates);
    }
    let copy_elapsed = start.elapsed();

    let frames = FRAMES as f64;
    let candidate_checks = frames * candidates.len() as f64;
    let visible_per_frame = visible_total as f64 / frames;

    println!("candidates: {}", candidates.len());
    println!("visible/frame: {visible_per_frame:.0}");
    println!(
        "cull+list: {:.1} ns/candidate, {:.1} ns/visible",
        cull_elapsed.as_nanos() as f64 / candidate_checks,
        cull_elapsed.as_nanos() as f64 / black_box(visible_total).max(1) as f64,
    );
    println!(
        "copy-only: {:.1} ns/candidate, entries {}",
        copy_elapsed.as_nanos() as f64 / candidate_checks,
        all.len(),
    );
}

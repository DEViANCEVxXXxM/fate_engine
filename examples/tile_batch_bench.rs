//! Windowless stress test for the shared textured-quad renderer.

use std::error::Error;
use std::time::{Duration, Instant};

use fate_engine::math::Vec2;
use fate_engine::renderer::{
    MAX_DRAWN_TILES, MAX_TILE_LAYERS, OffscreenRenderer, OffscreenScene, Overlay, RendererOptions,
    TextureOptions, TileLayer2d, TileMap2d,
};

const WARMUP: usize = 30;
const FRAMES: usize = 180;

fn main() -> Result<(), Box<dyn Error>> {
    let mut renderer = pollster::block_on(OffscreenRenderer::new_with_options(
        (512, 512),
        RendererOptions { gpu_timing: true },
    ))?;
    if let Some(adapter) = renderer.adapter_info() {
        println!(
            "{} / {:?} / {}",
            adapter.name, adapter.backend, adapter.driver_info
        );
    }
    let atlas = renderer.create_texture_with_options(
        "tile batch atlas",
        2,
        1,
        &[25, 140, 110, 180, 180, 150, 40, 120],
        TextureOptions::NEAREST,
    )?;
    let regions = [atlas.region(0, 0, 1, 1), atlas.region(1, 0, 1, 1)];
    let map = TileMap2d::new(
        (64, 64),
        Vec2::new(-8.0, 8.0),
        Vec2::new(0.25, 0.25),
        (0..64 * 64)
            .map(|i| Some(((i + i / 64) % 2) as u16))
            .collect(),
    )?;
    let layer = TileLayer2d::new(&map, &atlas, &regions);
    let layers = [layer; MAX_TILE_LAYERS];
    let timeout = Duration::from_secs(5);
    renderer.wait_idle(timeout)?;
    println!("512x512 | {WARMUP} warmup + {FRAMES} samples per case | p50/p95 ms");
    for count in [1, MAX_TILE_LAYERS] {
        let mut submit = Vec::with_capacity(FRAMES);
        let mut gpu = Vec::with_capacity(FRAMES);
        for frame in 0..WARMUP + FRAMES {
            let start = Instant::now();
            let result = renderer.render(
                OffscreenScene::Tiles {
                    layers: &layers[..count],
                    sprites: &[],
                },
                &Overlay::default(),
            );
            let elapsed = start.elapsed();
            assert_eq!(
                (result.instances, result.draw_calls),
                (count * MAX_DRAWN_TILES, 1)
            );
            renderer.wait_idle(timeout)?;
            let samples = renderer.poll_gpu_timings();
            if frame >= WARMUP {
                submit.push(elapsed);
                gpu.extend(samples.into_iter().map(|sample| sample.scene));
            }
        }
        println!(
            "{count} layer(s) | {} quads | 1 draw",
            count * MAX_DRAWN_TILES
        );
        print_times("CPU render/submit (no GPU wait)", &mut submit);
        print_times("GPU scene", &mut gpu);
    }
    Ok(())
}

fn print_times(label: &str, samples: &mut [Duration]) {
    if samples.is_empty() {
        println!("  {label}: unavailable");
        return;
    }
    samples.sort_unstable();
    let ms =
        |percent: usize| samples[(samples.len() * percent).div_ceil(100) - 1].as_secs_f64() * 1e3;
    println!(
        "  {label}: {:.4}/{:.4} ({} samples)",
        ms(50),
        ms(95),
        samples.len()
    );
}

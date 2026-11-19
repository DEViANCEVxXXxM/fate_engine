//! Render a scene without a window and save it as an uncompressed PPM image.

use std::error::Error;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::time::Duration;

use fate_engine::math::{Mat4, Vec2, Vec3};
use fate_engine::renderer::{Draw3d, OffscreenRenderer, OffscreenScene, Overlay, TextDraw, UiRect};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let output = args.next().ok_or("usage: offscreen_capture <output.ppm>")?;
    if args.next().is_some() {
        return Err("usage: offscreen_capture <output.ppm>".into());
    }
    let mut renderer = pollster::block_on(OffscreenRenderer::new((641, 360)))?;
    renderer.camera_3d.position = Vec3::new(0.0, 0.0, 4.5);
    let cube = renderer.cube_mesh()?;
    let draws = [
        Draw3d {
            mesh: &cube,
            model: Mat4::translation(Vec3::new(-0.8, -0.12, 0.0))
                .mul(Mat4::rotation_y(0.5))
                .mul(Mat4::rotation_x(-0.35)),
            tint: [1.0, 0.65, 0.3],
        }
        .into(),
        Draw3d {
            mesh: &cube,
            model: Mat4::translation(Vec3::new(0.8, -0.12, 0.0))
                .mul(Mat4::rotation_y(-0.6))
                .mul(Mat4::rotation_x(-0.35)),
            tint: [0.36, 0.95, 1.0],
        }
        .into(),
    ];
    let rects = [UiRect::new(
        Vec2::new(14.0, 14.0),
        Vec2::new(266.0, 32.0),
        [0.1, 0.16, 0.2, 1.0],
    )];
    let text = [TextDraw::new(Vec2::new(24.0, 22.0), "FATE OFFSCREEN CAPTURE").with_scale(2.0)];
    renderer.capture(
        OffscreenScene::Meshes(&draws),
        &Overlay {
            rects: &rects,
            text: &text,
            ..Default::default()
        },
    )?;
    let frame = renderer
        .wait_capture(Duration::from_secs(10))?
        .ok_or("no completed capture")?;
    let output = std::path::Path::new(&output);
    let mut writer = BufWriter::new(
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(output)?,
    );
    writeln!(writer, "P6\n{} {}\n255", frame.width, frame.height)?;
    for pixel in frame.rgba8.as_chunks::<4>().0 {
        writer.write_all(&pixel[..3])?;
    }
    writer.flush()?;
    println!(
        "Captured {}x{} ({} instances, {} draw) to {}",
        frame.width,
        frame.height,
        frame.instances,
        frame.draw_calls,
        output.display()
    );
    Ok(())
}

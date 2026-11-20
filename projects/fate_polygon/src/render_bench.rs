//! Windowless GPU benchmark: isolated scene passes, no present or pixel readback.

use std::error::Error;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use fate_engine::renderer::{
    OffscreenRenderer, OffscreenScene, OffscreenSubmission, Overlay, RendererOptions,
};

use crate::bench_io::{Percentiles, open_csv};
use crate::cli::{Case, GpuBench};
use crate::scene::{append_stress_grid, deck_geometry, grid_light, set_grid_camera};

const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

struct Row {
    cpu_prepare: Percentiles,
    cpu_submit: Percentiles,
    gpu_scene: Option<Percentiles>,
    gpu_samples: usize,
    visible: usize,
    submitted: OffscreenSubmission,
}

pub fn run(bench: GpuBench) -> Result<(), Box<dyn Error>> {
    let mut renderer = pollster::block_on(OffscreenRenderer::new_with_options(
        bench.size,
        RendererOptions { gpu_timing: true },
    ))?;
    set_grid_camera(&mut renderer.camera_3d, 0.0);
    renderer.mesh_light = grid_light();
    let (name, backend, driver) = renderer.adapter_info().map_or_else(
        || (String::new(), String::new(), String::new()),
        |info| {
            (
                info.name.clone(),
                format!("{:?}", info.backend),
                info.driver_info.clone(),
            )
        },
    );
    let white = renderer.create_texture("polygon white albedo", 1, 1, &[255; 4])?;
    let mut csv = open_csv(bench.csv.as_deref())?;
    if let Some(csv) = &mut csv {
        writeln!(
            csv,
            "case,preset,sweep_setting,sweep_value,width,height,adapter,backend,driver,warmup_frames,measured_frames,candidates,detail,batch,cull,after_cull,instances,draw_calls,submitted_triangles,cpu_prepare_p50_ms,cpu_prepare_p95_ms,cpu_submit_p50_ms,cpu_submit_p95_ms,gpu_scene_p50_ms,gpu_scene_p95_ms,gpu_samples"
        )?;
    }

    let mut out = io::stdout().lock();
    writeln!(
        out,
        "fate_polygon | windowless render (no presentation, HUD or pixel readback)"
    )?;
    writeln!(
        out,
        "{} / {} | {}x{} | {} warmup + {} measured frames per case",
        name, backend, bench.size.0, bench.size.1, bench.warmup, bench.frames
    )?;
    if !renderer.gpu_timing_available() {
        writeln!(
            out,
            "GPU timestamps are unavailable; reporting CPU measurements only."
        )?;
    }
    writeln!(
        out,
        "p50/p95 ms per frame per phase; GPU waits excluded. Not an FPS benchmark."
    )?;

    for (i, case) in bench.cases.iter().enumerate() {
        let s = case.settings;
        let (vertices, indices) = deck_geometry(s.detail);
        let mesh = renderer.create_mesh_with_uvs("polygon load tile", &vertices, &indices)?;
        // Resource creation is outside the per-frame samples.
        renderer.wait_idle(WAIT_TIMEOUT)?;
        let mut draws = Vec::with_capacity(s.candidates as usize);
        let mut prep_times = Vec::with_capacity(bench.frames);
        let mut submit_times = Vec::with_capacity(bench.frames);
        let mut gpu_times = Vec::with_capacity(bench.frames);
        let mut counts = OffscreenSubmission {
            instances: 0,
            draw_calls: 0,
        };
        for frame in 0..bench.warmup + bench.frames {
            let prepare = Instant::now();
            let frustum = s
                .cull
                .then(|| renderer.camera_3d.frustum(renderer.aspect()));
            draws.clear();
            append_stress_grid(s, frustum, &mesh, &white, &mut draws);
            let prepared = prepare.elapsed();
            let submit = Instant::now();
            let submitted = renderer.render(OffscreenScene::Meshes(&draws), &Overlay::default());
            let submission_time = submit.elapsed();
            renderer.wait_idle(WAIT_TIMEOUT)?;
            let timestamps = renderer.poll_gpu_timings();
            if frame >= bench.warmup {
                prep_times.push(prepared);
                submit_times.push(submission_time);
                gpu_times.extend(timestamps.into_iter().map(|sample| sample.scene));
                counts = submitted;
            }
        }
        let row = Row {
            cpu_prepare: Percentiles::from_samples(prep_times).expect("nonempty measurement"),
            cpu_submit: Percentiles::from_samples(submit_times).expect("nonempty measurement"),
            gpu_samples: gpu_times.len(),
            gpu_scene: Percentiles::from_samples(gpu_times),
            visible: draws.len(),
            submitted: counts,
        };
        print_row(&mut out, i + 1, bench.frames, case, &row)?;
        if let Some(csv) = &mut csv {
            csv_row(csv, i + 1, &bench, case, &row, [&name, &backend, &driver])?;
        }
    }
    if let Some(mut csv) = csv {
        csv.flush()?;
        writeln!(out, "CSV: {}", bench.csv.expect("CSV path").display())?;
    }
    Ok(())
}

fn print_row(
    out: &mut impl Write,
    index: usize,
    frames: usize,
    case: &Case,
    row: &Row,
) -> io::Result<()> {
    let s = case.settings;
    writeln!(
        out,
        "\nCase {index}: detail {} | batch {} | cull {}",
        s.detail, s.batch, s.cull
    )?;
    writeln!(
        out,
        "  {} candidates -> {} after cull -> {} submitted | {} draws | {} submitted triangles",
        s.candidates,
        row.visible,
        row.submitted.instances,
        row.submitted.draw_calls,
        row.submitted.instances * s.detail as usize * s.detail as usize * 2
    )?;
    for (label, p) in [
        ("CPU prepare", row.cpu_prepare),
        ("CPU submit", row.cpu_submit),
    ] {
        writeln!(out, "  {label:<12} {:>9.4} / {:<9.4}", p.p50, p.p95)?;
    }
    if let Some(p) = row.gpu_scene {
        writeln!(
            out,
            "  GPU scene    {:>9.4} / {:<9.4} ({}/{frames} timestamps)",
            p.p50, p.p95, row.gpu_samples
        )
    } else {
        writeln!(out, "  GPU scene    unavailable (0/{frames} timestamps)")
    }
}

fn csv_row(
    out: &mut impl Write,
    index: usize,
    bench: &GpuBench,
    case: &Case,
    row: &Row,
    adapter: [&str; 3],
) -> io::Result<()> {
    let s = case.settings;
    let (setting, value) = case.swept.map_or(("", String::new()), |(name, value)| {
        (name, value.to_string())
    });
    write!(
        out,
        "{index},{},{setting},{value},{},{}",
        bench.preset.name(),
        bench.size.0,
        bench.size.1
    )?;
    for value in adapter {
        write!(out, ",\"{}\"", value.replace('"', "\"\""))?;
    }
    write!(
        out,
        ",{},{},{},{},{},{},{},{},{},{},{:.6},{:.6},{:.6},{:.6}",
        bench.warmup,
        bench.frames,
        s.candidates,
        s.detail,
        s.batch,
        s.cull,
        row.visible,
        row.submitted.instances,
        row.submitted.draw_calls,
        row.submitted.instances * s.detail as usize * s.detail as usize * 2,
        row.cpu_prepare.p50,
        row.cpu_prepare.p95,
        row.cpu_submit.p50,
        row.cpu_submit.p95
    )?;
    if let Some(p) = row.gpu_scene {
        write!(out, ",{:.6},{:.6}", p.p50, p.p95)?;
    } else {
        write!(out, ",,")?;
    }
    writeln!(out, ",{}", row.gpu_samples)
}

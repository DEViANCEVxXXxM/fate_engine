//! Windowed regression for the renderer's resize and minimize/restore path.

use std::time::{Duration, Instant};

use fate_engine::math::{Mat4, Vec3};
use fate_engine::renderer::{Draw3d, Instance2d, Mesh3d, Overlay, Renderer};
use winit::dpi::PhysicalSize;
use winit::window::Window;

#[derive(Debug)]
enum Stage {
    Initial,
    Resizing(PhysicalSize<u32>),
    RestoringZeroSize,
    Minimizing,
    RestoringWindow,
    Complete,
}

pub(crate) struct Lifecycle {
    stage: Stage,
    deadline: Instant,
}

impl Lifecycle {
    pub(crate) fn new() -> Self {
        Self {
            stage: Stage::Initial,
            deadline: Instant::now() + Duration::from_secs(12),
        }
    }

    pub(crate) fn is_complete(&self) -> bool {
        matches!(self.stage, Stage::Complete)
    }

    pub(crate) fn tick(&mut self, window: &Window, renderer: &mut Renderer, mesh: &Mesh3d) {
        assert!(
            Instant::now() < self.deadline,
            "timed out during surface lifecycle stage {:?} (window size {:?}, renderer size {:?}, minimized {:?})",
            self.stage,
            window.inner_size(),
            renderer.surface_size(),
            window.is_minimized()
        );

        let draw = |renderer: &mut Renderer| {
            let submitted_2d = renderer.render(&[Instance2d::default()]);
            let submitted_3d = renderer.render_meshes_with_overlay_report(
                &[Draw3d {
                    mesh,
                    model: Mat4::translation(Vec3::new(0.0, 0.0, -7.0)),
                    tint: [1.0; 3],
                }],
                &Overlay::default(),
            );
            if let Some(report) = submitted_3d {
                assert_eq!((report.instances, report.draw_calls), (1, 1));
            }
            (submitted_2d, submitted_3d.is_some())
        };

        match self.stage {
            Stage::Initial => {
                if draw(renderer) == (true, true) {
                    let initial = window.inner_size();
                    let target = PhysicalSize::new(initial.width * 3 / 4, initial.height * 3 / 4);
                    let expected = match window.request_inner_size(target) {
                        Some(size) => {
                            renderer.resize(size);
                            size
                        }
                        None => target,
                    };
                    assert_ne!(initial, expected, "resize did not change the window size");
                    self.stage = Stage::Resizing(expected);
                }
            }
            Stage::Resizing(expected) => {
                if window.inner_size() == expected
                    && renderer.surface_size() == (expected.width, expected.height)
                    && draw(renderer) == (true, true)
                {
                    println!("surface lifecycle: resized to {expected:?}; 2D and 3D submitted");
                    renderer.resize(PhysicalSize::new(0, expected.height));
                    assert_eq!(draw(renderer), (false, false), "zero-size draw submitted");
                    assert_eq!(renderer.surface_size(), (expected.width, expected.height));
                    renderer.resize(expected);
                    self.stage = Stage::RestoringZeroSize;
                }
            }
            Stage::RestoringZeroSize => {
                if draw(renderer) == (true, true) {
                    println!("surface lifecycle: zero-size skipped; same-size restore submitted");
                    window.set_minimized(true);
                    self.stage = Stage::Minimizing;
                }
            }
            Stage::Minimizing => {
                if window.is_minimized() == Some(true) {
                    assert_eq!(draw(renderer), (false, false), "minimized draw submitted");
                    println!("surface lifecycle: minimized; 2D and 3D skipped");
                    window.set_minimized(false);
                    self.stage = Stage::RestoringWindow;
                }
            }
            Stage::RestoringWindow => {
                let size = window.inner_size();
                if window.is_minimized() == Some(false) && size.width > 0 && size.height > 0 {
                    // Some platforms do not emit a resize when restoring to the same size.
                    renderer.resize(size);
                    if draw(renderer) == (true, true) {
                        self.stage = Stage::Complete;
                    }
                }
            }
            Stage::Complete => {}
        }
    }
}

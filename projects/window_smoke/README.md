# fate_window_smoke

A minimal windowed consumer used to validate the renderer's public seam. It is
deliberately independent from `fate_demo`:

- it owns its own `winit` event loop and window;
- it initializes `Renderer` and lets the renderer own the surface and GPU
  resources;
- it uploads colored and UV-mapped meshes through `Renderer::create_mesh`
  and `Renderer::create_mesh_with_uvs` instead of naming a `wgpu::Device`;
- it submits a 3D draw list and handles resize events;
- it uploads a procedural two-tile atlas through
  `Renderer::create_texture_with_options`, reuses it on a 3D mesh and in the
  HUD, draws both atlas regions with
  nearest filtering, tiles one region across a strip using repeat addressing,
  rotates one icon, crops an oversized tile with `UiClip`, and wraps a paragraph
  with `wrap_text`; `render_scene` submits the meshes and HUD together;
- it exits after a fixed number of redraws, so it can run unattended.

Its manifest depends on `fate_engine`, `pollster`, and `winit` only. There is no
direct `wgpu` dependency: the renderer's public API is sufficient for window
creation, surface ownership, GPU resource creation, and render submission.

Run from the repository root:

```powershell
cargo run -p fate_window_smoke
```

The process prints the number of submitted and skipped frames when it exits.
This is a seam check, not a benchmark; it makes no performance claims.

### Surface lifecycle regression

```powershell
cargo run --release --locked -p fate_window_smoke -- --surface-lifecycle
```

This mode resizes the actual window, feeds the renderer a zero-width size and
restores the same dimensions, then minimizes and restores the actual window.
It checks that 2D and 3D both skip frames while suspended and draw again
afterwards. A failed check or timeout exits nonzero. This mode needs a desktop
session, a usable GPU adapter, and a window system that reports minimization
through `winit`. It has been verified on Windows and is not part of headless CI.

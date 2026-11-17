# fate_window_smoke

A minimal windowed consumer used to validate the renderer's public seam. It is
deliberately independent from `fate_demo`:

- it owns its own `winit` event loop and window;
- it initializes `Renderer` and lets the renderer own the surface and GPU
  resources;
- it uploads a mesh through `Renderer::create_mesh` instead of naming a
  `wgpu::Device`;
- it submits a 3D draw list and handles resize events;
- it uploads a procedural two-tile atlas through
  `Renderer::create_texture_with_options`, draws both atlas regions with
  nearest filtering, tiles one region across a strip using repeat addressing,
  and crops an oversized tile with `UiClip` through
  `render_meshes_with_overlay`, exercising the public overlay layers the combat
  slice can use for text;
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

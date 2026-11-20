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
The default run is a seam check, not a performance benchmark.

### Optional GPU timings

Use `--flat-lighting` to shade the rotating colored and textured pyramids
with a world-space directional light. Omit the flag for the unlit comparison.
The flag also works with `--mesh-load` for an instanced comparison.

```powershell
cargo run --release --locked -p fate_window_smoke -- --flat-lighting --gpu-timing --dbg
```

Use `--smooth-lighting` to compare three rotating spheres with matching
geometry: flat on the left, smooth with an atlas texture in the center, and
smooth without a texture on the right.

```powershell
cargo run --release --locked -p fate_window_smoke -- --smooth-lighting --gpu-timing --dbg
```

Use `--depth-fog` to see seven rows of lit cubes fade into the background.
The scene briefly switches fog off for comparison. With `--dbg`, it checks
that both states submit the same 14 instances in one indexed draw.

```powershell
cargo run --release --locked -p fate_window_smoke -- --depth-fog --gpu-timing --dbg
```

Use `--mesh-picking` to highlight the closest pyramid under the pointer.
The HUD shows the triangle number and world distance. When the pointer is
outside the window the probe uses screen center, where two pyramids overlap.

```powershell
cargo run --release --locked -p fate_window_smoke -- --mesh-picking --gpu-timing --dbg
```

Use `--dense-picking` for the same pointer probe on four instances of a
16,128-triangle sphere. The HUD reports exact triangle tests across the
instances. Startup checks the overlapping center hit and prints its work
counts; `--dbg` verifies all four instances still draw in one indexed call.

```powershell
cargo run --release --locked -p fate_window_smoke -- --dense-picking --gpu-timing --dbg
```

Use `--scene-picking` to probe two overlapping grids of 182 total pyramids.
The HUD shows the nearest instance and triangle, then lists CPU scene bounds
and instance counts separately from CPU mesh bounds and triangle counts. These
are work counts, not GPU timings. The center probe checks that the front grid
wins; `--dbg` verifies that rendering still uses one indexed draw.

```powershell
cargo run --release --locked -p fate_window_smoke -- --scene-picking --gpu-timing --dbg
```

Use `--2d` to run the same HUD showcase over instanced 2D triangles instead
of meshes. With GPU timing enabled, the smoke checks both 2D frames with an
overlay pass and frames without one:

```powershell
cargo run --release --locked -p fate_window_smoke -- --2d --gpu-timing --dbg
```

Use `--sprites` to exercise world-space atlas sprites with camera movement
and a fixed HUD. A rectangular in-place update animates part of the atlas
shown by both layers. Diagnostics check that three valid sprites sharing
the atlas submit in one draw and that the invalid fourth record is skipped.
The consumer culls an additional offscreen sprite before submission. Move
the pointer over the sprites to highlight the last drawn quad under it.

```powershell
cargo run --release --locked -p fate_window_smoke -- --sprites --gpu-timing --dbg
```

Use `--tiles` to pan and zoom across three atlas-backed layers: a
screen-fixed backdrop, a 128×96 world-fixed ground with empty cells and live
edits, and a sparse detail layer. A moving sprite draws above every layer.
Diagnostics report the combined input count, visible tiles submitted, draw
count, and separate tilemap/HUD pass timings.

```powershell
cargo run --release --locked -p fate_window_smoke -- --tiles --gpu-timing --dbg
```

Add `--dbg` to open the reusable native diagnostics companion alongside this
consumer, including with `--mesh-load` or `--surface-lifecycle`:

```powershell
cargo run --release --locked -p fate_window_smoke -- --mesh-load --dbg
```

The companion attaches directly to the renderer; the smoke app does not build
project-specific inspection UI. This validates the same integration available
to other engine projects. `--dbg --gpu-timing` also checks that the existing
GPU polling API and the diagnostics subscription can coexist.

```powershell
cargo run --release --locked -p fate_window_smoke -- --gpu-timing
cargo run --release --locked -p fate_window_smoke -- --mesh-load --gpu-timing
```

The first command reports separate 3D and overlay pass durations. The second
reports the 3D pass for the 256-instance load alongside its CPU timings. GPU
samples exclude surface acquisition, presentation, and time between passes.
If the adapter does not support timestamp queries, the smoke run still draws
and prints that GPU timing is unavailable.

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

# Engine improvement roadmap

> **Status:** Working plan, reviewed 2026-11-11. This is a sequence of small
> engineering steps, not a release schedule or a stable API promise.

## Current baseline

- `fate_engine` is a library providing generic input, frame/fixed-step timing,
  floating-point math, and `wgpu`/`winit` rendering foundations.
- Dependency-free `fate_sim` provides fixed-point scalar/vector math, a seeded
  RNG, generational entity handles with slot reuse, fixed component columns,
  and a rule-neutral state container with versioned snapshots.
- Concrete commands, tick policy, replay, and rollback belong to the consumer
  projects (`projects/demo` and `projects/combat_slice`); they are not reusable
  `fate_sim` APIs.
- `fate_net` provides a transport-agnostic local protocol and ordered
  client/server sessions. It does not provide sockets, authentication,
  encryption, clock synchronization, or prediction/reconciliation.
- The renderer supports built-in 2D triangles, indexed 3D meshes with depth
  testing and opaque color textures, cameras, visibility helpers, screen-space
  UI rectangles and textured images, and a bitmap-font text overlay with alpha
  blending.
  Consumers own scene traversal, culling decisions, asset loading, and HUD
  layout; there is no general asset, material, or lighting system.
- `fate_engine::audio` plays bounded procedural one-shot tones through the
  platform output device and degrades to silence when no device is available.
  Consumers own the mapping from events to sounds; there are no audio assets,
  spatialization, or mixer buses.
- Headless and windowed smoke consumers exercise the public APIs independently
  of the demo. CPU benchmarks exist, but do not establish GPU performance.
- `fate_engine::diagnostics` provides rolling frame reports, budget
  classification, and metric sink/source seams. `fate_sim` and `fate_net`
  expose read-only inspection snapshots. Presentation and export remain
  consumer-owned.

## Step 1 — Harden ECS lifecycle and snapshot restoration

**Goal:** Make entity/component invariants hold at every public entry point.

**Status: done.** Component columns are private; consumers read through
`World::positions()` and related accessors, attach components through checked
setters, and update attached values through checked `*_mut` accessors. World
snapshot decoding rejects duplicate free-list IDs and free lists that do not
cover every dead slot exactly once. Regression tests cover dead and stale
handles, repeated slot reuse, and snapshot round trips after reuse.

- Reject duplicate free-list IDs during world snapshot decoding. Check that
  the free list covers every dead slot exactly once.
- Prevent direct component-column mutations from bypassing world lifecycle
  checks. Preserve useful read access and controlled value updates.
- Add regression tests for duplicate free IDs, stale/dead entity insertion,
  repeated slot reuse, and snapshot round trips after those operations.

**Done when:** Malformed snapshots are rejected, public APIs cannot attach
components to dead/stale entities, and valid restored worlds allocate distinct
live handles.

## Step 2 — Define snapshot tick consistency in the local protocol

**Depends on:** Step 1.

**Status: done.** The client requires the packet's `server_tick` to equal the
tick encoded in the restored state and rejects contradictions before changing
session state. Regression tests cover inconsistent metadata, stale snapshots,
rejected packets leaving the session unchanged, and recovery by a subsequent
valid packet.

- Document the relationship between packet `server_tick` and the tick encoded
  in `SimulationState`; reject contradictions before accepting a snapshot.
- Test inconsistent ticks, stale snapshots, and recovery by a subsequent valid
  packet. Verify that a rejected packet leaves the client session unchanged.
- Keep checksums explicitly diagnostic; these checks do not authenticate peers.

**Done when:** Accepted snapshots have an explicit, enforced tick contract and
invalid metadata cannot advance the client's accepted server tick.

## Diagnostics foundation

**Status: done.** The demo's frame telemetry moved into
`fate_engine::diagnostics`, preserving the existing report fields and text
output. The module also defines structured sink/source seams for future
exporters. `fate_sim` exposes world and simulation snapshots, and `fate_net`
exposes client/server session snapshots; all are read-only inspection data.

- Keep frame reporting reusable while leaving overlays, HUDs, and external
  exporters to consumers.
- Provide read-only snapshots for future renderer, simulation, and networking
  tooling without adding speculative dependencies.
- Demonstrate at least one source implementation in the demo and cover the
  snapshot APIs with tests.

**Done when:** Consumers can obtain frame and session diagnostics without
duplicating demo-only telemetry, and the engine exposes no presentation policy.

## Step 3 — Automate the quality gate

**Depends on:** Steps 1–2 for the new regression cases; existing checks can be
added to CI independently.

**Status: done.** `.github/workflows/ci.yml` runs formatting, strict Clippy,
workspace tests, and the headless consumer smoke check on pull requests, branch
pushes, and manual dispatches. The workflow uses the stable Rust toolchain on
`ubuntu-latest`; the local baseline and the separation from windowed/GPU checks
are documented in the engine guide.

- Add CI for `cargo test --workspace`, `cargo fmt --all -- --check`, and
  `cargo clippy --workspace --all-targets -- -D warnings`.
- Run the headless smoke consumer as an integration check.
- Document the tested toolchain/platform. Keep windowed/GPU checks separate
  from headless CI and record them when renderer changes require them.

**Done when:** The automated gate runs on pull requests and branch updates;
required CPU-only checks do not need a window or GPU adapter.

## Step 4 — Clarify timing and performance measurements

**Status: done.** `Time::frame_period_ms()` exposes the exact 120 Hz period,
while `frame_budget_ms()` is documented and tested as the conservative integer
scheduling value. Presentation-delta clamping, retained backlog, and the
per-frame step cap are documented and covered by tests.
`FrameSample`/`FrameReport` now separate simulation, CPU draw-list preparation,
and CPU render submission; GPU execution is explicitly unmeasured. The
release-mode `draw_list_bench` provides the repeatable CPU draw-list baseline.

- Distinguish the exact 120 Hz period (approximately 8.333 ms) from the current
  conservative integer 8 ms scheduling/budget value.
- Document presentation-delta clamping separately from retained simulation
  backlog and its per-frame processing cap.
- Keep CPU simulation, draw-list preparation, render submission, and GPU
  execution measurements distinct. Add a repeatable submission baseline before
  choosing batching or instancing.

**Done when:** Timing reports state what they measure, budget semantics are
explicit and tested, and performance claims include workload/build conditions.

## Step 5 — Validate with a small playable consumer

**Depends on:** Steps 1–3; use Step 4 measurements where relevant.

**Status: done.** [`projects/combat_slice/`](../projects/combat_slice/) is a
small playable first-person arena scenario with movement, mouse look, hitscan
shooting, enemy and player health, and explicit victory/defeat conditions. Its
rules, input mapping, scene content, and presentation live in the project; no
gameplay policy moved into a reusable crate. The simulation is fixed-step and
fixed-point, and the project verifies a 240-tick replay window and periodic
snapshot round trips against the live state. Headless tests cover the rules,
replay, snapshot codec, interpolation, culling, and app behavior.

The slice recorded three concrete gaps. First, `fate_sim` had no fixed-point
trigonometric functions, so the project initially rotated aim with local small-angle
polynomials. This gap is now resolved: `fate_sim::math` provides deterministic
`sin_cos()`, `FixedQuat`, and `FixedTransform`, and `combat_slice` was updated to
use engine trigonometry. Second, the engine had no text or HUD renderer, so the
project initially presented player and verification state through the window
title and `log`. This gap is now resolved: the renderer provides `TextDraw`,
`text_width`, and `render_meshes_with_text` with a built-in bitmap font, and the
slice draws its HUD in the window while keeping the title and log output. Third,
combat events had no audible feedback. This gap is now resolved:
`fate_engine::audio` plays procedural one-shot tones through a bounded mixer,
and the slice maps shots, hits, kills, damage, reloads, and outcomes to sounds
without letting audio affect deterministic state.

- Choose one compact original gameplay scenario with movement, an interaction,
  and a clear success/failure condition.
- Keep its rules, commands, and content in a project, not in reusable crates.
- Exercise state save/restore and deterministic replay of the chosen scenario.
- Record actual missing engine capabilities and implement the smallest one
  needed to complete the scenario.

**Done when:** The scenario is playable through documented public APIs, its
simulation is regression-tested, and the next engine task has a concrete
consumer rather than a speculative abstraction.

## Step 6 - Compose HUD overlays without backend types

**Depends on:** Step 5. Status: **done.**

The text overlay covered labels but not the surface behind them, so consumers
either accepted lower contrast over bright scenes or built meshes just to draw a
panel. `Renderer::render_meshes_with_overlay` now composites a borrowed
`Overlay`: screen-space rectangles, textured images, then text, all in one
surface submission. Positions use the same physical-pixel, top-left origin as
`TextDraw`; alpha blending supports translucent panels, and list order controls
layering. Rectangle and image counts are bounded, and invalid geometry is
skipped.

`Renderer::create_texture` adds a raw RGBA8 sRGB upload path with validation, so
a HUD can show real artwork without an asset pipeline or direct `wgpu` names.
`TextureRegion` and `Texture2d::region` select atlas tiles from one upload, and
`TextureOptions` chooses nearest filtering for crisp pixel art or repeat
addressing for tiled regions. `UiClip` bounds overlay records, so text and
artwork can be cropped to a panel or scrolling viewport, `UiImage` rotation
turns icons into spinners or angled indicators, and `wrap_text` breaks HUD copy
into lines that fit a pixel column. There are no decoders, mipmaps, or atlas
packers. The handle is consumer-owned and must
outlive any `UiImage` that references it. `projects/window_smoke` exercises the
seam with a two-tile procedural atlas, a nearest-filtered tiled
strip, a rotated icon, a clipped preview, a wrapped paragraph, a panel, an
accent bar, and text.

- Keep the common overlay path free of direct `wgpu` names.
- Use one surface acquisition and submission for meshes and every overlay layer.
- Test coordinate conversion, texture validation, and invalid input headlessly,
  plus offscreen GPU visibility and sampling checks that skip without an adapter.

**Done when:** A windowed consumer can place readable HUD content over arbitrary
scene colors without uploading a mesh or naming backend types.

## Step 7 — Draw opaque textured 3D meshes

**Status: done.** `window_smoke` uses one RGBA8 texture for a UV-mapped mesh and
HUD images. `render_scene` draws textured and untextured meshes with shared
depth and a shared draw budget, then composes the HUD in the same submission.
Transparent 3D materials and a general material system remain deferred.

## Step 8 — Recover drawing after window changes

**Status: done.** The renderer skips frames while the window is not drawable
and resumes after resize or restore. `window_smoke --surface-lifecycle`
provides a separate windowed regression for 2D and 3D. GPU device loss recovery
remains deferred.

## Step 9 — Batch built-in 2D triangles

**Status: done.** `Renderer::render` streams the model matrices for the built-in
triangles together, shares one camera uniform, and issues one instanced draw
in input order. Empty lists clear and present without issuing a draw. The
256-instance budget is unchanged. `window_smoke --surface-lifecycle --dbg`
checks submitted, empty, and skipped batch counts through renderer telemetry.

## Step 10 — Batch atlas-backed overlay images

**Status: done.** The image layer combines adjacent quads with the same
texture and scissor without changing alpha-blend order. The upload and draw
paths use the same accepted quad ranges. Offscreen tests check blend order,
texture and clip boundaries, invalid geometry, and the 64-image limit.

## Step 11 — Compose overlays over 2D scenes

**Status: done.** 2D and 3D now share one overlay path. `Renderer::render_with_overlay`
supports 2D scenes and overlay-only frames without a custom render pass or
direct `wgpu` calls. GPU timing samples the overlay only when a pass is recorded.
`window_smoke --2d --gpu-timing` exercises scene-plus-overlay, overlay-only,
and frames whose overlay inputs produce no draws.

## Step 12 — Draw textured world-space sprites

**Status: done.** `Sprite2d` reuses the existing texture, atlas, and sampling
options with the orthographic camera. The renderer batches in list order,
preserves alpha blending, and keeps the screen-space overlay separate.
`window_smoke --sprites --dbg --gpu-timing` exercises camera pan and zoom and
checks the batch counts through diagnostics.

## Step 13 — Update atlas pixels in place

**Status: done.** `Texture2d` supports full and rectangular RGBA8 updates
on its original queue. The existing texture bindings and atlas UVs are kept.
`window_smoke --sprites` animates part of one tile shared by the world scene
and HUD. An offscreen test checks unchanged neighboring texels and rejected
updates.

## Step 14 — Pick and cull world-space sprites

**Status: done.** `Camera2d` converts between world space and physical window
pixels using the render projection's pan, zoom, and aspect rules. Sprites
expose oriented bounds for picking and enclosing axis-aligned bounds for
conservative culling. `window_smoke --sprites` filters an offscreen sprite
before submission and highlights the last drawn sprite under the cursor.

## Step 15 — Render and capture without a window

**Status: done.** The windowed and offscreen renderers share scene and overlay
recording. `OffscreenRenderer` captures triangles, sprites, or depth-tested
meshes with optional HUD into tightly packed RGBA8 pixels. A reusable async
readback buffer limits the target to one pending capture. CPU tests validate
row alignment; offscreen GPU tests check compositing and depth. The
`offscreen_capture` example exports an image without a window.

## Step 16 — Render atlas-backed tilemaps

**Status: done.** A retained, editable `TileMap2d` holds an orthogonal
grid. Windowed and offscreen renderers traverse only the visible rows and
columns, batch one atlas per layer, and apply a dedicated visible-tile
limit. Sprites can draw above the map in the same scene pass.
`window_smoke --tiles --dbg --gpu-timing` pans and zooms over a
12,288-cell grid. Offscreen tests check atlas selection, edits, camera culling,
and the batch limit.

## Step 17 — Compose ordered tile layers

**Status: done.** `render_tile_layers` and
`render_tile_layers_with_sprites` draw up to eight layers back-to-front, with
camera-culled traversal for every layer. Each layer has an independent
4,096-tile budget and accepts a world-space offset plus a per-axis parallax
multiplier (`1` world-fixed, `0` screen-fixed). Layers sharing a texture batch together without
reordering. `window_smoke --tiles` shows a screen-fixed backdrop, a world-fixed
ground, a detail layer, and a sprite above all three. Offscreen tests check
 layer order, parallax placement, the layer cap, and per-layer budgets.

## Step 18 — Shade opaque meshes with one directional light

**Status: done.** Windowed and offscreen renderers can opt into flat
directional lighting with an ambient floor. The visible triangle supplies
the normal, so existing colored and textured geometry needs no vertex-data
changes. `window_smoke --flat-lighting` lights the rotating pyramids.
Offscreen tests verify lit and unlit pixels without changing the draw count.

## Step 19 — Smooth shading from vertex normals

**Status: done.** Windowed and offscreen renderers accept indexed meshes with
validated object-space normals. The same directional light now supports
smooth shading, including under nonuniform or mirrored scales, while meshes
without normals retain flat shading. `window_smoke --smooth-lighting` compares
flat and smooth shading on the same geometry, with a textured variant.

## Step 20 — Add depth fog to opaque 3D scenes

**Status: done.** Windowed and offscreen renderers can fade opaque meshes
into a matching background using forward camera depth. Fog blends after
lighting without a new pass. `window_smoke --depth-fog` alternates between
fogged and clear views of the same instanced scene. Offscreen tests check
partial and full fog, including textured meshes, and verify that the HUD
and 2D path remain unaffected.

## Step 21 — Pick 3D meshes without GPU readback

**Status: done.** Consumers can keep an optional `PickMesh3d` beside a
render mesh and reuse it across instances. A local bounds check filters
misses before the triangle query. `Camera3d::pick_ray` limits pointer
rays to the camera clip planes; a hit reports the triangle, world distance,
position, and barycentric weights. `window_smoke --mesh-picking` checks
selection with overlapping objects and highlights the closest hit.

## Step 22 — Accelerate picking on dense meshes

**Status: done.** `PickMesh3d` builds a balanced hierarchy of triangle
bounds once and reuses it across instances. CPU tests compare accelerated
hits to a direct scan and preserve source-triangle tie breaking.
`window_smoke --dense-picking` exercises four instances of one dense mesh
and displays exact triangle-test counts.

## Step 23 — Accelerate picking across many instances

**Status: done.** `PickScene3d` indexes world-space instance bounds and reuses
shared mesh triangle trees. Its CPU query returns the nearest instance and
triangle and reports how many instance and triangle queries it needed.
`window_smoke --scene-picking` exercises two overlapping instanced grids;
CPU tests compare against a direct instance scan.

## Step 24 — Measure rendering without a window or pixel readback

**Status: done.** `OffscreenRenderer` can submit a scene without allocating
a capture buffer. Optional timestamps report GPU pass durations.
`fate_polygon --offscreen` varies grid tessellation, candidate count, or
batching and reports CPU preparation, CPU submission, and GPU scene timings
separately.

## Deferred until a consumer needs them

- Arbitrary ECS components, composed queries, and parallel scheduling.
- General assets, materials, lighting, scene graphs, and renderer backends.
- Network transport, peer identity, encryption, clock synchronization, and
  multiplayer prediction/reconciliation. Define a threat model before adding
  production networking.

For each step, update the [engine guide](engine-guide.md) if observable behavior
changes and keep application policy outside reusable crates. Choose a license
before presenting the repository as ready for external adoption.

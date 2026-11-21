# Working with fate_engine

> **Status: early implementation.** This guide describes the checked-in
> workspace and APIs, not a stable release contract. Public interfaces,
> dependency boundaries, and behavior may change while the engine is being
> developed.

Use this guide to build the workspace, integrate the libraries, and verify an
application. For project status, see the [roadmap](roadmap.md).

- **Get started:** [Prerequisites and first run](#3-requirements-and-first-run) ·
  [Integrate a project](#4-use-the-engine-from-a-project)
- **Build:** [Input](#6-input) · [Fixed-step simulation](#7-time-and-deterministic-simulation) ·
  [Rendering](#8-rendering) · [Audio](#9-audio)
- **Inspect:** [Diagnostics](#10-errors-diagnostics-and-operational-notes) ·
  [Verification](#11-verification-and-examples) · [Benchmarks](#performance-measurement)

## 1. What the engine currently is

The codebase is a Rust workspace containing a renderer/platform-facing library,
a deterministic simulation crate, and a separate runnable demo project. The demo
is the first validation workload; the engine is not intended to hardcode any
specific application's rules.

| Area | Current state | Important limitation |
|---|---|---|
| Workspace/library | Root package `fate_engine` is a library | Experimental APIs; no published versioned integration contract |
| Simulation | `fate_sim` supplies fixed-point math, ECS, deterministic RNG, and a rule-neutral state/snapshot container | The consuming project owns tick policy, commands, systems, replay, and rollback |
| Input | Generic button/action state with press/release edges and analog axes (per-frame accumulated deltas) | Projects must translate their platform events and define their own actions, axes, and mappings |
| Rendering | Windowed and offscreen `wgpu` rendering: instanced 2D quads, tile layers, opaque 3D meshes, HUD overlays, and CPU picking | No interchangeable renderer backend or general scene, material, or asset system |
| Audio | Bounded procedural one-shot tones through the platform output device | Optional device; no assets, spatialization, looping, or mixer buses |
| Demo | Standalone app under `projects/demo` | Validation harness only; its scenes and controls are not engine guarantees |
| Networking | `fate_net` provides a versioned local protocol prototype with bounded payloads, ordered inputs, acknowledgement history, and snapshot validation | No transport, authentication, encryption, clock synchronization, or production server policy |

Use the primitives as an experimental starting point. Do not assume that
planned features in a design document are already implemented.

## 2. Workspace structure

```text
fate_engine/
  Cargo.toml                 # workspace root and fate_engine library package
  src/                       # reusable audio, input, math, renderer, and time APIs
  crates/
    fate_net/                # versioned local protocol prototype
    fate_sim/                # headless deterministic simulation crate
    fate_sim_bench/          # CPU-only simulation benchmark
    fate_debug/              # opt-in native diagnostics
  projects/
    combat_slice/            # small playable first-person consumer
    consumer_smoke/          # headless example consumer
    demo/                    # separate runnable consumer application
    fate_polygon/            # live, headless, and offscreen stress workloads
    window_smoke/            # window and renderer integration checks
  docs/
    engine-guide.md          # this guide
```

Responsibilities are intentionally split:

- **`fate_engine`** — generic input state, renderer-facing floating-point math,
  window/GPU renderer foundations, procedural audio output, and a frame/fixed-step
  clock.
- **`fate_sim`** — deterministic simulation primitives. It has no third-party
  dependencies and forbids unsafe code.
- **`fate_net`** — transport-agnostic packet encoding and client/server session
  state machines built on `fate_sim` snapshots. It does not open sockets.
- **`fate_demo`** — window/event loop, platform bindings, showcase scene setup,
  deterministic simulation driving, controls, and consumer-owned diagnostics
  presentation.
- **`fate_combat_slice`** — small playable first-person arena scenario. It owns
  its rules, input mapping, scene content, and log/title presentation, and uses
  the engine only through public APIs.
- **`fate_sim_bench`** — headless simulation measurements. Its timings do not
  measure rendering, GPU work, or full-frame performance.
- **[`fate_polygon`](../projects/fate_polygon/)** — repeatable render and
  simulation workloads. Physics and AI policy stay in the lab.
- **[`fate_debug`](../crates/fate_debug/)** — optional native companion for
  live frame data, inspection, and trace export.

Keep game-specific rules, content, action names, and scene setup in a project.
Keep reusable mechanisms in the engine crates only when they have a clear
consumer beyond the demo or form a general capability.

## 3. Requirements and first run

Use current stable Rust. The workspace uses Rust 2024; edition support alone
does not guarantee that an older toolchain can build the dependencies.
Windowed runs need a desktop session and a `wgpu`-compatible adapter.

On Debian or Ubuntu, install the development libraries needed for workspace
audio and controller support, then override the checkout's Windows-specific
build output path:

```sh
sudo apt-get install -y libudev-dev libasound2-dev
export CARGO_TARGET_DIR="$PWD/target"
```

These build dependencies do not require a connected audio or controller
device. Linux cross-compilation does not establish native desktop support;
see the [roadmap](roadmap.md#step-27--establish-a-cross-platform-validation-baseline).

From the repository root:

```sh
cargo run --release --locked -p fate_demo -- --dbg
cargo run --locked -p fate_consumer_smoke
cargo test --locked -p fate_sim
```

The [showcase guide](../projects/demo/) covers controls and the diagnostics
companion. The consumer smoke and `fate_sim` tests are headless. For full
workspace checks or CPU-only measurement, see [verification](#11-verification-and-examples).

## 4. Use the engine from a project

### Project outside this repository

The current integration method is a Cargo path dependency. Paths are relative
to the consuming project's `Cargo.toml`, not the shell's working directory:

```toml
[dependencies]
fate_engine = { path = "../path/to/fate_engine" }
fate_sim = { path = "../path/to/fate_engine/crates/fate_sim" }
```

Include only the crates the project needs. A renderer-only or input-only
consumer does not need `fate_sim`; a simulation-only tool does not need
`fate_engine`. Because this is a local path integration, consumers must have
the engine source available at that path. There is no published crate release
or semver compatibility guarantee yet.

To verify this integration from a scratch project, follow the
[standalone consumer recipe](../projects/consumer_smoke/#verify-outside-the-workspace).
It builds with its own manifest and lockfile; it does not depend on workspace
membership.

### Project inside this workspace

Add the project directory to `members` in the root `Cargo.toml`, create a
project-local manifest and executable, and use path dependencies like the
demo does:

```toml
[dependencies]
fate_engine = { path = "../.." }
fate_sim = { path = "../../crates/fate_sim" }
```

The workspace's `default-members` currently selects `projects/demo`, so plain
`cargo run` from the root runs the demo. The headless
[`fate_consumer_smoke`](../projects/consumer_smoke/README.md) package is an
independent example consumer and can be run without opening a window:

```powershell
cargo run -p fate_demo
cargo run -p fate_consumer_smoke
cargo test -p fate_engine
cargo test -p fate_sim
```

Keep the project's own `Cargo.toml`, entry point, configuration, content, and
tests under its directory. For a minimal headless consumer, start with
[`projects/consumer_smoke/`](../projects/consumer_smoke/); for windowed
rendering and platform integration, see [`projects/demo/`](../projects/demo/).

## 5. A project's runtime responsibilities

There is no engine-level `Game` trait or turnkey application bootstrap yet. A
consumer currently owns its platform integration and composes the available
pieces. The demo's [`app.rs`](../projects/demo/src/app.rs) is the most complete
reference for a `winit` event loop and renderer lifecycle.

A typical frame follows this order:

1. Receive platform events; update the project's button state.
2. Advance `Time` once to collect a clamped wall-clock delta.
3. Convert the current project actions into a simulation command snapshot.
4. Consume fixed steps up to the clock's per-frame limit, applying the
   appropriate command for each tick; leave any excess backlog queued.
5. Build renderer-facing transforms from simulation state.
6. Submit a render request and handle a skipped frame if the surface is
   temporarily unavailable.
7. Clear input edge state once, after all frame consumers have read it.

The demo additionally schedules redraws through `winit`, handles resize/focus,
captures the cursor for its 3D camera, and reports performance telemetry. Those
are demo implementation choices rather than a reusable engine application
shell.

## 6. Input

The public type is `fate_engine::input::Input<Action, Button, Axis>`. All three
type parameters must be `Copy + Eq + Hash`. `Axis` defaults to `()` for
callers that do not need analog axes. `Input` stores held buttons, edge state,
a many-to-many action map, and accumulated per-frame analog axis deltas. It
does not know about keys, gamepad APIs, `winit`, or game-specific action names.

### Digital buttons and actions

```rust
use fate_engine::input::Input;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum GameAction {
    Confirm,
}

fn main() {
    let mut input: Input<GameAction, u8> = Input::new();
    input.bind(GameAction::Confirm, 1);

    input.press(1); // call from the platform adapter's pressed event
    assert!(input.button_down(1));
    assert!(input.button_just_pressed(1));
    assert!(input.action_down(GameAction::Confirm));
    assert!(input.action_just_pressed(GameAction::Confirm));

    input.end_frame(); // only after every consumer has read the edge state
    assert!(!input.action_just_pressed(GameAction::Confirm));
}
```

Call `release(button)` for the corresponding platform release event. Use
`clear()` when the host loses focus to discard held buttons, pending edge
transitions, and axis deltas so controls cannot remain stuck. Repeated
presses while held do not create additional press edges. If several buttons
are bound to one action, the action is pressed when its first button becomes
held and released when its last held button is released. Adding/removing a
binding while a button is held does not synthesize a transition; establish
bindings before normal event processing.

### Analog axes

`update_axis(axis, delta)` accumulates a floating-point delta onto a named
axis for the current frame. Multiple calls per frame are summed so raw device
events (e.g. `winit::DeviceEvent::MouseMotion`) can be fed directly without
pre-aggregation. `axis_delta(axis)` returns the total accumulated value and is
`0.0` if the axis was not updated this frame.

Both `end_frame()` and `clear()` reset all axis deltas alongside button edge
state, so callers must read axes **before** calling `end_frame()`.

```rust
use fate_engine::input::Input;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum GameAction { Fire }

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Axis { MouseX, MouseY }

fn main() {
    let mut input: Input<GameAction, u8, Axis> = Input::new();

    // feed raw device events (may fire multiple times per frame)
    input.update_axis(Axis::MouseX, 3.0);
    input.update_axis(Axis::MouseX, -1.5);
    input.update_axis(Axis::MouseY, 7.0);

    assert!((input.axis_delta(Axis::MouseX) - 1.5).abs() < f32::EPSILON);
    assert!((input.axis_delta(Axis::MouseY) - 7.0).abs() < f32::EPSILON);

    input.end_frame(); // resets both button edges and axis deltas
    assert_eq!(input.axis_delta(Axis::MouseX), 0.0);
}
```

The demo's `Axis { MouseX, MouseY }` enum is defined in
[`demo_input.rs`](../projects/demo/src/demo_input.rs). `DeviceEvent::MouseMotion`
deltas are fed via `input.update_axis(...)` in `device_event`; the 3D camera
reads them via `input.axis_delta(...)` in `update_3d_camera` before
`end_frame()` clears them.

### Translation boundary

Translate physical input at the project boundary. The demo defines its own
actions, axes, and `winit::KeyCode` bindings in
[`demo_input.rs`](../projects/demo/src/demo_input.rs). For deterministic
simulation, convert those actions to a simple command value; do not pass
platform input objects into simulation code.

## 7. Time and deterministic simulation

### Fixed-step clock

`fate_engine::time::Time` tracks wall-clock frame delta and an accumulator.
`Time::new(hz)` takes a frequency in ticks per second. The current Phase 1
simulation default is 120 Hz, but the clock accepts another rate:

```rust
use fate_sim::ecs::{Position, Velocity, movement_system};
use fate_sim::math::{Fixed, FixedVec2};
use fate_sim::simulation::SimulationState;
use fate_engine::time::Time;

fn main() {
    let mut time = Time::new(120.0);
    let mut simulation = SimulationState::with_seed(1234);
    let entity = simulation.world.spawn();
    simulation.world.set_position(entity, Position(FixedVec2::ZERO))
        .expect("fresh entity must be alive");
    simulation.world.set_velocity(
        entity,
        Velocity(FixedVec2::new(Fixed::ONE, Fixed::ZERO)),
    ).expect("fresh entity must be alive");

    let frame_delta_seconds = time.tick();
    while time.next_fixed_step().is_some() {
        movement_system(&mut simulation.world, Fixed::from_ratio(1, 120));
        simulation.advance_tick().expect("tick counter exhausted");
    }

    let interpolation_alpha = time.alpha();
    let _presentation_values = (frame_delta_seconds, interpolation_alpha);
}
```

`tick()` returns a frame delta clamped to `MAX_FRAME_DELTA_SECONDS` (0.1 s) for
presentation work, but adds the full wall-clock delta to the fixed accumulator.
This prevents a long frame from silently discarding simulation time. The
retained backlog is separate from the clamped presentation delta:
`next_fixed_step()` runs at most `MAX_FIXED_STEPS_PER_FRAME` (eight) ticks per
frame, and any remaining backlog stays queued for subsequent frames. Use
`is_catching_up()` or `pending_fixed_steps()` to observe that backlog. `alpha()`
remains the fractional remainder for interpolation and excludes queued whole
ticks, so the rendered state can temporarily lag wall time while the simulation
catches up.

`Time::frame_period_ms()` returns the exact target period at 120 Hz
(approximately 8.333 ms). `Time::frame_budget_ms()` returns a deliberately
conservative integer 8 ms value for scheduling and budget checks; it is not the
exact tick period and does not guarantee 120 presented frames per second.

The demo keeps its simulation active after focus loss, clears held input and
mouse motion to avoid stuck controls, skips rendering, and throttles redraws to
20 Hz. At the demo's 120 Hz simulation rate, that leaves room for the eight
step-per-frame limit to catch up after a brief stall. The engine does not
define a local gameplay-pause state; networked pause should be represented in
the simulation state while network ticks continue.

`SimulationState` does not choose a tick rate or invoke systems. The example
uses 120 Hz and advances its state once for each consumed fixed step; the
consumer must keep its chosen `Time` frequency and per-tick system delta in
agreement.

Use frame delta for non-deterministic presentation work, such as a free camera.
Advance deterministic state only with fixed ticks and explicit input values.
Do not read wall-clock time, renderer state, or platform devices from
simulation systems.

### Simulation state and its limits

`fate_sim::simulation::SimulationState` owns a `World`, deterministic tick
counter, and seeded random stream; it does not select or run gameplay systems,
define input commands, or create entities on behalf of an application. Construct with
`new()` or `with_seed(seed)`, run the consumer's systems explicitly, then call
`advance_tick()`. Snapshots include the world, tick, and RNG state.

```rust
use fate_sim::ecs::{Position, Velocity, movement_system};
use fate_sim::math::{Fixed, FixedVec2};
use fate_sim::simulation::SimulationState;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = SimulationState::with_seed(1234);
    let entity = state.world.spawn();
    state.world.set_position(entity, Position(FixedVec2::ZERO))?;
    state.world.set_velocity(
        entity,
        Velocity(FixedVec2::new(Fixed::ONE, Fixed::ZERO)),
    )?;
    movement_system(&mut state.world, Fixed::from_ratio(1, 120));
    state.advance_tick()?;
    assert_eq!(state.tick(), 1);
    Ok(())
}
```

Simulation state serialization uses the `StateWriter` trait (`write_state(&self, writer: &mut W)`).
Owned snapshot buffers are produced with `state_bytes()`, while per-tick state hashes are computed
via `state_hash()` using the zero-allocation `StateHasher` (a streaming FNV-1a 64-bit diagnostic sink),
avoiding per-tick heap allocations during recording, playback, or resimulation.

Snapshot validation rejects duplicate free-list IDs and free lists that do not
cover every dead slot exactly once. It remains a structural invariant check,
not an authenticity check: successful decoding or a matching checksum does not
prove that arbitrary external state is trustworthy. The local networking client
also requires a snapshot packet's `server_tick` to match the tick encoded in its
restored state before accepting it.

The fixed ECS component set and 2D systems are still small prototypes; projects
choose which systems to call and how to interpret commands. The demo's
`EngineCore`, scripted cube-grid simulation, replay/rollback checks, and arena
scene live under `projects/demo/src`.

### ECS and fixed-point math

The `fate_sim::ecs::World` uses generational `Entity` handles (32-bit ID + 32-bit
generation) and fixed columns for `Position`, `Velocity`, `Rotation`, and `Spin`.
`spawn()` allocates a fresh handle or recycles a despawned slot from an internal
free list with an incremented generation, preventing the ABA problem and unbounded
vector growth during normal slot reuse. Despawning is safe and idempotent.
Checked setters (`World::set_position`, `set_velocity`, etc.) return
`Err(EcsError::EntityNotAlive)` when given a dead or invalid entity handle.
Columns validate the stored generation on `get`, `get_mut`, and `remove`.
Read attached components through `World::positions()`, `velocities()`,
`rotations()`, and `spins()`, which expose read-only columns. Update attached
values through `position_mut`, `velocity_mut`, `rotation_mut`, and `spin_mut`,
which return `None` for dead or stale handles. The columns themselves are
private, so direct insertion or replacement cannot bypass world lifecycle
checks. This remains a lightweight indexed-column foundation; arbitrary
component registration, query composition, archetype storage, and parallel
scheduling are deferred to later milestones.

`fate_sim::math::Fixed` is signed Q32.32. Arithmetic saturates on overflow;
division truncates. `Fixed` provides deterministic constants (`PI`, `TAU`,
`FRAC_PI_2`, `FRAC_PI_4`) and polynomial trigonometric functions (`sin_cos()`,
`sin()`, `cos()`). `FixedVec2` and `FixedVec3` provide 2D and 3D vector operations:
addition, scaling, dot product, 3D cross product, length, and normalization.
`FixedQuat` represents unit quaternions with axis-angle constructors, Euler angles
(`from_yaw_pitch_roll`), quaternion composition, and vector rotation (`rotate_vec3`).
`FixedTransform` composes fixed-point position, orientation, and uniform scale,
transforming points and vectors. `FixedVec3::to_f32x3()` and `FixedQuat::to_f32x4()`
convert vectors and rotations to renderer-facing float primitives.

Convert to `f32` only when preparing renderer-facing values; converting back
does not preserve deterministic precision. The engine's renderer-facing
`fate_engine::math::{Vec2, Vec3, Mat4}` uses `f32`. `Camera3d` exposes
its orthonormal basis (`forward()`, `right()`, `up()`, `basis()`) and separate
`view_matrix()` and `projection_matrix()` helpers.

### Deterministic randomness

Use `SimulationState`'s seeded RNG methods or
`fate_sim::rng::DeterministicRng` for reproducible simulation randomness.
Available operations include `next_u64`, bounded unsigned/signed/fixed-point
ranges, and seeded shuffling. Ranges are half-open: `[min, max)`. Invalid or
empty ranges assert rather than returning an error.

The RNG state is included in `SimulationState` snapshots. To continue the same
sequence after restore, restore the same simulation snapshot rather than
seeding a new generator.

### Snapshots and replay

`SimulationState::state_bytes()` produces a versioned canonical little-endian
snapshot; `SimulationState::from_state_bytes()` validates and restores it.
`state_hash()` is an FNV-1a 64-bit diagnostic checksum. It is not
cryptographic, authenticated, or suitable as proof that remote data is
trustworthy. Snapshot format changes require a version bump and may make older
snapshots unreadable; persist an application-level version/metadata alongside
saved data if your project needs migration.

`SimulationState` snapshot format version 2 stores only generic simulation
state; it is not compatible with the earlier prototype `EngineCore` snapshot
schema.
The demo-specific [`ReplayTape`](../projects/demo/src/demo_replay.rs) records
the demo command schema and bounded input history. Its replay serialization is
versioned but has no authentication or encryption; treat replay files as
untrusted input.

The showcase's [headless replay commands](../projects/demo/#headless-replay-inspection)
record a scripted sample and verify every tick in an existing showcase replay.
They do not initialize graphics or audio and do not open sockets.

The playable slice keeps its own in-memory replay window in
[`projects/combat_slice/src/replay.rs`](../projects/combat_slice/src/replay.rs).
It records one input per fixed tick, replays the window from a snapshot, and
compares the final hash with the live state every 240 ticks. The slice's
snapshot codec and verification loop live in
[`projects/combat_slice/src/sim.rs`](../projects/combat_slice/src/sim.rs) and
[`projects/combat_slice/src/verification.rs`](../projects/combat_slice/src/verification.rs).
Both are project-owned rather than reusable engine APIs.

### Rollback

The demo-specific `RollbackSession` in
[`demo_replay.rs`](../projects/demo/src/demo_replay.rs) creates a bounded local
input history; a zero capacity returns an error. It advances the demo
simulation and reconciles authoritative snapshots, optionally applying
corrected inputs for retained ticks. The reusable `fate_sim` crate does not
currently prescribe a command format or rollback policy.

This is a deterministic state/input primitive, not a networking solution.
Transport, packet ordering, authentication, server policy, interpolation,
ownership, and production threat handling remain the consuming application's
responsibility.

### Local networking protocol prototype

`fate_net` is a transport-agnostic local protocol prototype built on
`fate_sim` snapshots. It encodes versioned little-endian packets for client
inputs and server snapshots; it does not open sockets or authenticate peers.

- `Limits` bounds input payloads, snapshot payloads, and acknowledgement
  history. Encoding and decoding reject payloads over the configured limits.
- `ClientSession::make_input` numbers inputs and produces encoded input packets.
- `ServerSession::accept_input` validates packet structure and ordered ticks,
  applies the input transactionally, advances one simulation tick, and keeps a
  bounded acknowledgement history.
- `ServerSession::snapshot_bytes` produces a snapshot packet with the server
  tick, highest acknowledged input tick, and a diagnostic state hash.
- `ClientSession::receive_snapshot` validates the snapshot schema and hash,
  requires the packet's `server_tick` to match the tick encoded in the state,
  rejects stale or future acknowledgements and server ticks, and returns the
  accepted snapshot.

Inputs must arrive in order; the transport is responsible for retransmission.
Client input is untrusted until the application's applier validates it, and
server snapshots are untrusted until `SimulationState::from_state_bytes`
accepts them. Snapshot acceptance has an explicit tick contract: `server_tick`
must equal the tick encoded in `SimulationState`, and accepted server ticks and
input acknowledgements must not move backwards. A rejected packet leaves the
client session unchanged, so a later valid snapshot can recover. State hashes
are diagnostic only and must not be treated as authentication. The crate does
not implement transport, encryption, peer identity, clock drift compensation,
interpolation, ownership, or production server policy.
`crates/fate_net/tests/loopback.rs` runs both roles end to end.

## 8. Rendering

### Windowless capture

`OffscreenRenderer` uses the same triangle, sprite, tilemap, mesh, and overlay pipelines
without creating a window or surface. It still needs a `wgpu` adapter. The
target size is fixed at construction, and assets must be uploaded through
this renderer: resources from a windowed renderer belong to another device.

```rust,ignore
let mut renderer = OffscreenRenderer::new((641, 360)).await?;
renderer.capture(OffscreenScene::Triangles(&instances), &overlay)?;
// Poll during later updates, or wait with a timeout in a native command-line tool.
if let Some(frame) = renderer.poll_capture()? {
    // frame.rgba8 is tightly packed sRGB RGBA8, top row first.
}
```

One capture can be pending at a time. A new request returns `OffscreenError::Busy`
until the result is retrieved; a timeout does not discard it. The renderer
reuses its readback buffer and strips GPU row padding. It does not encode files.
To try an export without a window:

```powershell
cargo run --release --locked -p fate_engine --example offscreen_capture -- capture.ppm
```

Pass two output paths to capture both landscape and portrait using one
renderer and the same uploaded meshes. `OffscreenRenderer::resize` changes
subsequent frames without changing the current device, cameras, or settings.
A pending capture retains its original size and pixels; retrieve it before
requesting another capture. Resizing does not allocate a readback buffer.
On capture, an existing buffer is reused when large enough, and only the
current image range is mapped.

```sh
cargo run --release --locked -p fate_engine --example offscreen_capture -- wide.ppm tall.ppm
```

See the [standalone consumer recipe](../projects/consumer_smoke/#verify-outside-the-workspace)
to run this renderer example with its own Cargo manifest.

Use `OffscreenRenderer::render` to submit without pixel readback. It returns
scene instance and draw counts immediately and does not allocate the capture
buffer until a capture is requested. An already queued capture retains its
pixels. Opt into timestamps with `new_with_options`; `poll_gpu_timings`
returns completed scene and overlay samples on supported adapters.
Native tools can call `wait_idle(timeout)` outside their CPU submission
measurement. See [Polygon's render benchmark](../projects/fate_polygon/)
for a windowless consumer.

For a dense 2D workload, run the tile batch benchmark on a GPU-equipped host:

```powershell
cargo run --release --locked -p fate_engine --example tile_batch_bench
```

It submits one and eight full tile layers and reports CPU render/submit and
GPU scene p50/p95 separately, without a window, presentation, or pixel readback.
Sprites, tile layers, and HUD images use one 64-byte instance per quad;
only adjacent compatible quads merge, so transparent draw order is unchanged.

### Available windowed paths

`fate_engine::renderer::Renderer` provides these draw entry points:

- `render(&[Instance2d])` draws the built-in 2D triangle in one instanced
  draw, taking up to 256 transforms in input order. An empty list clears and
  presents without a draw call. `Instance2d` supplies world `position`,
  `rotation`, and `scale`; it does not provide a general 2D mesh/material API.
- `render_with_overlay(&[Instance2d], &Overlay)` adds the same physical-pixel
  rectangle, image, and text layers used by 3D, with one surface acquisition
  and queue submission. Pass an empty instance list for overlay-only screens.
  When GPU timing is enabled, the 2D scene and overlay durations are separate.
- `render_sprites(&[Sprite2d], &Overlay)` draws a textured 2D scene instead
  of the built-in triangles. Sprites use world-space centers, sizes, and
  counterclockwise rotation (+Y up). They follow `Camera2d`; the overlay
  stays in physical pixels. Sprites share `Texture2d` and `TextureRegion` with
  HUD images. Adjacent sprites using the same texture share a draw without
  reordering, and invalid records do not count toward the 256-instance limit.
- For 2D picking, pass the physical cursor position and drawable size to
  `Camera2d::pixel_to_world`. `world_to_pixel` does the reverse. Use
  `Sprite2d::world_bounds().is_some_and(|b| b.contains_point(world))` to
  test the rotated quad, and walk the submitted sprites back-to-front if
  the last drawn hit should win. This tests geometry, not texture alpha.
  For conservative culling, intersect `bounds.axis_aligned()` with the
  camera's `visible_bounds(renderer.aspect())` before submitting. The
  renderer does not apply that culling automatically.
- `render_tile_layers(&[TileLayer2d], &Overlay)` draws ordered atlas-backed
  tile layers, up to `MAX_TILE_LAYERS` (8). Each layer uses one atlas; the map
  is row-major with a world-space top-left origin: columns go right and rows go
  down. Each cell is `None` (empty) or a `u16` index into that layer's
  `TextureRegion` palette. Edit cells with `set` and pick coordinates with
  `TileLayer2d::cell_at_world(world, camera_center)`, which applies the layer's
  offset and parallax. Later layers composite above earlier ones. The layer's
  `parallax` scales camera travel (`1` is world-fixed, `0` is screen-fixed),
  and `offset` adds a world-space shift on top; builders set both. The renderer
  visits only rows and columns intersecting each layer's translated view,
  skips invalid palette entries, and submits up to `MAX_DRAWN_TILES` (4,096)
  visible tiles per layer. This is a tile renderer, not a map file loader or
  level editor. `render_tiles` is the one-layer convenience. Use
  `render_tile_layers_with_sprites` to draw sprites above every layer in the
  same scene pass; the tile and sprite budgets are independent.
  `render_tiles_with_sprites` is the one-layer convenience. For windowless
  capture use `OffscreenScene::Tiles { layers, sprites }`.
- `render_meshes(&[Draw3d])` draws indexed `Mesh3d` geometry with perspective
  projection and a depth buffer. `MeshVertex` contains position and vertex
  color; each `Draw3d` adds a model matrix and RGB tint.
- `render_meshes_with_text(&[Draw3d], &[TextDraw])` draws the same 3D list and
  then a text overlay in one surface submission. `render_meshes` is equivalent
  to calling it with an empty overlay.
- `render_meshes_with_overlay(&[Draw3d], &Overlay)` also draws screen-space
  rectangles, textured images, and text between the 3D scene and presentation.
  Use this when a panel or bar needs to sit behind a label, or an icon needs to
  sit beside it.
- `render_scene(&[SceneDraw3d], &Overlay)` draws a mixed list of opaque textured
  and untextured meshes with the same depth buffer and draw budget, then draws
  the overlay in the same submission.

For a stable 3D candidate list, build `VisibilitySet3d::new(bounds)` from
world-space `Bounds3d` records in draw order. Call
`visibility.query(camera.frustum(aspect), &mut visible)` to replace a reusable
`Vec<usize>` with the visible input indices in that same order. Query stats
separate group checks from individual bounds tests. Grouping works best when
nearby objects are adjacent in the input; rebuild when bounds or order change.
The renderer leaves these choices to the consumer. To compare with a direct
scan without opening a window or GPU:

```sh
cargo run --release --locked -p fate_engine --example visibility_bench
```

Set `renderer.mesh_light` on either `Renderer` or `OffscreenRenderer` to
opt into flat 3D lighting. `FlatLight3d::new(direction, ambient)` takes a
world-space direction from the surface toward the light and an ambient floor
from `0.0` to `1.0`. It normalizes the direction and returns `None` on invalid
input. Set `mesh_light` back to `None` for the default unlit output. By default,
the light shades the visible side of each triangle without changing draw batches.

For smooth shading, upload `NormalMeshVertex` data with
`create_mesh_with_normals`. This works with textured and untextured draws.
Supply object-space normals; upload normalizes them and rejects zero or
non-finite values. Duplicate vertices where a hard edge needs different normals
on each side. The renderer applies the inverse transpose of the model's linear
transform for nonuniform and mirrored scales. If that transform is singular,
it falls back to a geometric face normal. Lighting affects opaque meshes only
and does not cast shadows.

Set `renderer.mesh_fog` to `DepthFog3d::new(linear_rgb, start, end)` to fade
opaque 3D meshes into a matching background. The distances are measured
forward along the camera axis in world units, not radially or in depth-buffer
coordinates. Pixels before `start` are unchanged; from `end` onward they match
the fog color. The blend happens after lighting, in linear RGB. Fog does not
change clipping, depth tests, batching, 2D scenes, or the HUD. `None` restores
the default clear color and turns fog off.

For CPU-side 3D picking, build a `PickMesh3d` from the object-space positions
and indices used for mesh upload. Ordinary mesh uploads do not retain this
data. Use `Camera3d::pick_ray(pixel, (width, height))` for a physical pointer
position, or construct a `PickRay3d` with an explicit distance interval.

```rust,ignore
if let Some(ray) = renderer.camera_3d.pick_ray(pointer, renderer.surface_size()) {
    if let Some(hit) = pick_mesh.intersect(ray, draw.model) {
        // Use hit.triangle, hit.distance, hit.position, or hit.barycentric.
    }
}
```

The query tests both sides of each triangle and skips singular or non-affine
models. For multiple objects, query the submitted draw candidates and compare
world distances. The consumer owns selection policy and any filtering by
material, visibility, or HUD occlusion. This is a geometric query, not a
sample of the frame's depth buffer.

`PickMesh3d::new` builds a local bounding-volume hierarchy once per mesh;
instances reuse it even when their model transforms change. Queries test
nearer bounds first and skip groups that cannot beat the current hit,
without allocating. `intersect_with_stats` returns the same hit plus bounds
and exact triangle test counts. These are CPU work counts, not timings.
Meshes whose triangle bounds overlap heavily can still require many tests.
Rebuild the pick mesh when its positions or indices change.

For many instances, build a `PickScene3d` from `PickInstance3d` records in
consumer draw order. Each record pairs an `Arc<PickMesh3d>` with a model
matrix, so instances can share mesh geometry and its triangle tree. The
scene index skips groups of instances before querying their meshes. The
nearest hit reports the original input index and a `MeshHit3d`; exact
distance ties favor the earlier input. When a few instances move, call
`scene.refit_transform(index, model)` for each changed input index. This updates
only that instance's leaf and ancestors. For widespread motion, call
`scene.refit_transforms(&models)` with one matrix per original input in draw
order. Neither path allocates. An invalid index or slice length returns `false`
without changing the scene. Invalid transforms make their slots unpickable
until a valid transform is supplied.

Rebuild the index if instance order, membership, or mesh references change,
or if large movements make the existing tree less selective. The renderer
does not maintain the index automatically. `fate_window_smoke --scene-picking`
uses the same moving transforms for both rendering and picking.

Measure picking separately from render timings with the headless example:

```powershell
cargo run --release --locked -p fate_engine --example picking_bench
```

It reports scene build and per-ray p50/p95 for grid hits, misses, edge
grazes, and heavily overlapping instances. It compares full refit and rebuild
for coherent and shuffled motion, then measures sparse versus full refits at
several update densities. The sparse threshold depends on the scene and motion;
start with sparse refits for a few changes and measure before choosing a cutoff.

Adjacent overlay images using the same `Texture2d` and effective scissor
share a draw, even when their atlas regions, transforms, or tints differ.
Images are never reordered. Invalid quads and clips that resolve to an empty
scissor are skipped before applying the image limit.

Use `Texture2d::update_rgba8` to replace a texture's pixels, or update just
one atlas rectangle without changing its sampler, dimensions, or draw
references. Both take tightly packed RGBA8 sRGB pixels (straight alpha).

```rust,ignore
// Replace a 2×2 tile at texel (16, 4), leaving the rest of the atlas alone.
atlas.update_region_rgba8([16, 4], [2, 2], &[255_u8; 16])?;
renderer.render_sprites(&sprites, &overlay);
```

Updates are validated before queueing and become visible to subsequent
submissions. The origin is in texels from the top left, not normalized UVs.
No row alignment padding is needed; invalid updates leave the texture unchanged.

`UiRect` is one axis-aligned screen-space rectangle in physical pixels, using
the same top-left origin as `TextDraw`. `position` is the top-left corner,
`size` extends right and down, and `color` includes alpha. `UiImage` places a
`Texture2d`, multiplied by a tint color, at the same kind of rectangle. By
default it samples the whole texture; `.with_region(texture.region(x, y, width,
height))` selects an atlas tile using normalized coordinates, and
`.with_rotation(radians)` rotates the quad clockwise around its center without
changing the sampled region.
`Overlay` groups borrowed slices:

```rust
let overlay = Overlay {
    rects: &panels,
    images: &icons,
    text: &labels,
};
renderer.render_meshes_with_overlay(&draws, &overlay);
```

Layers draw in field order: rectangles, images, then text. Every layer is alpha
blended, skips depth testing, and preserves slice order, so a translucent panel
behind a label simply comes first. Up to `MAX_UI_RECTS` (currently 256)
rectangles and `MAX_UI_IMAGES` (currently 64) images are submitted per frame.
Records with non-finite values or non-positive sizes are skipped. The coordinate
conversion and skip rules are covered by headless tests, and offscreen GPU tests
skip when no adapter is available.

Every record can opt into a `UiClip` with `with_clip(UiClip::new(position,
size))`. Only pixels inside the clip are drawn, so overflowing text or artwork
stays inside a panel even when the record's geometry does not. Clips are
independent per record and are intersected with the window; empty or invalid
clips draw nothing. Scissor state is grouped by consecutive equal clip, so the
cost is proportional to clip changes rather than record count.

Image rotation is purely geometric: it changes the quad's corners, not its
region, tint, or clip. A rotated image can extend past its unrotated bounds, so
pair it with a `UiClip` when it must stay inside a slot.

`Renderer::create_texture(label, width, height, rgba8)` uploads an RGBA8 sRGB
texture and returns a `Texture2d` handle. The byte slice must contain exactly
`width * height * 4` bytes; empty, zero-sized, oversized, and mismatched uploads
return `TextureCreateError` before touching the GPU. Sampling defaults to linear
filtering with clamp-to-edge addressing. `TextureRegion` describes a normalized
sub-rectangle, and `Texture2d::region(x, y, width, height)` converts an atlas
tile from pixels. Regions are not clamped or inset.

Pass `TextureOptions` to `create_texture_with_options` to choose sampling
behavior. `TextureFilter::Nearest` keeps pixel art crisp and avoids atlas bleed;
`TextureOptions::NEAREST` and `TextureOptions::NEAREST_REPEAT` are the common
presets. `TextureAddress::Repeat` (or `MirrorRepeat`) combined with a region
wider than `1.0` tiles the texture: a region of `0..4` on `u` repeats it four
times across the image. The same sampler settings apply to mesh UVs.

There are no mipmaps, packers, decoders, or hot reload: leave padding between
linearly filtered atlas tiles, and let consumers own asset loading and texture
lifetime. A consumer-owned texture can be used with the renderer that created
it and borrowed by both mesh draws and `UiImage` records in the same frame.

`TextDraw` places one run of text at a top-left position in physical pixels,
measured from the window's top-left corner. The renderer ships a built-in 5x7
bitmap font covering printable ASCII (`0x20..=0x7E`); unknown characters render
as `?`. There is no font file, shaping, or anti-aliasing. Each draw carries a
pixel `scale` and RGBA `color`, and alpha blending composites glyphs over the
scene. `TextDraw::new(position, text)`, `.with_scale(scale)`, and
`.with_color(color)` build records; `text_width(text, scale)` returns the width
of the widest line so consumers can center or right-align text. Newlines and
tabs are supported, and blank glyphs (such as spaces) emit no quads.

`wrap_text(text, max_width, scale)` splits a string into lines that fit a pixel
column, so HUD copy can adapt to a panel instead of using hand-split strings. It
wraps at ASCII whitespace, hard-breaks a word that is wider than the column,
keeps explicit paragraphs and blank lines, and collapses whitespace inside a
paragraph to single spaces. The returned `Vec<String>` usually feeds one
`TextDraw` per line, stepped by the font's line height. An invalid or
non-positive `max_width` or `scale` returns the original paragraphs unwrapped,
so callers can fall back to a single draw.

Text positions and sizes are physical pixels, matching
`Renderer::surface_size()`, which returns the current drawable size. Consumers
own HUD layout, content, and anchoring; multiply logical layout values by the
window's scale factor (from `winit`) so text keeps the same apparent size on
high-DPI displays. The engine does not overlay diagnostics or force a HUD on
any project. One frame is capped at 2048 glyphs, and text beyond the budget is
dropped with a warning. The layout helpers are pure and tested headlessly, and
the GPU path has an offscreen visibility test that skips when no adapter is
available.

`Renderer::create_mesh(label, vertices, indices)` uploads a mesh through the
renderer's device and returns a `Result`; `Renderer::cube_mesh()` uploads the
built-in unit cube. Both avoid naming `wgpu::Device`. The lower-level
`Mesh3d::new(device, label, vertices, indices)` remains available for headless
tools that create their own device. Mesh creation checks for empty data,
triangle index count, out-of-range indices, and unsupported index count. Meshes
are caller-owned and can be reused by multiple draw records.

For a textured mesh, upload `UvMeshVertex` values with
`create_mesh_with_uvs(label, vertices, indices)`. Each vertex adds a `[u, v]`
coordinate: `[0.0, 0.0]` is the upper-left corner of the texture, and
`[1.0, 1.0]` is the lower-right. Non-finite UVs are rejected. Meshes uploaded
without UVs use `[0.0, 0.0]` at every vertex.

```rust
let textured = Draw3d {
    mesh: &uv_mesh,
    model,
    tint: [1.0; 3],
}
.with_texture(&texture);
renderer.render_scene(&[color_draw.into(), textured], &overlay);
```

A textured mesh multiplies the sampled RGB by the vertex color and the draw
tint; use white for both to show the texture unchanged. The 3D pass is opaque:
texture alpha is ignored on meshes but still blends in overlays. The renderer
does not provide lighting, transparent 3D materials, or a scene graph.

The camera types are `Camera2d` and `Camera3d`. The 2D camera uses a world-space
center, view size, and zoom. The 3D camera uses +Y as up and faces -Z when yaw
and pitch are zero. Projection parameters are sanitized, but consumers still
own choosing sensible world transforms and camera movement.

### Surface, resize, and draw limits

`Renderer::new(Arc<Window>)` is async and returns `Result<Renderer,
GpuInitError>`. The demo uses `pollster::block_on` during window setup.
Call `resize(PhysicalSize<u32>)` when the window changes size. A render call
returns `false` when the surface cannot be drawn for that frame; callers
should treat this as a skipped render, not as successful presentation.
Pass zero dimensions through too: they suspend frame acquisition until the
next nonzero resize, even if the restored size matches the old one.
`surface_size()` retains the last configured dimensions (or nonzero
initial defaults if the window has not been drawable yet). A minimized or
occluded window can also skip frames without a zero-size event.
The renderer retries recoverable surface changes on subsequent draws.
Recovery from loss of the GPU device itself is not supported.

On a desktop with a usable GPU adapter, run
`cargo run --release -p fate_window_smoke -- --surface-lifecycle` to check
resize, zero-size suspension, and window minimize/restore for both 2D and 3D.
This is separate from the headless quality gate.

The renderer caps work at `MAX_DRAWN_INSTANCES` (currently 256) and warns when
draw pressure reaches the warning threshold. This is a hard draw count limit,
not automatic culling: the consumer must filter invisible entities before
constructing the submitted list. For 2D, `Camera2d::visible_bounds(aspect)`
returns a `renderer::Bounds2d` matching the camera projection. For 3D,
`Camera3d::frustum(aspect)` returns a `renderer::Frustum3d` extracted from the
view-projection matrix (wgpu `0..=1` depth); test candidates with
`intersects_bounds(Bounds3d)` or `intersects_sphere`. `Bounds2d` and `Bounds3d`
also provide `contains_point`, `intersects`, `expanded`, `min`, and `max`. The
demo uses the 3D helpers to cull arena and cube instances before submission,
but still owns scene traversal and object selection. Culling is not automatic,
and the demo's diagnostics presentation remains project code.

For a repeatable CPU-side baseline, run
`cargo run --release -p fate_engine --example draw_list_bench`. It reports the
candidate count, visible draw count, and per-candidate/per-visible preparation
time without initializing a GPU; driver-side submission cost is not included.

### Backend coupling

The renderer uses `wgpu` and `winit` directly. The current public coupling is:

- `Renderer::new(Arc<winit::window::Window>)` and
  `Renderer::resize(PhysicalSize<u32>)` require `winit` types at the app
  boundary, because the consumer owns the event loop and window.
- `Renderer::gpu` and the `Gpu` fields expose `wgpu::Instance`, `Surface`,
  `Adapter`, `Device`, `Queue`, and `SurfaceConfiguration` for lower-level
  work. `GpuInitError` also wraps `wgpu` error types.
- The common consumer path does not need to name `wgpu`: `Renderer::aspect`,
  `Renderer::surface_size`, and the public mesh, texture, and draw methods
  cover resource uploads and frame submission with 3D meshes and overlays.

`projects/window_smoke` is a second windowed consumer that validates this seam.
Its manifest depends only on `fate_engine`, `pollster`, and `winit`; it creates
its own window, uploads colored and UV meshes, and reuses one procedural atlas
across the scene and HUD through `render_scene`. There is no second backend and
no backend-portability trait.
Treat the `wgpu`/`winit` boundary as a prototype seam, pin dependency versions,
and do not assume future backend compatibility.

## 9. Audio

`fate_engine::audio` provides procedural one-shot sounds. The engine synthesizes
tones; the project decides which sound belongs to which event.

```rust
use fate_engine::audio::{AudioEngine, Tone};

let audio = AudioEngine::new();
if let Ok(audio) = audio {
    audio.play(Tone::square(180.0, 0.05, 0.16));
    audio.set_volume(0.5);
}
```

`Tone` combines frequency, duration, amplitude, and a `Waveform` (`Sine`,
`Square`, `Triangle`, or `Noise`). Tones start immediately and are not looped or
positioned in space. The mixer applies a short attack/release envelope, sums up
to `MAX_VOICES` tones, and clamps the result to full scale. When the voice pool
is full, new tones are dropped rather than replacing an active sound.

`AudioEngine::set_volume` controls a master gain in `0..=1`; non-finite values
mute and out-of-range values clamp. The change applies to the next rendered
buffer, including tones already playing, so a consumer can mute immediately
without stopping the stream.

`AudioEngine::new()` is the only call that touches a device. It returns an
`AudioError` when the platform has no default output, the default configuration
cannot be read, the stream cannot be built, or the sample format is unsupported.
Treat audio as optional: log the error and keep running without it. If the
output device disappears later, the stream error is logged and the engine does
not rebuild the stream automatically. CPU-only checks never need to open a
device because the mixer is tested separately.

The engine does not provide an asset format, spatial audio, looping music, or
mixer buses. Consumers choose those policies when a real workload needs them.

Audio is presentation. Do not feed tone events back into deterministic
simulation state, replay data, or snapshots.

## 10. Errors, diagnostics, and operational notes

- Handle `GpuInitError` and `MeshCreateError` explicitly during startup/content
  creation. A missing adapter or malformed mesh is a real initialization
  failure; do not silently treat it as a successful render setup.
- A `false` renderer result means the current frame was skipped because of
  surface state; request a later redraw according to the host application's
  policy.
- Configure logging in the consuming executable. The demo initializes
  `env_logger`; the library uses the `log` facade.
- Input edge state lasts until `end_frame()`. Clear it too early and consumers
  miss events; fail to clear it and a one-frame event can appear held as an
  edge.
- Keep snapshot/replay validation errors visible. Their checksums detect
  deterministic divergence, but do not validate the identity or trustworthiness
  of a sender.
- `fate_net` protocol and session errors are explicit. Treat malformed,
  out-of-order, or stale packets as rejected input, not as partial success;
  the server commits an input only when its applier returns `Ok`.

### Frame diagnostics and metric sinks

For a native companion, run `cargo run --release -p fate_demo -- --dbg`.
The separate `fate_debug` crate owns UI, history and export. Its
`DebugCompanion` adapter can attach to any `Renderer`, route its own winit
events, and refresh while unfocused. Applications may add inspectors and
commands; no application rules are required for basic renderer diagnostics.
See the [integration guide](../crates/fate_debug/README.md).

`Renderer::subscribe_diagnostics()` exposes a presentation-neutral bounded
channel of 2D/3D render counts, call/recording durations, skipped frames and
GPU pass samples (when enabled). A slow reader drops telemetry, never rendering;
`take_dropped()` reports overflow. GPU sample IDs are independent of CPU frame
IDs. Existing `poll_gpu_timings()` remains usable with a subscription.

`fate_engine::diagnostics` provides reusable, consumer-owned diagnostics:

- `FrameSample` describes one frame's measured CPU phases: fixed-step
  simulation, draw-list preparation, and render submission. It also carries the
  drawn instance count. `FrameDiagnostics` keeps a rolling window and reports a
  `FrameReport` with averages, worst case, budget headroom, draw pressure, and a
  `BudgetPolicy` classification. GPU execution time is not measured.
- `FrameReport::summary()` returns a stable single-line text form for logs. The
  structured fields are available for consumers that build their own HUD or
  metrics output.
- `DiagnosticSink` receives `(group, name, DiagnosticValue)` tuples;
  `DiagnosticSource` lets a system contribute values on demand. `NullSink` and
  `LogSink` are provided, and `Diagnostics` samples frame data and sources
  together. Exported data is presentation-neutral; the engine does not draw an
  overlay or choose an external metrics backend.

Read-only inspection snapshots are available from the simulation and protocol
crates:

- `World::diagnostics()` returns `WorldDiagnostics`: live entities, allocated
  capacity, free slots, and populated component counts. `Column::count()`
  reports populated slots in one column.
- `SimulationState::diagnostics()` returns `SimulationDiagnostics`: tick, world
  counts, and the diagnostic state hash.
- `ServerSession::diagnostics()` and `ClientSession::diagnostics()` return
  session progress (ticks, acknowledgement history, snapshot state). Their
  simulation/hash fields remain diagnostic checksums, not authentication.

These snapshots are cheap read-only data. Consumers decide how to display,
log, or export them, and tooling must not become a dependency of the engine.

## 11. Verification and examples

Run the workspace quality gate after changing engine-facing behavior:

```sh
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

### Automated quality gate

[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) runs the same
CPU-only checks on pull requests, branch pushes, and manual dispatches. CI
uses current stable Rust. The Windows reference measurements in the roadmap
used Rust 1.99.1 (`x86_64-pc-windows-gnu`).

The Ubuntu job runs, in order:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p fate_consumer_smoke --locked
```

Before the first build step it installs `libudev-dev` and `libasound2-dev`,
because the combat slice's Linux controller support links against the system
`libudev` through `pkg-config`, and the engine's audio output resolves ALSA the
same way. The windowing and rendering dependencies load their system libraries
at runtime, so they need no development packages.

The consumer smoke run is a headless integration check: it exercises input,
simulation, movement, snapshots, and restoration through the public APIs. It
does not initialize a window or GPU.

The Windows job runs the Polygon tests and the headless consumer smoke.
The `portable_cpu_baseline` test runs on both platforms and checks fixed work
counts and final state checksums for physics, navigation, and ECS. It does not
assert timings or initialize a window, GPU adapter, or audio device.
To run just that baseline locally:

```sh
cargo test -p fate_polygon --locked headless::tests::portable_cpu_baseline -- --exact
```

Renderer tests request a test adapter when one is available and skip the GPU
cases when none exists, so the headless test job still passes on machines
without graphics hardware. A skipped renderer test is not evidence that
rendering works.

Windowed and GPU checks are intentionally separate from the required CI jobs.
Run the relevant consumers manually on a desktop with a graphics adapter.
For a platform baseline:

```sh
cargo run --release --locked -p fate_window_smoke -- --surface-lifecycle --dbg
cargo run --release --locked -p fate_polygon -- --dbg --smoke
cargo run --release --locked -p fate_polygon -- --dbg
```

Check that the lifecycle smoke restores both 2D and 3D submission and that
Polygon's smoke exercises all six presets. In the interactive run, resize and
minimize/restore the viewport, then put focus on another window while leaving
the companion visible. Verify that its counters advance, that closing it keeps
the viewport alive, and that **F3** reopens it. Record the Rust toolchain,
OS and display backend (e.g. Wayland or X11), GPU backend and adapter,
and checks that passed, failed, or were not exercised.

### Performance measurement

Keep these measurements distinct; they answer different questions:

- `fate_sim_bench` measures fixed-step simulation work only.
- [Polygon](../projects/fate_polygon/) measures fixed CPU workloads with
  `--headless` and GPU-backed render workloads without presentation or readback
  with `--offscreen`. Use `--compare` for same-machine CSV baseline checks.
- `draw_list_bench` measures CPU frustum culling and draw-list assembly only.
  It does not initialize a GPU, so render submission and GPU execution are
  excluded.
- `FrameSample`/`FrameReport` separate simulation, CPU draw-list preparation,
  and CPU render submission. They do not include GPU timings.
- To sample GPU passes separately, construct the renderer with
  `Renderer::new_with_options` and `RendererOptions { gpu_timing: true }`.
  Check `gpu_timing_available()` and drain completed samples with
  `poll_gpu_timings()`. Results can arrive on later frames and exclude surface
  acquisition, presentation, and time between passes. Rendering still works
  when timestamps are unavailable or the consumer does not drain the samples.

Run performance commands in release mode against a fixed workload:

```powershell
cargo run --release --locked -p fate_sim_bench -- --ticks 10000
cargo run --release --locked -p fate_engine --example draw_list_bench
cargo run --release --locked -p fate_window_smoke -- --mesh-load --gpu-timing
```

Record the toolchain, platform, workload, and build profile with any result.
Do not compare numbers across machines or claim a full-frame/GPU improvement
from a simulation-only or draw-list-only baseline. Establish a repeatable
draw-list/submission baseline before choosing batching, instancing, or other
submission changes.

Useful source references:

- [`src/lib.rs`](../src/lib.rs) — engine library module surface.
- [`projects/consumer_smoke/`](../projects/consumer_smoke/) — minimal
  independent consumer package and invocation.
- [`projects/combat_slice/`](../projects/combat_slice/) — small playable
  first-person consumer with project-owned rules, replay, and snapshots.
- [`projects/window_smoke/`](../projects/window_smoke/) — minimal independent
  windowed consumer that exercises the renderer seam without a direct `wgpu`
  dependency.
- [`src/input.rs`](../src/input.rs) and
  [`projects/demo/src/demo_input.rs`](../projects/demo/src/demo_input.rs) —
  generic input state and a platform-specific mapping.
- [`src/time.rs`](../src/time.rs) — fixed-step clock, exact frame period, and
  conservative scheduling budget.
- [`src/diagnostics.rs`](../src/diagnostics.rs) — frame phase reports, budget
  classification, and metric sink/source seams.
- [`examples/draw_list_bench.rs`](../examples/draw_list_bench.rs) — repeatable
  CPU draw-list baseline.
- [`crates/fate_sim/src/simulation.rs`](../crates/fate_sim/src/simulation.rs)
  and [`crates/fate_sim/src/ecs/`](../crates/fate_sim/src/ecs/) — simulation
  and current ECS.
- [`projects/demo/src/demo_replay.rs`](../projects/demo/src/demo_replay.rs) and
  [`crates/fate_sim/src/rng.rs`](../crates/fate_sim/src/rng.rs) — demo replay/
  rollback and deterministic random generation.
- [`src/renderer/`](../src/renderer/) and
  [`projects/demo/src/app.rs`](../projects/demo/src/app.rs) — render APIs and
  window/event-loop integration.

## 12. Keeping this guide useful

When a public capability changes, update the relevant guide section in the
same change. Include the goal, prerequisites, setup, public API, a complete
consumer flow, failure behavior, current limitations, and a link to a working
example or test. Clearly label planned functionality; do not document a design
goal as if it were usable today.

Keep this guide focused on using the engine. Put architectural invariants in
[`ENGINE_RULES.md`](../ENGINE_RULES.md), project-specific controls in
[`projects/demo/README.md`](../projects/demo/README.md), and long-term
direction in the design documents.

# Working with fate_engine

> **Status: early implementation.** This guide describes the checked-in
> workspace and APIs, not a stable release contract. Public interfaces,
> dependency boundaries, and behavior may change while the engine is being
> developed.

This is the practical guide for someone building a project on the current
engine. It aims to be complete about what is available and what remains a
prototype. It should be read alongside the engineering constraints in
[`ENGINE_RULES.md`](../ENGINE_RULES.md).
For current gaps and prioritized future work, see the
[engine roadmap](roadmap.md).

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
| Rendering | `wgpu` renderer with a built-in 2D triangle path and indexed 3D mesh path | Windowing and GPU types are exposed; renderer is not backend-independent, and there is no general scene/material/asset system |
| Demo | Standalone app under `projects/demo` | Validation harness only; its scenes and controls are not engine guarantees |
| Networking | `fate_net` provides a versioned local protocol prototype with bounded payloads, ordered inputs, acknowledgement history, and snapshot validation | No transport, authentication, encryption, clock synchronization, or production server policy |

Use the primitives as an experimental starting point. Do not assume that
planned features in a design document are already implemented.

## 2. Workspace structure

```text
fate_engine/
  Cargo.toml                 # workspace root and fate_engine library package
  src/                       # reusable input, math, renderer, and time APIs
  crates/
    fate_net/                # versioned local protocol prototype
    fate_sim/                # headless deterministic simulation crate
    fate_sim_bench/          # CPU-only simulation benchmark
  projects/
    consumer_smoke/          # headless example consumer
    demo/                    # separate runnable consumer application
  docs/
    engine-guide.md          # this guide
```

Responsibilities are intentionally split:

- **`fate_engine`** — generic input state, renderer-facing floating-point math,
  window/GPU renderer foundations, and a frame/fixed-step clock.
- **`fate_sim`** — deterministic simulation primitives. It has no third-party
  dependencies and forbids unsafe code.
- **`fate_net`** — transport-agnostic packet encoding and client/server session
  state machines built on `fate_sim` snapshots. It does not open sockets.
- **`fate_demo`** — window/event loop, platform bindings, demo scene setup,
  demo simulation population, controls, and presentation/telemetry.
- **`fate_sim_bench`** — headless simulation measurements. Its timings do not
  measure rendering, GPU work, or full-frame performance.

Keep game-specific rules, content, action names, and scene setup in a project.
Keep reusable mechanisms in the engine crates only when they have a clear
consumer beyond the demo or form a general capability.

## 3. Requirements and first run

Use a Rust toolchain that supports the workspace's Rust 2024 edition. Running
the windowed demo also requires a working window-system backend and a GPU
adapter supported by `wgpu`.

From the repository root:

```powershell
cargo run -p fate_demo
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

The demo's key bindings, modes, and known scene limitations are in
[`projects/demo/README.md`](../projects/demo/README.md). To run only the
dependency-free simulation tests:

```powershell
cargo test -p fate_sim
```

Run the standalone CPU-only benchmark separately:

```powershell
cargo run -p fate_sim_bench -- --ticks 10000
```

The benchmark warms up and measures selected simulation workloads. It is not
a promise of target frame rate on a particular device.

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
`EngineCore`, movement/boost rules, playfield, and swarm/stress-scene
population live under `projects/demo/src`.

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
division truncates. `FixedVec2` and `FixedVec3` provide the current 2D and 3D
vector operations: addition, scaling, dot product, 3D cross product, length,
and normalization. `FixedVec3::to_f32x3()` converts a vector to a
renderer-facing float triple.
Convert to `f32` only when preparing renderer-facing values; converting back
does not preserve deterministic precision. The engine's renderer-facing
`fate_engine::math::{Vec2, Vec3, Mat4}` uses `f32`.

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

### Available paths

`fate_engine::renderer::Renderer` currently provides two prototype paths:

- `render(&[Instance2d])` draws a built-in 2D triangle for each transform.
  `Instance2d` supplies world `position`, `rotation`, and `scale`; it does not
  provide a general 2D mesh/material API.
- `render_meshes(&[Draw3d])` draws indexed `Mesh3d` geometry with perspective
  projection and a depth buffer. `MeshVertex` contains position and vertex
  color; each `Draw3d` adds a model matrix and RGB tint.

`Renderer::create_mesh(label, vertices, indices)` uploads a mesh through the
renderer's device and returns a `Result`; `Renderer::cube_mesh()` uploads the
built-in unit cube. Both avoid naming `wgpu::Device`. The lower-level
`Mesh3d::new(device, label, vertices, indices)` remains available for headless
tools that create their own device. Mesh creation checks for empty data,
triangle index count, out-of-range indices, and unsupported index count. Meshes
are caller-owned and can be reused by multiple draw records. The current 3D
pipeline is not a full material, lighting, texture, asset-loading, or
scene-graph system.

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

The renderer caps work at `MAX_DRAWN_INSTANCES` (currently 256) and warns when
draw pressure reaches the warning threshold. This is a hard draw count limit,
not automatic culling: the consumer must filter invisible entities before
constructing the submitted list. For 2D, `Camera2d::visible_bounds(aspect)`
returns a `renderer::Bounds2d` matching the camera projection. For 3D,
`Camera3d::frustum(aspect)` returns a `renderer::Frustum3d` extracted from the
view-projection matrix (wgpu `0..=1` depth); test candidates with
`intersects_bounds(Bounds3d)` or `intersects_sphere`. `Bounds2d` and `Bounds3d`
also provide `contains_point`, `intersects`, `expanded`, `min`, and `max`. The
demo uses these helpers for both draw lists but still owns scene traversal and
object selection. Culling is not automatic, and the demo's performance HUD
remains project code.

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
  `Renderer::create_mesh`, `Renderer::cube_mesh`, `render`, and `render_meshes`
  are sufficient for window creation, surface ownership, mesh upload, and draw
  submission.

`projects/window_smoke` is a second windowed consumer that validates this seam.
Its manifest depends only on `fate_engine`, `pollster`, and `winit`; it creates
its own window, uploads a custom mesh through the renderer, handles resize, and
submits 3D draws. There is no second backend and no backend-portability trait.
Treat the `wgpu`/`winit` boundary as a prototype seam, pin dependency versions,
and do not assume future backend compatibility.

## 9. Errors, diagnostics, and operational notes

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

## 10. Verification and examples

Run the workspace quality gate after changing engine-facing behavior:

```powershell
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

### Automated quality gate

[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) runs the same
CPU-only checks on pull requests, branch pushes, and manual dispatches. CI
uses the current stable Rust toolchain with `rustfmt` and `clippy` on
`ubuntu-latest`. The local development baseline is Rust 1.99.1 on Windows
(`x86_64-pc-windows-gnu`); edition 2024 requires Rust 1.85 or newer. CI installs
the latest stable release, so a newer toolchain may report additional lints.

The workflow runs, in order:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p fate_consumer_smoke --locked
```

The consumer smoke run is a headless integration check: it exercises input,
simulation, movement, snapshots, and restoration through the public APIs. It
does not initialize a window or GPU.

Renderer tests request a test adapter when one is available and skip the GPU
cases when none exists, so the headless test job still passes on machines
without graphics hardware. A skipped renderer test is not evidence that
rendering works.

Windowed and GPU checks are intentionally separate from the required CI job.
Run `cargo run -p fate_window_smoke` and `cargo run -p fate_demo` manually on a
machine with a window and graphics adapter when renderer changes need visual
verification, and record what was exercised. They are not headless checks and
are not part of the automated gate.

### Performance measurement

Keep these measurements distinct; they answer different questions:

- `fate_sim_bench` measures fixed-step simulation work only.
- `draw_list_bench` measures CPU frustum culling and draw-list assembly only.
  It does not initialize a GPU, so render submission and GPU execution are
  excluded.
- `FrameSample`/`FrameReport` separate simulation, CPU draw-list preparation,
  and CPU render submission. GPU execution time is not measured; adding it
  would require backend timestamp queries.

Run performance commands in release mode against a fixed workload:

```powershell
cargo run --release -p fate_sim_bench -- --ticks 10000
cargo run --release -p fate_engine --example draw_list_bench
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

## 11. Keeping this guide useful

When a public capability changes, update the relevant guide section in the
same change. Include the goal, prerequisites, setup, public API, a complete
consumer flow, failure behavior, current limitations, and a link to a working
example or test. Clearly label planned functionality; do not document a design
goal as if it were usable today.

Keep this guide focused on using the engine. Put architectural invariants in
[`ENGINE_RULES.md`](../ENGINE_RULES.md), project-specific controls in
[`projects/demo/README.md`](../projects/demo/README.md), and long-term
direction in the design documents.

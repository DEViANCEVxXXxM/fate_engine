# Working with fate_engine

> **Status: early implementation.** This guide describes the checked-in
> workspace and APIs, not a stable release contract. Public interfaces,
> dependency boundaries, and behavior may change while the engine is being
> developed.

This is the practical guide for someone building a project on the current
engine. It aims to be complete about what is available and what remains a
prototype. It does not replace the longer-term [engine design document](../../engine-doc.md),
the game-specific [game design document](../../game-doc.md), or the engineering
constraints in [`ENGINE_RULES.md`](../ENGINE_RULES.md).
For current gaps and prioritized future work, see the
[engine roadmap](roadmap.md).

## 1. What the engine currently is

The codebase is a Rust workspace containing a renderer/platform-facing library,
a deterministic simulation crate, and a separate runnable demo project. The
3D co-op FPS described by the design documents is the first validation target;
the engine is not intended to hardcode that genre.

| Area | Current state | Important limitation |
|---|---|---|
| Workspace/library | Root package `fate_engine` is a library | Experimental APIs; no published versioned integration contract |
| Simulation | `fate_sim` supplies fixed-tick simulation, small ECS, deterministic RNG, snapshots, replay, and bounded rollback | `EngineCore` and its movement command currently implement one 2D prototype workload, not a general game framework |
| Input | Generic button/action state with press/release edges | Projects must translate their platform events and define their own actions and mappings |
| Rendering | `wgpu` renderer with a built-in 2D triangle path and indexed 3D mesh path | Windowing and GPU types are exposed; renderer is not backend-independent, and there is no general scene/material/asset system |
| Demo | Standalone app under `projects/demo` | Validation harness only; its scenes and controls are not engine guarantees |
| Networking | Snapshot/replay and rollback primitives exist | No transport, server authority protocol, authentication, or production networking layer |

Use the primitives as an experimental starting point. Do not assume that
planned features in a design document are already implemented.

## 2. Workspace structure

```text
fate_engine/
  Cargo.toml                 # workspace root and fate_engine library package
  src/                       # reusable input, math, renderer, and time APIs
  crates/
    fate_sim/                # headless deterministic simulation crate
    fate_sim_bench/          # CPU-only simulation benchmark
  projects/
    demo/                    # separate runnable consumer application
  docs/
    engine-guide.md          # this guide
```

Responsibilities are intentionally split:

- **`fate_engine`** — generic input state, renderer-facing floating-point math,
  window/GPU renderer foundations, and a frame/fixed-step clock.
- **`fate_sim`** — deterministic simulation primitives. It has no third-party
  dependencies and forbids unsafe code.
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
`cargo run` from the root runs the demo. Prefer explicit package selectors in
scripts and instructions:

```powershell
cargo run -p fate_demo
cargo test -p fate_engine
cargo test -p fate_sim
```

Keep the project's own `Cargo.toml`, entry point, configuration, content, and
tests under its directory. For a working consumer layout, start with
[`projects/demo/`](../projects/demo/).

## 5. A project's runtime responsibilities

There is no engine-level `Game` trait or turnkey application bootstrap yet. A
consumer currently owns its platform integration and composes the available
pieces. The demo's [`app.rs`](../projects/demo/src/app.rs) is the most complete
reference for a `winit` event loop and renderer lifecycle.

A typical frame follows this order:

1. Receive platform events; update the project's button state.
2. Advance `Time` once to collect a clamped wall-clock delta.
3. Convert the current project actions into a simulation command snapshot.
4. Consume every available fixed step, applying the command for each tick.
5. Build renderer-facing transforms from simulation state.
6. Submit a render request and handle a skipped frame if the surface is
   temporarily unavailable.
7. Clear input edge state once, after all frame consumers have read it.

The demo additionally schedules redraws through `winit`, handles resize/focus,
captures the cursor for its 3D camera, and reports performance telemetry. Those
are demo implementation choices rather than a reusable engine application
shell.

## 6. Input

The public type is `fate_engine::input::Input<Action, Button>`. Both type
parameters must be `Copy + Eq + Hash`. `Input` stores held buttons, edge state,
and a many-to-many action map. It does not know about keys, gamepad APIs,
`winit`, or game-specific action names.

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

Call `release(button)` for the corresponding platform release event. Repeated
presses while held do not create additional press edges. If several buttons
are bound to one action, the action is pressed when its first button becomes
held and released when its last held button is released. Adding/removing a
binding while a button is held does not synthesize a transition; establish
bindings before normal event processing.

Translate physical input at the project boundary. The demo defines its own
actions and `winit::KeyCode` bindings in
[`demo_input.rs`](../projects/demo/src/demo_input.rs). For deterministic
simulation, convert those actions to a simple command value; do not pass
platform input objects into simulation code.

## 7. Time and deterministic simulation

### Fixed-step clock

`fate_engine::time::Time` tracks wall-clock frame delta and an accumulator.
`Time::new(hz)` takes a frequency in ticks per second. The current Phase 1
simulation default is 120 Hz, but the clock accepts another rate:

```rust
use fate_sim::simulation::{EngineCore, SimulationInput};
use fate_engine::time::Time;

fn main() {
    let mut time = Time::new(120.0);
    let mut simulation = EngineCore::with_seed(1234);
    let input = SimulationInput {
        move_x: 1,
        move_y: 0,
        boost: false,
    };

    let frame_delta_seconds = time.tick();
    while time.next_fixed_step().is_some() {
        simulation.fixed_update(input);
    }

    let interpolation_alpha = time.alpha();
    let _presentation_values = (frame_delta_seconds, interpolation_alpha);
}
```

`tick()` clamps a single frame delta to 0.1 seconds and adds it to the fixed
accumulator. Consume steps in a loop; `next_fixed_step()` removes one step per
call. `alpha()` is the remaining accumulator fraction after consumed steps, so
consume all available steps before using it as a render interpolation factor.
The clock does not implement pause policy, a maximum number of catch-up ticks,
or a scheduler for the application.

The current `EngineCore` advances at its own fixed 120 Hz and does not accept a
configurable delta. Pair it with `Time::new(120.0)` unless your project
deliberately owns a different simulation loop; changing the clock rate alone
does not change the simulation's rate.

Use frame delta for non-deterministic presentation work, such as a free camera.
Advance deterministic state only with fixed ticks and explicit input values.
Do not read wall-clock time, renderer state, or platform devices from
simulation systems.

### Current simulation API and its limits

`fate_sim::simulation::EngineCore` is constructed with `new()` or
`with_seed(seed)`. It currently creates one player entity with a default 2D
position/velocity and `fixed_update(SimulationInput)` implements the current
prototype's movement, boost, spin, and fixed playfield boundaries.
`SimulationInput` has `move_x`, `move_y` axes intended to be `-1..=1`, and a
`boost` flag. Keep the axes in range; replay and rollback validate that range.

```rust
use fate_sim::simulation::{EngineCore, SimulationInput};

fn main() {
    let mut simulation = EngineCore::with_seed(1234);
    simulation.fixed_update(SimulationInput {
        move_x: 1,
        move_y: 0,
        boost: false,
    });
    assert_eq!(simulation.tick(), 1);
}
```

This is **not yet a configurable or general-purpose game simulation API**:
movement constants and the playfield are currently built into `EngineCore`.
Projects can access its public `world` and `player`, but arbitrary component
types, a plugin system, configurable gameplay systems, or a generic command
interface are not provided. The demo's swarm/stress-scene population is kept
outside `fate_sim` in
[`demo_simulation.rs`](../projects/demo/src/demo_simulation.rs).

### ECS and fixed-point math

The current `fate_sim::ecs::World` uses monotonically allocated `u32` entity
IDs and fixed columns for `Position`, `Velocity`, `Rotation`, and `Spin`. Each
column is indexed by entity ID and stores optional values.
`spawn()` returns an entity ID; component columns support `insert`, `get`,
`get_mut`, `remove`, `contains`, and iteration. `despawn()` removes all four
known component types. This is a deliberately small indexed-column prototype,
not the planned archetype ECS; arbitrary component registration, query
composition, entity ID reuse, and parallel scheduling are not implemented.

`fate_sim::math::Fixed` is signed Q32.32. Arithmetic saturates on overflow;
division truncates. `FixedVec2` provides the current 2D vector operations.
Convert to `f32` only when preparing renderer-facing values; converting back
does not preserve deterministic precision. The engine's renderer-facing
`fate_engine::math::{Vec2, Vec3, Mat4}` uses `f32`.

### Deterministic randomness

Use the seeded `EngineCore` RNG helpers or
`fate_sim::rng::DeterministicRng` for reproducible simulation randomness.
Available operations include `next_u64`, bounded unsigned/signed/fixed-point
ranges, and seeded shuffling. Ranges are half-open: `[min, max)`. Invalid or
empty ranges assert rather than returning an error.

The RNG state is included in `EngineCore` snapshots. To continue the same
sequence after restore, restore the same simulation snapshot rather than
seeding a new generator.

### Snapshots and replay

`EngineCore::state_bytes()` produces a versioned canonical little-endian
snapshot; `EngineCore::from_state_bytes()` validates and restores it.
`state_hash()` is an FNV-1a 64-bit diagnostic checksum. It is not
cryptographic, authenticated, or suitable as proof that remote data is
trustworthy. Snapshot format changes require a version bump and may make older
snapshots unreadable; persist an application-level version/metadata alongside
saved data if your project needs migration.

`fate_sim::replay::ReplayTape` records an initial state and a bounded list of
`SimulationInput` commands, stores a hash after each tick, serializes with
`to_bytes()`, restores with `from_bytes()`, and can verify playback with
`playback()`. It is tied to the current `EngineCore` command schema and has a
10-million-tick limit. Replay serialization has versioning but no
authentication or encryption. Treat replay files as untrusted input.

### Rollback

`RollbackSession::new(core, history_capacity)` creates a bounded local input
history; a zero capacity returns an error. Use `advance(input)` instead of
advancing its core directly, then call `reconcile(authoritative_snapshot)`
when an authoritative state arrives. `reconcile_with_corrections` can replace
predicted input for retained ticks. Reconciliation validates snapshot/tick
relationships and fails without changing live state or history if the
required input history is unavailable or corrections are invalid.

This is a deterministic state/input primitive, not a networking solution.
Transport, packet ordering, authentication, server policy, interpolation,
ownership, and production threat handling remain the consuming game's
responsibility.

## 8. Rendering

### Available paths

`fate_engine::renderer::Renderer` currently provides two prototype paths:

- `render(&[Instance2d])` draws a built-in 2D triangle for each transform.
  `Instance2d` supplies world `position`, `rotation`, and `scale`; it does not
  provide a general 2D mesh/material API.
- `render_meshes(&[Draw3d])` draws indexed `Mesh3d` geometry with perspective
  projection and a depth buffer. `MeshVertex` contains position and vertex
  color; each `Draw3d` adds a model matrix and RGB tint.

`Mesh3d::new(device, label, vertices, indices)` returns a `Result` and checks
for empty data, triangle index count, out-of-range indices, and unsupported
index count. Meshes are caller-owned and can be reused by multiple draw
records. The current 3D pipeline is not a full material, lighting, texture,
asset-loading, or scene-graph system.

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
not automatic frustum culling: the consumer must filter invisible entities
before constructing the submitted list. The demo's 2D culling and its
performance HUD are project code, not renderer features.

### Backend coupling

The renderer currently uses `wgpu` and `winit`; initialization takes an
`Arc<winit::window::Window>`, mesh creation needs a `wgpu::Device`, and the
renderer exposes GPU-related fields. This is a known prototype boundary. A
design goal of backend-facing abstractions does not mean such an abstraction
already exists. Pin or inspect dependency/API versions as part of your own
integration; do not assume future backend compatibility.

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

## 10. Verification and examples

Run the workspace quality gate after changing engine-facing behavior:

```powershell
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Useful source references:

- [`src/lib.rs`](../src/lib.rs) — engine library module surface.
- [`src/input.rs`](../src/input.rs) and
  [`projects/demo/src/demo_input.rs`](../projects/demo/src/demo_input.rs) —
  generic input state and a platform-specific mapping.
- [`src/time.rs`](../src/time.rs) — fixed-step clock.
- [`crates/fate_sim/src/simulation.rs`](../crates/fate_sim/src/simulation.rs)
  and [`crates/fate_sim/src/ecs/`](../crates/fate_sim/src/ecs/) — simulation
  and current ECS.
- [`crates/fate_sim/src/replay.rs`](../crates/fate_sim/src/replay.rs) and
  [`crates/fate_sim/src/rng.rs`](../crates/fate_sim/src/rng.rs) — replay,
  rollback, and deterministic random generation.
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

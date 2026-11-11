# Engine work review and next tasks

> **Status:** Working backlog, reviewed 2026-11-11. This records the current
> implementation in the working tree and the next engineering tasks; it is
> not a release schedule.

The engine is intentionally early. The items below distinguish foundations
that exist from capabilities that are still prototypes or design goals. Keep
this page updated as tasks are completed or reprioritized.

## What is in place

- The root `fate_engine` package is a library; the runnable consumer is a
  separate `projects/demo` workspace package.
- Deterministic simulation code is isolated in `crates/fate_sim`, which has no
  third-party dependencies and forbids unsafe code. It includes fixed-point
  scalar, 2D, and 3D vector math, a rule-neutral state container, an
  indexed-column ECS, deterministic RNG, and versioned snapshots. Demo behavior,
  replay, and rollback live under `projects/demo`.
- A standalone CPU-only simulation benchmark lives in `crates/fate_sim_bench`.
- `crates/fate_net` provides a transport-agnostic local protocol prototype:
  versioned input/snapshot packets, bounded payloads and acknowledgement
  history, ordered transactional server input application, and client snapshot
  validation. It does not open sockets or authenticate peers.
- The renderer has 2D triangle and indexed 3D mesh paths, cameras, depth
  testing, 2D/3D visibility helpers, per-draw 3D tint, and renderer-owned mesh
  upload helpers. The demo has 2D and 3D scenes and free-camera controls. A
  second windowed consumer (`projects/window_smoke`) validates the public
  renderer seam without a direct `wgpu` dependency. A CPU-only example records
  draw-list preparation cost; GPU submission is not measured.
- Reusable input state is generic over project-defined action, button, and axis
  types; analog axes accumulate per-frame deltas via `update_axis` / `axis_delta`
  and are reset by `end_frame()` / `clear()`. Platform key bindings, mouse-axis
  routing, and demo actions live in the demo project.
- The demo exercises the engine as a separate consumer. A practical user guide
  documents workspace setup, current APIs, examples, limitations, and
  verification commands.
- Workspace tests, formatting, strict Clippy, and diff checks passed after the
  latest code changes. The guide's internal links were checked after its
  expansion.

## Important gaps

- `fate_sim::ecs::World` still has a fixed component set; its current systems
  are limited to 2D movement, spin, and bounds.
- `fate_sim::ecs::World` has fixed component columns and no arbitrary
  component registration, composed queries, entity reuse, or parallel
  scheduler.
- `fate_engine::renderer::Renderer` currently depends directly on `winit` and
  exposes `wgpu` types. 3D mesh drawing is a foundation, not a general scene,
  material, asset, or lighting system.
- The renderer enforces a draw-count cap and provides `Bounds2d`/`Bounds3d`
  with camera bounds/frustum helpers for visibility checks, but there is no
  automatic culling. Consumers still traverse their scenes and choose draw
  candidates.
- Consumers must coordinate their chosen `Time` rate, per-tick delta, and
  calls to `SimulationState::advance_tick()`.
- Snapshot/replay/rollback are deterministic local primitives. `fate_net` adds
  a local protocol prototype, but there is still no transport, authentication,
  encryption, clock synchronization, interpolation, ownership model, or
  production server policy.

## Prioritized backlog

### 1. Validate the engine as an independent consumer

**Goal:** Find missing exports and accidental demo/workspace assumptions
before adding more subsystems.

**Status: done.** [`projects/consumer_smoke/`](../projects/consumer_smoke/)
is a headless package with path dependencies only on `fate_engine` and
`fate_sim`. It exercises generic input, one fixed simulation tick, and snapshot
restore without importing demo modules or starting a window/GPU. Run it with
`cargo run -p fate_consumer_smoke`.

**Done when:** The consumer builds using only documented public APIs; its
dependencies and commands are documented; the workspace quality gate passes.

### 2. Separate reusable simulation plumbing from prototype gameplay

**Goal:** Let a project own its gameplay rules without losing the useful
deterministic primitives.

**Status: done.** `fate_sim::simulation::SimulationState` now owns only world,
tick, deterministic RNG, and generic snapshot/hash behavior. The concrete
2D prototype `EngineCore`, its command shape and movement policy, and its
replay/rollback implementation are under `projects/demo`. The CPU benchmark
uses its own workload, and the headless consumer smoke test defines its own
movement behavior using reusable primitives. Snapshot compatibility is
explicitly versioned: generic state uses format version 2 and is not compatible
with the removed prototype-core snapshot schema.

**Done when:** A second minimal simulation consumer can choose different
gameplay rules without changing `fate_sim`; deterministic replay and rollback
tests still validate their documented scope.

### 3. Define fixed-step overload and pause behavior

**Goal:** Make clock behavior predictable when frames stall, windows pause, or
simulation work takes longer than the frame budget.

**Status: done.** The client never resets or drops accumulated simulation
time. `Time` caps work at eight fixed steps per rendered frame and retains any
backlog for later frames, exposing its presence and depth to the host. Its
interpolation alpha uses only the fractional remainder. The demo continues
simulation while unfocused, clears held input, skips rendering, and throttles
redraws to 20 Hz. Clock-drift compensation against a server tick is deferred
until a network clock/tick source exists; client prediction and replicated
state must use server ticks when that layer is implemented.

**Done when:** The policy is explicit, bounded according to the chosen
behavior, observable to the host app, and covered by tests.

### 4. Harden ECS lifecycle and define the next ECS capability

**Goal:** Ensure the current ECS remains correct under project use before
replacing its storage model.

**Status: done.** Generational handles (`Entity` with 32-bit ID and 32-bit generation)
prevent the ABA problem and stale component accesses. A slot free-list (`free_list`)
recycles despawned entities with incremented generations, preventing unbounded ID
growth. Lifecycle invariants strictly prevent dead or dangling entities from acquiring
components (`World::set_position` etc. return `Err(EcsError::EntityNotAlive)`). Repeated
`despawn` is idempotent and safe. Canonical snapshot encoding stores all slot generations,
the recycled free list, and live component records, with strict invariant validation during
restore (`live_count + free_list.len() == next_id`).

**Done when:** Lifecycle invariants are explicit and regression-tested, and the
next storage/scheduling change is justified by a measurable need.

### 5. Continue the 3D renderer from a renderer-only test toward a reusable
scene workload

**Goal:** Test renderer capabilities without embedding demo policy in the
renderer.

**Status: done.** `Bounds2d`, `Bounds3d`, `Camera2d::visible_bounds`, and
`Camera3d::frustum` provide 2D/3D visibility primitives. Tests cover edges,
overlap, projection alignment, window resizes (including zero-size), aspect
ratios, and degenerate input. The demo builds both draw lists with them.

`examples/draw_list_bench.rs` records a CPU-side baseline for 4,096 box
candidates: 3,296 visible per frame, 11.4 ns per candidate and 14.2 ns per
visible draw in release mode on an AMD Ryzen 9 3950X. It measures frustum
culling and draw-list assembly only, not GPU submission. Run it with
`cargo run --release -p fate_engine --example draw_list_bench`.

- Add a small render-facing bounds/frustum visibility helper or another
  renderer-owned culling primitive; keep scene traversal and object selection
  with the consumer.
- Test boundary cases for visibility and camera/projection behavior, including
  resize and degenerate inputs.
- Record a baseline for indexed-mesh draw count and CPU submission cost before
  choosing batching or instancing.
- Defer general materials, lighting, asset loading, and scene graphs until a
  second consumer demonstrates their requirements.

**Done when:** A non-demo test/consumer can prepare a visible draw list using
documented helpers, projection/depth regressions pass, and performance claims
are backed by repeatable measurements.

### 6. Reduce renderer backend coupling when a second consumer needs it

**Goal:** Avoid designing abstractions without evidence while preserving a
clear future seam.

**Status: done for the current evidence.** The audit found two coupling points:

- Window creation and resize require `winit` types at the app boundary:
  `Renderer::new(Arc<winit::window::Window>)` and
  `Renderer::resize(winit::dpi::PhysicalSize<u32>)`.
- `Renderer::gpu` and the `Gpu` fields expose `wgpu::Instance`, `Surface`,
  `Adapter`, `Device`, `Queue`, and `SurfaceConfiguration`, and mesh upload
  originally required a `&wgpu::Device`.

`projects/window_smoke` is a second, independent windowed consumer. It depends
only on `fate_engine`, `pollster`, and `winit`; it owns its window, initializes
`Renderer`, uploads a custom mesh, handles resize, and submits 3D draws. A local
run completed 180 redraws with 180 submitted frames. To keep that consumer off
`wgpu`, `Renderer::create_mesh`, `Renderer::cube_mesh`, and `Renderer::aspect`
were added as thin delegates; the demo uses the same methods. No trait or
backend abstraction was added: `winit` is still required to create a window,
`wgpu` remains visible for lower-level access, and backend portability is not
promised until a second backend exists.

- Identify which public APIs force consumers to depend directly on `winit` or
  `wgpu`. (Done; see the audit above.)
- Ask a second consumer to exercise window creation, surface ownership, GPU
  resources, and render submission before extracting interfaces. (Done;
  `projects/window_smoke`.)
- Keep backend-specific resource ownership explicit; do not promise backend
  portability until a second backend or a meaningful mock/test seam exists.
  (Kept explicit; no portability claim added.)

**Done when:** A documented consumer need motivates the abstraction and at
least two implementations or a concrete independent test seam validate it.

### 7. Revisit networking only after simulation ownership and snapshot contracts
are clear

**Goal:** Build on stable deterministic inputs and state rather than treating
rollback as a networking stack.

**Status: done for the local protocol prototype.** `crates/fate_net` provides
versioned input/snapshot packets with explicit payload and history limits, a
server session that applies ordered inputs transactionally and keeps a bounded
acknowledgement history, and a client session that validates snapshots and
rejects stale or future acknowledgements. `crates/fate_net/tests/loopback.rs`
runs both roles end to end.

Trust boundaries are explicit: client input is untrusted until the
application's applier validates it, and server snapshots are untrusted until
`SimulationState::from_state_bytes` accepts them. State hashes remain
diagnostic only and are not authentication. The prototype does not include a
transport, encryption, peer identity, clock drift compensation, interpolation,
ownership, or production server policy; inputs must arrive in order and the
transport must retransmit.

- Define the authoritative input/state exchange and threat model separately
  from serialization and local rollback. (Local exchange and trust boundaries
  are defined; production threat modeling remains open.)
- Use server ticks as the client prediction target and preserve backlog under
  the local per-frame work cap; compensate measured clock drift with small,
  bounded tick-rate adjustments (approximately 1–2%) instead of dropping
  simulation ticks. Implement this only once server-tick observations exist.
  (Server-tick observations exist; clock compensation is not implemented.)
- Specify snapshot schema evolution, input acknowledgement/history windows,
  and behavior when required history expires. (Protocol versioning, bounded
  acknowledgement history, and stale/future rejection are implemented.)
- Prototype transport independently; do not treat diagnostic hashes as
  authentication or trust arbitrary network snapshots. (Packets are
  transport-agnostic; no transport is included.)

**Done when:** A local protocol prototype has explicit trust boundaries,
bounded resource use, failure handling, and end-to-end tests across client and
server roles. This item is intentionally downstream of tasks 2 and 3.

## Suggested immediate next task

No concrete engine task is currently unblocked. The prioritized backlog is
complete for the current evidence; further work should be driven by a real
consumer:

- A production transport (for example QUIC) and clock synchronization should be
  added only when a networked consumer exists.
- Authentication, encryption, and peer identity need an explicit threat model
  before implementation.
- A `FixedQuat` or fixed-point transform type should be added only when a
  consumer needs deterministic 3D rotation.
- Further renderer backend work needs a second backend or a meaningful
  mock/test seam; the current `wgpu`/`winit` boundary is documented and
  validated by two consumers.

Pick the next task from a demonstrated consumer need rather than adding
speculative abstractions.

## Review checklist for future updates

- Does the proposed engine change solve a demonstrated consumer need?
- Is application-specific policy still outside reusable crates?
- Does the usage guide describe the actual behavior and failure modes?
- Are deterministic serialization/versioning effects covered by tests?
- Did the relevant workspace checks pass?

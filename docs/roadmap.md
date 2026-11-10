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
  math, fixed-tick updates, an indexed-column ECS, deterministic RNG,
  versioned snapshots, replay, and bounded rollback primitives.
- A standalone CPU-only simulation benchmark lives in `crates/fate_sim_bench`.
- The renderer has 2D triangle and indexed 3D mesh paths, cameras, depth
  testing, and per-draw 3D tint. The demo has 2D and 3D scenes and free-camera
  controls.
- Reusable input state is generic over project-defined action and button
  types; platform key bindings and demo actions live in the demo project.
- The demo exercises the engine as a separate consumer. A practical user guide
  documents workspace setup, current APIs, examples, limitations, and
  verification commands.
- Workspace tests, formatting, strict Clippy, and diff checks passed after the
  latest code changes. The guide's internal links were checked after its
  expansion.

## Important gaps

- `fate_sim::simulation::EngineCore` still embeds one concrete 2D player
  movement model, its input shape, constants, and playfield. It is not yet a
  general-purpose simulation host.
- `fate_sim::ecs::World` has fixed component columns and no arbitrary
  component registration, composed queries, entity reuse, or parallel
  scheduler.
- `fate_engine::renderer::Renderer` currently depends directly on `winit` and
  exposes `wgpu` types. 3D mesh drawing is a foundation, not a general scene,
  material, asset, or lighting system.
- The renderer enforces a draw-count cap but does not provide general frustum
  culling. The demo performs its own 2D visibility filtering.
- The clock and simulation rate are not yet one configurable policy: `Time`
  accepts a rate, while `EngineCore` advances at its built-in 120 Hz.
- Snapshot/replay/rollback are deterministic local primitives, not a network
  protocol or secure persistence format.

## Prioritized backlog

### 1. Validate the engine as an independent consumer

**Goal:** Find missing exports and accidental demo/workspace assumptions
before adding more subsystems.

- Add a minimal consumer example or smoke-test package that depends only on
  the engine crates it uses.
- Demonstrate generic input, a fixed-step simulation update, and the
  documented path-dependency setup without importing demo modules.
- Keep project-specific actions, content, and scene code out of reusable
  crates.

**Done when:** The consumer builds using only documented public APIs; its
dependencies and commands are documented; the workspace quality gate passes.

### 2. Separate reusable simulation plumbing from prototype gameplay

**Goal:** Let a project own its gameplay rules without losing the useful
deterministic primitives.

- First describe the smallest consumer-driven API needed to own a simulation
  tick, command data, and system selection.
- Move the current player movement, boost, spin/bounds policy, and their
  constants behind the demo/game layer or a clearly named example workload.
- Preserve `fate_sim` fixed-point, world-state, RNG, snapshot, replay, and
  rollback capabilities where they remain reusable.
- Define snapshot compatibility/versioning consequences before changing its
  serialized state schema.

**Done when:** A second minimal simulation consumer can choose different
gameplay rules without changing `fate_sim`; deterministic replay and rollback
tests still validate their documented scope.

### 3. Define fixed-step overload and pause behavior

**Goal:** Make clock behavior predictable when frames stall, windows pause, or
simulation work takes longer than the frame budget.

- Decide whether catch-up ticks are capped, accumulated indefinitely, or
  intentionally dropped, and document the time-dilation implications.
- Make the current 120 Hz `EngineCore` rate explicit at the boundary; do not
  imply that changing `Time::new(hz)` changes simulation frequency.
- Specify pause/resume behavior and input handling across focus loss.
- Add deterministic tests for long stalls, repeated overload, pause/resume,
  and interpolation alpha after consuming available steps.

**Done when:** The policy is explicit, bounded according to the chosen
behavior, observable to the host app, and covered by tests. Resolve the policy
decision before implementation rather than selecting a cap implicitly.

### 4. Harden ECS lifecycle and define the next ECS capability

**Goal:** Ensure the current ECS remains correct under project use before
replacing its storage model.

- Define behavior for invalid IDs, repeated despawn, and entity ID exhaustion.
- Ensure dead entities cannot silently acquire components that appear valid
  in snapshots or iteration.
- Document which operations require a live entity and test those invariants.
- Use a measured consumer workload to decide whether the next step is
  arbitrary component columns, query ergonomics, archetype storage, or entity
  generations; avoid implementing all at once.

**Done when:** Lifecycle invariants are explicit and regression-tested, and the
next storage/scheduling change is justified by a measurable need.

### 5. Continue the 3D renderer from a renderer-only test toward a reusable
scene workload

**Goal:** Test renderer capabilities without embedding demo policy in the
renderer.

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

- Identify which public APIs force consumers to depend directly on `winit` or
  `wgpu`.
- Ask a second consumer to exercise window creation, surface ownership, GPU
  resources, and render submission before extracting interfaces.
- Keep backend-specific resource ownership explicit; do not promise backend
  portability until a second backend or a meaningful mock/test seam exists.

**Done when:** A documented consumer need motivates the abstraction and at
least two implementations or a concrete independent test seam validate it.

### 7. Revisit networking only after simulation ownership and snapshot contracts
are clear

**Goal:** Build on stable deterministic inputs and state rather than treating
rollback as a networking stack.

- Define the authoritative input/state exchange and threat model separately
  from serialization and local rollback.
- Specify snapshot schema evolution, input acknowledgement/history windows,
  and behavior when required history expires.
- Prototype transport independently; do not treat diagnostic hashes as
  authentication or trust arbitrary network snapshots.

**Done when:** A local protocol prototype has explicit trust boundaries,
bounded resource use, failure handling, and end-to-end tests across client and
server roles. This item is intentionally downstream of tasks 2 and 3.

## Suggested immediate next task

Start with **Task 1: independent-consumer smoke test**. It is small, checks
whether the new separation and user guide work in practice, and can reveal
public API friction before larger simulation or ECS redesigns begin.

## Review checklist for future updates

- Does the proposed engine change solve a demonstrated consumer need?
- Is game-specific policy still outside reusable crates?
- Does the usage guide describe the actual behavior and failure modes?
- Are deterministic serialization/versioning effects covered by tests?
- Did the relevant workspace checks pass?

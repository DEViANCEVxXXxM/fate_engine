> **Status:** EARLY WORK. API, project structure, and architectural decisions may change significantly.

# fate_engine

**fate_engine** is an experimental game engine in Rust. It is currently early work and an architectural prototype: the project is developing around validating key technical decisions before expanding game content.

## Vision

The first validation target is a 3D cooperative FPS, not the only game the
engine should be able to host. Its needs guide priorities and benchmarks, while
game rules stay outside the reusable engine core. The project should generalize
proven primitives, not build speculative abstractions for every possible genre.

Current architectural hypotheses include:

- **Archetype-based ECS** with SoA layout and a parallel scheduler.
- **Fixed deterministic FixedUpdate** on fixed-point math.
- **Server-authoritative networking** with prediction and rollback.

The design document evaluates `ash` for Vulkan and `quinn` for transport, but
neither is a locked-in dependency: the current renderer uses `wgpu`, and both
choices should be made from prototype evidence. Consumers should depend on
engine-facing interfaces rather than backend-specific types.

Game requirements — responsiveness, procedural generation, and co-op — guide
budgets and subsystem boundaries. They must not become hardcoded game rules in
the engine core.

## Current status

The project is in early development.

- The engine is a library crate at the repository root. The demo/game
  application lives separately in [`projects/demo/`](projects/demo/) with its
  own manifest, entry point, scenes, simulation setup, controls, and tests. It
  depends on the engine library and `fate_sim`; demo-specific scene creation
  does not live in either reusable crate.
- The renderer provides both 2D and perspective indexed-mesh paths with depth
  testing. The demo exercises them; see its
  [project README](projects/demo/README.md) for run instructions and controls.

- [ ] Archetype ECS
- [ ] SoA component storage
- [ ] Parallel system scheduler
- [x] Deterministic fixed-point `FixedUpdate`
- [ ] Server-authoritative network model
- [ ] Client prediction
- [ ] Rollback and replay
- [ ] Rendering on `ash` / Vulkan
- [ ] Transport on `quinn`
- [ ] Vertical slice: scene, input, simulation, rendering, networking

The simulation now advances only through explicit 120 Hz fixed ticks and
platform-independent input snapshots. Reusable input tracking is generic over
button and action types; projects own their platform-specific bindings and
action names. Gameplay components and simulation
systems use signed Q32.32 fixed-point arithmetic; conversion to `f32` happens
only when preparing renderer-facing transforms. A golden replay regression
test applies a fixed command sequence and verifies exact fixed-point state.
The core also exposes a versioned canonical little-endian snapshot with
validated restore and an FNV-1a 64-bit diagnostic hash, checked at every
replay tick. The hash is non-cryptographic and is not suitable for
authentication. Cross-platform bitwise determinism still needs validation on
multiple targets.

`ReplayTape` can record a sequence of bounded movement commands with a
per-tick state hash, serialize it, and replay it from an initial snapshot.
Playback reports the first diverging tick; the replay format is diagnostic
and is not authenticated.

The simulation exposes a seedable deterministic random stream for
reproducible procedural generation. Its state is included in snapshots, so
restoring a snapshot also restores the exact future random sequence. The
generator provides unbiased bounded unsigned, signed, and fixed-point ranges
plus seeded Fisher–Yates shuffling. Demo scene generation is implemented in
the demo project using these reusable simulation primitives.

The replay module also provides a bounded `RollbackSession`: it accepts an
authoritative snapshot, restores it, and reapplies buffered local inputs to
the current tick. It can replace predicted commands at specific buffered
ticks before replaying. Reconciliation fails without changing live state or
input history if a correction is invalid or required history has expired.

The deterministic core, ECS, fixed-point math, replay, and rollback code live
in the `fate_sim` workspace crate. It has no graphics, windowing, or
third-party dependencies and can be tested independently with
`cargo test -p fate_sim`. The separate demo project depends on both `fate_sim`
and the root `fate_engine` library.

Run the standalone CPU-only benchmark without initializing a window or GPU:

```powershell
cargo run -p fate_sim_bench -- --ticks 10000
```

It warms up each scenario, then reports idle, player movement, and entity-load
measurements at 64, 256, and 1,024 entities. Reports include average, median,
p95, p99, and worst tick times against the exact 120 Hz simulation budget.
For less noisy results, use `cargo run --release -p fate_sim_bench -- --ticks
10000`. This is simulation-only timing, not a full-frame or GPU performance
guarantee.

## Working with the engine

See the [engine guide](docs/engine-guide.md) for workspace setup, consuming the
library, and using the current input, time, simulation, and rendering APIs.
For a review of current capabilities and a prioritized backlog, see the
[engine roadmap](docs/roadmap.md).

## Architectural principles

### Reuse without over-generalizing

- Keep reusable mechanisms in engine crates and game-specific rules in the
  application/game layer.
- Make consequential policies configurable or replaceable when real consumers
  need variation (for example, simulation rate or renderer/network backend).
- Avoid adding abstractions without a demonstrated second use case; verify
  boundaries with a small alternative consumer or test where practical.

### ECS

An archetype-based ECS model is planned: entities are grouped by component set, and component data is stored in SoA layout. This should improve data locality and make mass entity processing predictable in performance.

The parallel scheduler is intended to explicitly separate systems by data access: reads, writes, and dependencies between systems must be controllable and verifiable.

### Determinism

Simulation must run at a fixed step with fixed point. The goal is reproducibility of game state given the same sequence of inputs and commands.

This is especially important for:

- network synchronization;
- prediction and rollback;
- replays and debugging;
- procedural generation;
- co-op gameplay.

### Networking

The server is the authoritative source of game state. The client may predict its own actions for responsive control, but discrepancies are resolved via server state and rollback.

Planned model:

- the server simulates authoritative state;
- the client predicts local actions;
- the server confirms or corrects state;
- on discrepancy, rollback and resimulation are performed.

## Rendering

`ash` was chosen as a thin wrapper over Vulkan: it provides direct access to the Vulkan API while retaining control over resources, synchronization, pipelines, and memory. [3][11]

Rendering approach:

- explicit Vulkan resource management;
- minimal abstraction layer over the API;
- explicit GPU work synchronization;
- control over memory allocation and pipeline state;
- no hidden heavy dependencies in the engine core.

## Transport

`quinn` is planned as the QUIC transport. QUIC provides stream multiplexing and transport-level encryption. [6][8]

Planned separation of responsibilities:

| Layer | Responsibility |
|---|---|
| `quinn` | QUIC connections, encryption, multiplexing, transport |
| Engine network protocol | Message semantics, reliability, priorities, snapshots |
| Simulation | Deterministic `FixedUpdate`, commands, state |
| Client | Input, prediction, interpolation, rollback |

## Subsystem boundaries

The engine core must not know about a specific game. Game systems should be built on top of the core and use its primitives.

| Subsystem | Responsible for | Not responsible for |
|---|---|---|
| ECS | Storing entities and components, scheduling systems | Game logic |
| Simulation | Deterministic `FixedUpdate`, fixed-point math | Rendering and network transport |
| Networking | Transport, protocol, replication, rollback | Specific game rules |
| Rendering | Vulkan resources, frame, synchronization | Game logic |
| Assets | Loading and preparing resources | Gameplay decisions |

## Vertical slice

The first priority is not content, but validation of the load-bearing pillars.

The vertical slice must confirm that the architecture withstands load:

1. Creation and processing of a large number of entities in ECS.
2. Deterministic simulation in `FixedUpdate`.
3. Scene rendering via `ash` and Vulkan.
4. Network connection via `quinn`.
5. Basic server-authoritative replication.
6. Prediction and rollback on a minimal example.
7. Measurement of frame, memory, and network traffic budgets.

Only after confirming these pillars is it planned to expand content, tools, and gameplay systems.

## Target requirements

- Responsive control: minimal delay between input and visual reaction.
- Procedural generation: reproducible results with the same seed and input data.
- Co-op: predictable network simulation and state conflict resolution.
- Performance: controlled budgets for frame, memory, CPU, and network.
- Extensibility: game systems must not modify the engine core.

## Technologies

- **Rust** — the project's primary language.
- **ECS** — the architectural foundation of simulation.
- **SoA layout** — component data layout.
- **Fixed-point** — deterministic simulation math.
- **ash** — Vulkan bindings for Rust. [3][11]
- **quinn** — async QUIC transport for Rust. [6][8]

## Roadmap

### Stage 1 — foundation

- Basic workspace and module structure.
- Minimal ECS.
- Basic determinism tests.
- Rendering skeleton on `ash`.

### Stage 2 — vertical slice

- Fixed-point `FixedUpdate`.
- Parallel system scheduling.
- Basic scene rendering.
- Basic QUIC transport on `quinn`.

### Stage 3 — networking

- Server-authoritative simulation.
- Client commands and server snapshots.
- Client prediction.
- Rollback and reconcile.

### Stage 4 — performance

- ECS benchmarks.
- Rendering profiling.
- Frame and memory budgets.
- Network protocol optimization.

## License

License not yet chosen.

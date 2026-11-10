> **Status:** EARLY WORK. API, project structure, and architectural decisions may change significantly.

# fate_engine

**fate_engine** is an experimental game engine in Rust. It is currently early work and an architectural prototype: the project is developing around validating key technical decisions before expanding game content.

## Vision

The engine is built around three load-bearing pillars:

- **Archetype-based ECS** with SoA layout and a parallel scheduler.
- **Fixed deterministic FixedUpdate** on fixed-point math.
- **Server-authoritative networking** with prediction and rollback.

Rendering is planned via `ash` to retain direct control over Vulkan without an excessive abstraction layer. The transport layer is planned on `quinn`: QUIC provides stream multiplexing and built-in encryption.

Game requirements — responsiveness, procedural generation, and co-op — are treated as constraints on budgets and subsystem boundaries. They must not leak into the core architecture.

## Current status

The project is in early development.

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

## Running the current demo

Run the windowed prototype with `cargo run`. The demo starts with a small moving
swarm plus off-screen entities. The camera follows the larger player marker,
making fixed-step movement, world bounds, camera culling, and frame telemetry
visible together.

- **W/A/S/D** or **arrow keys** — move the player.
- **Left Shift** — boost movement.
- **F2** — toggle the 192-visible-instance stress scene. It reaches 75% of the
  draw budget and should report a warning tier while off-screen entities remain
  culled.
- **Escape** — close the demo.

The window title reports frame timings, rolling averages/worst-case, draw
pressure, budget headroom, and current policy tier.

The simulation now advances only through explicit 120 Hz fixed ticks and
platform-independent input snapshots. Gameplay components and simulation
systems use signed Q32.32 fixed-point arithmetic; conversion to `f32` happens
only when preparing renderer-facing transforms. A golden replay regression
test applies a fixed command sequence and verifies exact fixed-point state.
This strengthens deterministic behavior, but cross-platform bitwise
determinism still needs validation on multiple targets and a stable state
serialization/hash format.

## Architectural principles

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

# Engine improvement roadmap

> **Status:** Working plan, reviewed 2026-11-11. This is a sequence of small
> engineering steps, not a release schedule or a stable API promise.

## Current baseline

- `fate_engine` is a library providing generic input, frame/fixed-step timing,
  floating-point math, and `wgpu`/`winit` rendering foundations.
- Dependency-free `fate_sim` provides fixed-point scalar/vector math, a seeded
  RNG, generational entity handles with slot reuse, fixed component columns,
  and a rule-neutral state container with versioned snapshots.
- Concrete commands, gameplay tick policy, replay, and rollback belong to
  `projects/demo`; they are not reusable `fate_sim` APIs.
- `fate_net` provides a transport-agnostic local protocol and ordered
  client/server sessions. It does not provide sockets, authentication,
  encryption, clock synchronization, or prediction/reconciliation.
- The renderer supports built-in 2D triangles and indexed 3D meshes with depth
  testing, tint, cameras, and visibility helpers. Consumers own scene traversal
  and culling decisions; there is no general asset, material, or lighting system.
- Headless and windowed smoke consumers exercise the public APIs independently
  of the demo. CPU benchmarks exist, but do not establish GPU performance.

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

- Document the relationship between packet `server_tick` and the tick encoded
  in `SimulationState`; reject contradictions before accepting a snapshot.
- Test inconsistent ticks, stale snapshots, and recovery by a subsequent valid
  packet. Verify that a rejected packet leaves the client session unchanged.
- Keep checksums explicitly diagnostic; these checks do not authenticate peers.

**Done when:** Accepted snapshots have an explicit, enforced tick contract and
invalid metadata cannot advance the client's accepted server tick.

## Step 3 — Automate the quality gate

**Depends on:** Steps 1–2 for the new regression cases; existing checks can be
added to CI independently.

- Add CI for `cargo test --workspace`, `cargo fmt --all -- --check`, and
  `cargo clippy --workspace --all-targets -- -D warnings`.
- Run the headless smoke consumer as an integration check.
- Document the tested toolchain/platform. Keep windowed/GPU checks separate
  from headless CI and record them when renderer changes require them.

**Done when:** The automated gate runs on pull requests and branch updates;
required CPU-only checks do not need a window or GPU adapter.

## Step 4 — Clarify timing and performance measurements

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

- Choose one compact original gameplay scenario with movement, an interaction,
  and a clear success/failure condition.
- Keep its rules, commands, and content in a project, not in reusable crates.
- Exercise state save/restore and deterministic replay of the chosen scenario.
- Record actual missing engine capabilities and implement the smallest one
  needed to complete the scenario.

**Done when:** The scenario is playable through documented public APIs, its
simulation is regression-tested, and the next engine task has a concrete
consumer rather than a speculative abstraction.

## Deferred until a consumer needs them

- Arbitrary ECS components, composed queries, and parallel scheduling.
- Fixed-point 3D rotation/transform types.
- General assets, materials, lighting, scene graphs, and renderer backends.
- Network transport, peer identity, encryption, clock synchronization, and
  multiplayer prediction/reconciliation. Define a threat model before adding
  production networking.

For each step, update the [engine guide](engine-guide.md) if observable behavior
changes and keep application policy outside reusable crates. Choose a license
before presenting the repository as ready for external adoption.

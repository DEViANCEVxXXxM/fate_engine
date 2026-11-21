# fate_engine

An experimental Rust game engine for building and measuring deterministic
simulations and 2D/3D applications. The engine provides the primitives;
applications own their rules, content, and presentation.

**Status:** Early development. APIs and file formats can change. The
[roadmap](docs/roadmap.md) distinguishes tested capabilities from open work.

## Quick start

Use current stable Rust. Run these commands from the repository root:

```sh
# Open the showcase and its native diagnostics window.
cargo run --release --locked -p fate_demo -- --dbg

# Run a deterministic workload without a window, GPU adapter, or audio device.
cargo run --release --locked -p fate_polygon -- --headless --preset ecs
```

The showcase needs a desktop session and a `wgpu`-compatible adapter. On Linux,
see the [build prerequisites](docs/engine-guide.md#3-requirements-and-first-run).
Native Linux runtime validation is still open.

## What’s included

| Component | Use it for |
|---|---|
| `fate_engine` | Input, fixed-step timing, windowed and offscreen rendering, CPU scene picking, and procedural audio |
| `fate_sim` | Dependency-free fixed-point math, entity storage, seeded randomness, and versioned state snapshots |
| `fate_net` | A local protocol prototype and client/server session state; no socket transport |
| [`fate_debug`](crates/fate_debug/) | Optional native diagnostics, inspection, and trace export |
| [Showcase](projects/demo/) | See the engine in use, or record and inspect a deterministic replay headlessly |
| [Polygon](projects/fate_polygon/) | Stress-test rendering, ECS and snapshots, or lab-owned physics and AI; compare runs with CSV baselines |

## Build and verify

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --locked -p fate_consumer_smoke
```

The required checks don’t need a window, GPU adapter, or audio device. The
[verification guide](docs/engine-guide.md#automated-quality-gate) explains
what Ubuntu and Windows CI run and how to exercise the optional desktop checks.

## Documentation

- [Engine guide](docs/engine-guide.md) — setup, integration, and API examples.
- [Roadmap](docs/roadmap.md) — verified progress and next milestones.
- [Documentation index](docs/) — choose a guide or example for a specific task.

No license has been selected.

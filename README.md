> **Status:** EARLY WORK. API, project structure, and behavior may change significantly.

# fate_engine

**fate_engine** is an experimental game engine written in Rust. It is early
work: the project currently focuses on validating technical foundations before
expanding content.

## Current status

- The engine is a library crate at the repository root.
- A standalone demo application lives in [`projects/demo/`](projects/demo/); it
  depends on the engine and is not part of the reusable library.
- A small playable consumer lives in
  [`projects/combat_slice/`](projects/combat_slice/); it exercises the engine
  through a fixed-step first-person arena scenario and is also not part of the
  reusable library.
- The renderer provides 2D and indexed-mesh 3D paths, with depth testing and
  opaque textures for 3D meshes. It also provides screen-space UI rectangles
  and textured images with atlas regions, sampling
  options, clipping, and rotation, and a built-in bitmap-font text overlay with
  width-aware wrapping.
- Audio provides optional procedural one-shot tones through the platform output
  device, with a bounded mixer and a no-device fallback.
- Deterministic simulation primitives live in the `fate_sim` workspace crate:
  fixed-point math, an indexed-column ECS, seeded RNG, and versioned snapshots.
  Concrete input commands, replay, and rollback currently live in the demo.
- The headless consumer smoke test in
  [`projects/consumer_smoke/`](projects/consumer_smoke/) validates the public
  APIs outside the demo.
- A second, minimal windowed consumer in
  [`projects/window_smoke/`](projects/window_smoke/) exercises the renderer's
  public seam without a direct `wgpu` dependency.
- A transport-agnostic `fate_net` crate provides a versioned local protocol
  prototype with bounded payloads, ordered inputs, acknowledgement history, and
  snapshot validation. It does not open sockets.
- Reusable frame diagnostics and metric sink/source seams live in
  `fate_engine::diagnostics`; `fate_sim` and `fate_net` expose read-only
  inspection snapshots. Consumers own presentation and export.
- [`fate_debug`](crates/fate_debug/) provides an opt-in native diagnostics
  companion with live charts, inspectors, event filtering and capture/export.
  Render telemetry attaches to existing 2D/3D renderers without changing draw calls.

## Build and test

```powershell
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p fate_demo
cargo run --release -p fate_demo -- --dbg
cargo run -p fate_combat_slice
```

CI runs the headless parts of this gate (formatting, Clippy, workspace tests,
and the consumer smoke test) on pull requests and branch pushes. Windowed and
GPU checks stay manual; see the
[engine guide](docs/engine-guide.md#automated-quality-gate).

On Linux, building the workspace needs the `libudev` and ALSA development
packages. The combat slice's controller support links against `libudev`, and
the engine's audio output links against `libasound2` through `pkg-config`:

```sh
sudo apt-get install -y libudev-dev libasound2-dev
```

The CPU-only simulation benchmark does not initialize a window or GPU:

```powershell
cargo run -p fate_sim_bench -- --ticks 10000
```

It reports average, median, p95, p99, and worst tick times for idle, movement,
and entity-load scenarios against the 120 Hz simulation budget. Use `--release`
for less noisy results; this is simulation-only timing, not a full-frame or GPU
performance guarantee.

The CPU draw-list baseline is separate and does not initialize a GPU:

```powershell
cargo run --release -p fate_engine --example draw_list_bench
```

It measures frustum culling and draw-record assembly only. Render submission
and GPU execution are excluded, so it is not a full-frame measurement.

## Documentation

- [Engine guide](docs/engine-guide.md) — workspace setup, library usage, and the
  current input, time, simulation, and rendering APIs.
- [Roadmap](docs/roadmap.md) — current capabilities and prioritized backlog.
- [Archive](docs/archive/) — superseded engine notes kept for history.

Design notes are maintained privately and are not part of this repository.

## License

License not yet chosen.

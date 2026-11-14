# fate_demo

`fate_demo` is the standalone runnable example project. It consumes the engine
from the repository root plus the reusable `fate_sim` crate, and shows how a
consumer can combine fixed-step simulation, 3D rendering, culling, diagnostics,
and replay/rollback verification in one window.

Run from the repository root:

```powershell
cargo run -p fate_demo
```

Or run `cargo run` from this directory.

Frame timings are only representative in an optimized build; use
`cargo run --release -p fate_demo` when evaluating the reported timings.

The app renders an arena containing an animated cube grid. The grid follows a
scripted deterministic input pattern at 120 Hz; the camera is for inspection
only and does not affect simulation state. There is no gameplay loop: no
objectives, scoring, or win/lose rules.

## What it demonstrates

- Indexed 3D cube meshes with depth testing and per-cube frustum culling.
- Draw-instance budgeting and draw-pressure reporting.
- Fixed-step simulation with interpolation between ticks.
- Phase-separated frame diagnostics (simulation, draw-list preparation, CPU
  submission) with rolling averages, budget tiers, and headroom.
- A consumer-owned diagnostics sink and source; the app publishes structured
  values through `log` and the window title.
- State hashing and periodic snapshot round-trip checks.
- Replay recording and playback verification.
- Rollback reconciliation against authoritative snapshots.

## Controls

- **Tab** — switch between the auto-orbit camera and the free-fly camera.
- **W/A/S/D** or **arrow keys** — move the free camera.
- **Mouse** — look around while the free camera has captured the cursor.
- **Left Shift** — move the free camera faster.
- **Space** — pause or resume the simulation.
- **F1** — toggle periodic verbose diagnostics logging.
- **F2** — toggle replay verification.
- **F4** — run a snapshot round-trip check immediately.
- **Escape** — release the cursor; press again (or from orbit mode) to quit.

The free camera has no collision and is not a character controller.

## Diagnostics

The engine has no built-in profiler, overlay, or text renderer, so presentation
is owned by the demo:

- The window title refreshes twice per second with frame timings, draw load,
  budget tier, simulation tick, state hash, and verification status.
- A status line is logged every two seconds regardless of the verbose setting.
- **F1** gates only the verbose structured sink: while it is enabled, buffered
  `key=value` diagnostics are published about once per second through the
  `fate.showcase` log target.
- Verification failures are always logged with their detailed error message,
  even when verbose diagnostics are off.
- Frame-budget tier changes are logged as they happen: warnings for `warning`
  or `critical`, and an info line when the frame returns to `healthy`.

The default `env_logger` filter is `fate.showcase=info,warn`. Override it with
`RUST_LOG` when running, for example:

```powershell
$env:RUST_LOG = "fate.showcase=debug"
cargo run -p fate_demo
```

## Verification

The simulation is deterministic and independent of the window and GPU. The
demo checks it continuously:

- Every 240 ticks, a 240-tick replay window is replayed from a recorded state
  and its final hash is compared with the live state.
- Every 240 ticks, a rollback session is reconciled against a snapshot from
  120 ticks earlier and resimulated to the current tick.
- Every 240 ticks, the current state is serialized, decoded, and compared.

Results appear in the title, the status log, and the structured diagnostics.
`cargo test -p fate_demo` runs the headless tests for the simulation, scene
culling, diagnostics sink/source, replay, rollback, and snapshot checks.

Windowed rendering still requires a working window-system backend and a GPU
adapter supported by `wgpu`.

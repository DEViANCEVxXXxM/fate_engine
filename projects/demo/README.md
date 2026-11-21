# fate_demo

`fate_demo` is the engine showcase: an animated 3D cube grid driven by
scripted input at 120 Hz. The camera is for inspection and does not affect
the simulation. There are no gameplay objectives, scoring, or networking.

Run from the repository root:

```sh
cargo run --locked -p fate_demo
cargo run --release --locked -p fate_demo -- --dbg
```

Or run `cargo run` from this directory.

Run an optimized build (`--release`) when evaluating timings.

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
- Local rollback reconciliation against snapshots.

## Headless replay inspection

Record a 240-tick sample of the showcase's normal input script, then verify it:

```sh
cargo run --locked -p fate_demo -- --record-replay showcase.frep
cargo run --locked -p fate_demo -- --inspect-replay showcase.frep
```

Recording will not overwrite an existing file. The inspector accepts showcase
replay files (not standalone snapshots or other apps' recordings), up to
128 MiB. It prints the initial and final ticks, checks every recorded tick in
order, and reports the first checksum mismatch. Exit codes: `0` on success,
`1` on divergence, `2` on invalid arguments, unreadable files, or invalid
data. Neither command initializes a window, GPU, or audio device.

## Controls

- **Tab** — switch between the auto-orbit camera and the free-fly camera.
- **W/A/S/D** or **arrow keys** — move the free camera.
- **Mouse** — look around while the free camera has captured the cursor.
- **Left Shift** — move the free camera faster.
- **Space** — pause or resume the simulation.
- **F1** — toggle periodic verbose diagnostics logging.
- **F2** — toggle replay verification.
- **F3** — reopen the diagnostics companion when launched with `--dbg`.
- **F4** — run a snapshot round-trip check immediately.
- **Escape** — release the cursor; press again (or from orbit mode) to quit.

The free camera has no collision and is not a character controller.

## Diagnostics

Diagnostics presentation is consumer-owned:

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

### Native companion (`--dbg`)

Opens a separate dark-green diagnostics window:

- **Overview:** frame cadence, CPU phases, asynchronous GPU duration, draw counts,
  hoverable history, adjustable budget marker and trace freeze/resume.
- **Renderer:** device/driver/limits, GPU history, culling toggle and instance cap.
- **Simulation:** live state/hash, pause, single tick, reset, replay toggle and
  immediate snapshot round-trip verification.
- **Camera:** orbit/free mode, reset, position, yaw/pitch, field of view and clip planes.
- **Entities:** filterable render candidates, frustum visibility, selection and
  copyable world positions.
- **Events:** engine logs (respecting `RUST_LOG`), input, lifecycle and tool results.
- **Capture:** CPU/GPU CSV, inspection/event report, clipboard copy and binary
  simulation snapshots. Snapshot files go to `diagnostics-captures/`; trace/report
  exports use the directory entered in the window. Existing files are not overwritten.

Both windows continue updating while unfocused. Closing the companion leaves
the showcase running; F3 reopens it. Minimized windows skip drawing while
bounded telemetry continues collecting. Freeze holds CPU/GPU history only;
inspection and events remain live. Closing the showcase ends the session.

History retains 1,200 samples per stream and 500 events. The UI refreshes at
20 Hz; entity/event lists render only visible rows. The companion uses its own
GPU device: its rendering is outside scene pass timings, but its load can still
affect scene frame cadence. Compare optimized runs with and without `--dbg`.

Run the optional window/GPU regression:

```powershell
cargo run --release --locked -p fate_demo -- --dbg-smoke
```

The reusable integration API is documented in
[`fate_debug`](../../crates/fate_debug/README.md).

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

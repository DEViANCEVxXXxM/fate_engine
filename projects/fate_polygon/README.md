# fate_polygon

A native interactive stress lab for Fate. Rendering, ECS, and snapshots use
engine APIs. Physics and AI are **lab-owned workloads**, not general engine modules.

```powershell
cargo run --release --locked -p fate_polygon
cargo run --release --locked -p fate_polygon -- --dbg
```

The app opens a viewport and a separate native controls window. The controls
start on **Simulation**. Both windows work while unfocused. Closing Controls
leaves the viewport running; press **F3** in the viewport to reopen it. Press
**Space** to pause/resume fixed steps and **R** to reset to the current seed.
`--dbg` adds GPU timestamps on supported devices; live CPU counters work
without it.

| Page | Workload and live controls |
|---|---|
| **Simulation** | Mixed, Render, Physics, AI, Navigation, and ECS presets; pause, single-step, reset, seed, and fixed rate (15–240 Hz). Physics: up to 384 disks with all-pairs or grid search. AI: up to 1,024 agents with steering or navigation. ECS: up to 8,192 entities, replacement rate, and snapshot cadence. |
| **Renderer** | Toggle the stress grid, request up to 4,096 candidates, change mesh tessellation, split material runs, enable frustum culling, orbit the camera, adjust the redraw limit or toggle simulation markers. |
| **Overview / Capture** | Live cadence, CPU phase and draw statistics, optional GPU pass timings, and CSV trace export. |

Presets restore their settings and restart the simulation. Changing the seed
restarts it as well. Workload toggles are independent. Turn **Simulation
preview** off to measure Physics or AI without drawing their markers. The
preview shows up to 40 objects per enabled workload; it does not limit how many
are simulated. Preview draws share the engine's instance budget with the grid.
Use the Renderer page's candidate, culling and submission counters to see where
that budget clips work. Increasing tessellation or decreasing the material-run
size increases actual GPU or draw-call work without relying on over-cap inputs.

Physics performs bounded disk contact and wall reflection. The AI preset samples
neighbors and steers toward a periodically changing target with separation.
CPU timings are per-frame phase timings; GPU timestamps cover scene and HUD
passes separately. The controls window has its own GPU work and can still
affect overall cadence. Close Controls for an isolated viewport run.

## Repeatable CPU runs

Use `--headless` to run the same seeded Physics, AI, and ECS workloads for a fixed
number of ticks, without a window, GPU adapter or audio device. Rendering is
not measured in this mode. Preset names and `--set NAME=VALUE` overrides can
also be used when launching the live app.

```powershell
cargo run --release --locked -p fate_polygon -- --headless --preset physics --set seed=71 --sweep bodies=64,128,256 --warmup 120 --ticks 600 --csv physics-sweep.csv
cargo run --release --locked -p fate_polygon -- --headless --preset ai --sweep samples=8,24,48
cargo run --release --locked -p fate_polygon -- --preset render --set detail=40 --set batch=4 --dbg
```

Each sweep starts from the preset plus overrides, then applies the swept
value. Only one numeric setting can be swept per run (up to 32 distinct values).
Use `--help` to list names and ranges. Headless rejects live-only settings and
configurations with all CPU workloads off.

Reports include p50/p95 elapsed time per tick and enabled phase, measured work
counts, and a final state checksum. Warmup advances the simulation but does not
contribute timing or counts. Compare the checksum and work counts for repeated
runs with the same settings; elapsed time will vary. CSV contains one row per
case with the effective settings and will not overwrite an existing file.

To compare the physics pair searches, sweep `broad_phase`: `0` is the default
all-pairs scan, `1` is the spatial grid. Both use the same solver and contact
order. The grid updates during contact resolution so a disk pushed into a
new cell still finds its next contacts during that pass.

```powershell
cargo run --release --locked -p fate_polygon -- --headless --preset physics --sweep broad_phase=0,1 --csv physics-search.csv
```

Compare pair-test counts and CPU timing; contact counts and final state hashes
should match. The grid's performance depends on how densely bodies cluster.
In the live app, use **Simulation → Physics / pair search** to switch modes.

### Navigation workload

The **Navigation** preset moves agents one cell per decision through an
obstacle grid toward a rotating shared goal. The preview shows the walls and
current goal. `nav_mode=0` selects sampled steering, `1` runs a reverse
breadth-first search per moving agent, and `2` caches one distance field for
all agents until the goal changes. Both navigation modes choose the same paths.
`think_every` sets decision cadence in all AI modes; `nav_goal_ticks` sets
goal cadence only for navigation. Steering speed and neighbor sample count do
not affect navigation.

```powershell
cargo run --release --locked -p fate_polygon -- --preset navigation --dbg
cargo run --release --locked -p fate_polygon -- --headless --preset navigation --sweep nav_mode=1,2 --csv routes.csv
```

Compare searches and cells visited, along with CPU timing. Route moves and
final state hashes should match. This workload shares a goal and treats other
agents as nonblocking; it does not measure multi-agent avoidance or path
reservation.

### ECS and snapshot workload

The **ECS** preset replaces a configurable number of entities each tick,
then runs `fate_sim` movement, spin, and bounds systems. At the chosen
cadence, it encodes the world, restores it through the engine decoder,
verifies the canonical bytes, and continues from the restored state.
The live controls report occupied/allocated slots, update time, and
snapshot work. Disabling ECS discards its world; re-enabling starts
a fresh world from the configured seed.

```powershell
cargo run --release --locked -p fate_polygon -- --preset ecs --dbg
cargo run --release --locked -p fate_polygon -- --headless --preset ecs --sweep snapshot_every=0,15,60 --csv ecs-sweep.csv
```

`entities` sets population; `churn` replaces up to that many live entities
per tick, and `snapshot_every=0` turns restore off. ECS update timing excludes
snapshot time. Snapshot p50/p95 covers encode, decode, byte verification, and
restore per snapshot, not per tick. Disabling restore should leave the final
state checksum unchanged. ECS-enabled CSVs include additional settings and
metrics; runs with ECS off keep the existing CPU CSV format.

## Windowless render runs

`--offscreen` runs the live stress mesh grid on a fixed-size GPU target, with
a fixed camera and no simulation, HUD, presentation, or pixel readback. It
requires a GPU adapter. It reports CPU draw-list preparation, CPU recording
and submission, and GPU scene time separately. There is one frame in flight;
the completion wait is outside the measurements. These are phase timings, not
an estimate of windowed FPS.

```powershell
cargo run --release --locked -p fate_polygon -- --offscreen --size 960x540 --sweep detail=8,24,48 --csv render-sweep.csv
cargo run --release --locked -p fate_polygon -- --offscreen --set candidates=256 --sweep batch=1,8,256 --frames 120
```

The summary includes candidates, post-cull work, actual submissions, draw
calls and submitted triangles. `--set candidates=0` provides a clear-only
baseline. If timestamps are unsupported, GPU timings stay blank in CSV and
the CPU measurements still run. `--help` lists the supported parameters.

## Compare to a baseline

Run the same benchmark before and after a change, saving each run to a different
CSV, then compare the two files:

```powershell
cargo run --release --locked -p fate_polygon -- --compare baseline.csv current.csv
```

The comparison matches cases by settings, not row order or preset label. It
also requires the same adapter, backend, driver, and resolution for offscreen
runs. Use current Polygon exports on the same machine under comparable
conditions. An offscreen comparison requires complete GPU samples in both
runs, or no GPU samples in either run for a CPU-only comparison.

Each p50/p95 increase fails only if it exceeds both `--max-regression-pct` (default
**10%**) and `--noise-floor-ms` (default **0.005 ms**). A changed state checksum
or work count fails separately, and timings for that case are not compared.
Exit codes are **0** for pass, **1** for changed work or a timing regression,
and **2** for invalid or incomparable inputs. The comparison itself is CPU-only,
even when reading offscreen reports.

## Checks

```powershell
cargo test --locked -p fate_polygon
cargo run --release --locked -p fate_polygon -- --dbg --smoke
```

Unit tests need no window, GPU adapter or audio device. `--smoke` is a live
window test that cycles through all six presets and exits after checking
that the expected work and submissions occurred.

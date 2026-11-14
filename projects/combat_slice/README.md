# fate_combat_slice

`fate_combat_slice` is a small playable consumer project. It is not part of the
reusable library: it owns its rules, input mapping, scene content, and
presentation, and depends on the engine only through public APIs.

The slice is a compact first-person arena scenario. It exercises fixed-step
simulation, first-person 3D rendering, frustum culling, deterministic replay,
and snapshot round trips in one interactive window.

Run from the repository root:

```powershell
cargo run -p fate_combat_slice
```

Or run `cargo run` from this directory. Frame timings are only representative in
an optimized build; use `cargo run --release -p fate_combat_slice` when
evaluating the reported timings.

## What it demonstrates

- A 120 Hz fixed-step simulation built on `fate_sim` fixed-point math, with no
  wall-clock or renderer dependency.
- First-person `Camera3d` rendering of an indexed cube arena and dynamic cubes.
- Consumer-owned frustum culling and draw-instance budgeting.
- Render-only interpolation between fixed states.
- Continuous replay verification and periodic snapshot round trips.
- A playable loop with explicit victory and defeat conditions.

## Controls

- **W/A/S/D** or **arrow keys** — move.
- **Mouse** — look, while the window has captured the cursor.
- **Left Shift** — sprint.
- **Space** or **left mouse button** — fire.
- **R** — reload.
- **P** — pause or resume the simulation.
- **F1** — toggle verbose status logging.
- **F2** — toggle replay verification.
- **F4** — run a snapshot round-trip check immediately.
- **Enter** — restart after victory or defeat.
- **Escape** — release the cursor; press again to quit.

Clicking the window recaptures the cursor after it has been released. Losing
window focus releases the cursor and clears held input.

## Rules

The scenario runs until it reaches a terminal outcome:

- The player starts with 100 health and a 10-round magazine.
- A shot deals 18 damage and has a 15-tick cooldown.
- Reloading takes 84 ticks and refills the magazine.
- Firing with an empty magazine does nothing; press **R** to reload.
- Six enemies start with 30 health each and approach the player.
- Contact with an enemy deals 8 damage, at most once every 36 ticks.
- The run is a victory when every enemy is defeated.
- The run is a defeat when the player's health reaches zero or the 90-second
  time limit expires.

## Presentation

The engine has no text renderer or HUD, so the slice owns its presentation:

- The window title refreshes twice per second with the outcome, health, ammo,
  enemy count, tick, frame timings, draw load, verification status, and state
  hash.
- A status line is logged every two seconds. **F1** toggles the verbose
  structured form; a shorter summary is always logged.
- Outcome changes and verification failures are always logged.

The default `env_logger` filter is `fate.combat=info,warn`. Override it with
`RUST_LOG` when running, for example:

```powershell
$env:RUST_LOG = "fate.combat=debug"
cargo run -p fate_combat_slice
```

## Verification

The simulation is deterministic and independent of the window and GPU:

- Every 240 ticks, a 240-tick in-memory replay window is recorded from a
  snapshot and replayed; its final hash is compared with the live state.
- Every 240 ticks, the current state is serialized, decoded, and compared.
- **F4** forces an immediate snapshot round-trip check.

Results appear in the window title and the status log. `cargo test -p
fate_combat_slice` runs the headless tests for simulation rules, replay,
snapshot codec, interpolation, culling, input bindings, and app behavior. Those
tests do not need a window or GPU adapter.

## Limitations

- Rendering requires a working window-system backend and a GPU adapter supported
  by `wgpu`; windowed checks are manual.
- There is no text or HUD renderer, so all player-facing status is presented
  through the window title and logs.
- `fate_sim` has no fixed-point trigonometric functions. The slice keeps aim as
  unit yaw/pitch vectors and rotates them with project-owned small-angle
  polynomials, then converts to float only for the camera.
- The scene uses untextured cubes only: there are no assets, materials,
  lighting, or shadows.
- The slice does not use `fate_net`; it has no networking, prediction, or
  rollback. Rollback is demonstrated by `projects/demo`.

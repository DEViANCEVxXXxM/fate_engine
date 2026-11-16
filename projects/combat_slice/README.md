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

On Linux, building the slice needs the `libudev` development package, because the
controller backend links against it:

```sh
sudo apt-get install -y libudev-dev
```

## What it demonstrates

- A 120 Hz fixed-step simulation built on `fate_sim` fixed-point math, with no
  wall-clock or renderer dependency.
- First-person `Camera3d` rendering of an indexed cube arena and dynamic cubes.
- An engine-drawn text HUD layered over the scene in the same frame.
- Consumer-owned frustum culling and draw-instance budgeting.
- Render-only interpolation between fixed states.
- Keyboard, mouse, and Xbox/DualSense controller input merged into one
  fixed-step command stream, with analog stick walk speed.
- Continuous replay verification and periodic snapshot round trips.
- A playable loop with explicit victory and defeat conditions.

## Controls

Keyboard and mouse:

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

Controller (Xbox and DualSense):

- **Left stick** or **D-pad** - move. Stick deflection sets walk speed; the
  D-pad moves at full speed.
- **Right stick** - look.
- **Right trigger (RT / R2)** or **right bumper (RB / R1)** - fire.
- **Left trigger (LT / L2)**, **left bumper (LB / L1)**, or **left stick click** -
  sprint.
- **X (Xbox)** / **Square (DualSense)** - reload.
- **Start (Xbox)** / **Options (DualSense)** - pause or resume.
- **View (Xbox)** / **Create (DualSense)** - restart after victory or defeat.
- **Y (Xbox)** / **Triangle (DualSense)** - toggle verbose status logging.

Controllers are optional and can be connected or disconnected while the slice
runs. The first connected pad drives the slice; if it disconnects, another
connected pad takes over. The slice logs every pad it finds together with the
backend that is reading it.

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

The slice draws an in-window HUD on top of the scene:

- Top-left status lines show health, ammo, enemies, remaining time, frame rate,
  frame timings, replay and snapshot status, tick and state hash, and draw load.
- A centered **PAUSED** banner appears while the simulation is paused, and
  **VICTORY** or **DEFEAT** appears when the run ends.

The HUD is only presentation: the same state is still published through the
window title and `log`, so status remains available outside the window. HUD
sizes are laid out in physical pixels and scale with the display scale factor,
so they stay readable on high-DPI screens.

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

Results appear in the HUD, the window title, and the status log. `cargo test -p
fate_combat_slice` runs the headless tests for simulation rules, replay,
snapshot codec, interpolation, culling, input bindings, controller mapping and
DualSense report parsing, and app behavior. Those tests do not need a window,
GPU adapter, or controller.

## Limitations

- Rendering requires a working window-system backend and a GPU adapter supported
  by `wgpu`; windowed checks are manual.
- Xbox pads are read through `gilrs`: Windows Gaming Input on Windows and evdev
  on Linux. Unusual pads may need an SDL-style mapping that `gilrs` does not
  ship.
- On Windows, DualSense and DualSense Edge pads are read directly through the
  WinRT HID API, over USB or Bluetooth. Both report layouts are parsed; only the
  USB layout has been checked against hardware. This keeps Sony pads working when
  Windows Gaming Input reports no controllers at all, which happens when the
  `XBOX Synthetic Gamepad Bus` device fails to start, for example because a HID
  filter driver is attached above it. DualShock 4 pads are not covered by that
  fallback.
- Rumble, light bar, touchpad, and motion sensors are not implemented, and there
  is no rebinding or per-pad sensitivity.
- Controller stick direction is quantized to eight directions, though stick
  deflection does set analog walk speed with a radial dead zone.
- The HUD text path uses a built-in bitmap font only: there is no font loading,
  shaped text, anti-aliasing, or rich text layout.
- The scene uses untextured cubes only: there are no assets, materials,
  lighting, or shadows.
- The slice does not use `fate_net`; it has no networking, prediction, or
  rollback. Rollback is demonstrated by `projects/demo`.

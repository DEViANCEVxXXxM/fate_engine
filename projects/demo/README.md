# fate_demo

This is the standalone runnable demo project. Its manifest, windowed app,
scene setup, generated entities, performance display, and demo tests live here
and consume the engine from the repository root plus the reusable `fate_sim`
crate.

Run from the repository root:

```powershell
cargo run -p fate_demo
```

Or run `cargo run` from this directory.

The demo starts with a moving 2D swarm and off-screen entities. The camera
follows the primary entity marker so fixed-step movement, camera culling, and
frame telemetry can be observed together.

- **W/A/S/D** or **arrow keys** — move the primary entity.
- **Left Shift** — boost movement.
- **F2** — toggle the 192-visible-instance stress scene; off-screen entities
  remain culled.
- **F3** — switch to the 3D indexed-cube renderer test and back.
- In the 3D view, **W/A/S/D** or **arrow keys** move the camera, **mouse** looks
  around (the cursor is captured), and **Left Shift** moves faster.
- **Escape** in the 3D view releases the cursor and returns to the 2D demo.
- **Escape** in the 2D view closes the demo.

The 3D test scene contains a box arena, floor, walls, pillars, and colored
cubes. Camera movement has no collision and is not an application's character
controller or deterministic gameplay movement. The window title reports frame
timings, rolling averages/worst-case, draw pressure, budget headroom, and the
current policy tier.

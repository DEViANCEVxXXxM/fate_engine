# fate_debug

Opt-in native diagnostics tools for engine consumers. The core engine has no
dependency on this crate. Window creation is explicit, and CPU-only data,
statistics and export tests do not initialize a window or GPU.

## Run

```powershell
cargo run --release -p fate_demo -- --dbg
cargo run --release -p fate_window_smoke -- --mesh-load --dbg
```

The demo adds simulation/camera/entity tools. The smoke consumer uses the generic
renderer integration. Other projects can use the same API; `--dbg` is parsed by
the application, not silently read by the engine.

## Add to an existing or future project

Add a path dependency on `crates/fate_debug`, then keep a `DebugCompanion`
alongside your application state:

```rust,ignore
let debug = DebugCompanion::new(std::env::args().any(|arg| arg == "--dbg"));
```

In `ApplicationHandler::resumed`, create your renderer with optional timestamps,
attach it once, then open the companion:

```rust,ignore
let mut renderer = pollster::block_on(Renderer::new_with_options(
    window.clone(), self.debug.renderer_options(),
))?;
self.debug.attach_renderer("Main scene", &mut renderer);
pollster::block_on(self.debug.open(event_loop))?;
```

All existing `render`, `render_meshes`, and `render_scene` entry points then
publish telemetry automatically. No per-frame draw API change is necessary.
Attaching a renderer created without timestamps still supplies CPU telemetry.
Attach additional renderers with different names for a source selector;
reattaching the same name replaces its old subscription. Source changes clear
the displayed trace so measurements from different renderers are not combined.

Route events **before** your app handles close/resize/redraw:

```rust,ignore
if self.debug.on_window_event(window_id, &event) {
    return; // this was the companion window, not the scene
}
// Handle your own windows, checking their WindowIds.
```

In `about_to_wait`, drain telemetry and schedule the next diagnostics refresh:

```rust,ignore
self.debug.update(); // run regardless of window focus
let deadline = self.debug.next_deadline()
    .map_or(host_deadline, |debug_deadline| host_deadline.min(debug_deadline));
event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
```

The UI refreshes at most 20 Hz even when unfocused. For a host that otherwise
waits indefinitely, use `next_deadline()` to arrange wakeups. Keep calling the
host's own update/redraw path if you want simulation/rendering to continue in
the background. The toolkit never invents focus events, captures the host
cursor, or overrides application pause policy. A minimized companion skips
drawing but `update()` still collects telemetry. `open()` reopens a closed
companion without recreating the scene renderer.

`projects/window_smoke/src/main.rs` is a complete minimal integration.

## Optional project tools

Use `model_mut().inspection` to add facts, controls and entity rows. The generic
adapter owns the Overview and Renderer facts; applications own their other
pages. Pages with no project-specific data/tools are hidden.

```rust,ignore
let inspection = &mut self.debug.model_mut().inspection;
inspection.fact(Page::Simulation, "Tick", self.sim.tick());
inspection.controls.push(Control::action(
    Page::Simulation, "step", "Advance one tick",
));

for command in self.debug.take_commands() {
    match command.id {
        "step" => self.sim.step(),
        _ => {}
    }
}
```

Replace refreshed snapshots rather than appending the same facts/controls each
frame. Commands carry action, boolean or bounded-number values. The application
validates and executes them. `DebugWindow` and `DebugModel` are also available
separately for consumers supplying their own simulation/preparation/verification
timings, as the demo does.

To capture logs, wrap the application's logger with `capture_logs` and pass
the returned `LogInbox` to `set_logs`. Existing logger filtering is retained.
Input/focus/resize events are captured by the event-routing adapter.

## Measurement and resource limits

- Without a subscription the renderer does not collect this telemetry.
- The engine transport is nonblocking and bounded to 256 events per renderer.
  Drain regularly; dropped-event counts are surfaced in Events.
- CPU/GPU trace histories retain 1,200 samples each; events retain 500 entries.
  Event/entity lists are virtualized. The log inbox is bounded to 500 messages.
- Renderer-only integration measures render-call and command-recording time,
  not simulation or gameplay CPU work. Missing host phases are not displayed.
- Recording is a subset of the render call; do not sum both.
- GPU queries are optional and read asynchronously. Missing support does not
  prevent rendering. GPU IDs are **not** CPU frame IDs.
- The companion has a separate GPU device. Its pass work is excluded from scene
  timestamps, but it still shares system resources.
- Freeze holds traces while inspection and events stay live. Exports use unique
  names and report errors in the UI. Binary application snapshots are host tools.

## Checks

```powershell
cargo test -p fate_debug
cargo run --release -p fate_demo -- --dbg-smoke
cargo run --release -p fate_window_smoke -- --gpu-timing --dbg
cargo run --release -p fate_window_smoke -- --surface-lifecycle --dbg
```

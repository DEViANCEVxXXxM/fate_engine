# Engine Rules

This document defines the core rules for the Phase 1 engine milestone. These are engineering constraints, not gameplay design goals.

## 1. Deterministic fixed-step simulation

- The simulation tick is fixed at 120 Hz unless a later milestone explicitly changes it.
- The same world state and the same input sequence must produce the same result on repeated runs.
- Gameplay values must not rely on floating-point behavior for state that affects logic or rollback.
- The fixed-step loop must remain stable when the frame delta is large or when the app is briefly paused.

## 2. Frame and time rules

- `Time::tick()` must only advance wall-clock time.
- `next_fixed_step()` must consume fixed-step ticks from the accumulator.
- `alpha()` must stay in a valid range and must handle zero or invalid fixed-time values safely.
- A giant frame delta must be clamped instead of causing a large simulation jump.

## 3. ECS lifecycle

- Every entity is tracked by a stable entity id in the world.
- Despawn removes the entity from all relevant component columns.
- `alive` state and component state must remain consistent.
- Systems must operate on the world state only and must not reach into the app shell for gameplay data.

## 4. Input semantics

- `key_down` means the key is currently held.
- `key_just_pressed` means it transitioned to down this frame.
- `key_just_released` means it transitioned to up this frame.
- The same rules apply to actions built on top of the keyboard bindings.
- Edge-state must be cleared once per frame via `end_frame()`.

## 5. Rendering and camera rules

- The render subsystem is responsible only for drawing the world.
- The camera transform must remain stable across window resize and reconfiguration.
- Projection and camera matrices must remain finite even when the window is minimized, zero-sized, or temporarily reports a degenerate aspect ratio.
- Matrices must follow the project’s column-major convention.
- The app shell may request redraws, but it must not directly manipulate gameplay state in the render path.

## 6. App shell responsibilities

- The app owns the window, event loop, and top-level lifecycle.
- The app may request render work and fixed-step simulation, but it does not own gameplay rules directly.
- The event loop must not spin at an unbounded rate; redraw scheduling must respect the 120 FPS performance target.
- The app must expose a frame-budget signal so the engine can detect whether the current frame was within the target budget.
- Fixed-step and render work must be measured separately so frame pacing issues can be identified without guessing.
- The engine must keep a rolling performance history to estimate average and worst-case frame time, not just the most recent sample.
- The engine must emit an explicit warning when the combined fixed-step and render time exceeds the 120 FPS frame budget.
- If a renderer is missing, a frame must degrade gracefully without emitting undefined behavior.

## 7. Allowed unsafe use

- Unsafe is forbidden in the gameplay, ECS, and engine-core code unless there is a documented, isolated reason.
- Renderer backend code may use raw GPU APIs where required.
- Any unsafe block must be documented with the safety invariants it depends on.

## 8. Quality gate

The Phase 1 gate is considered complete only when the project passes:

- `cargo test`
- `cargo fmt`
- `cargo clippy --all-targets -- -D warnings`

These checks are the minimum engineering baseline for further engine work.

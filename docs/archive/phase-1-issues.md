> **Archived.** Phase 1 issue breakdown. Paths in this file predate the
> demo/engine split; most items are complete or superseded by
> [../roadmap.md](../roadmap.md).

1. [Core timing contract] Stabilize fixed timestep in `src/time.rs`
- Description:
  - Make the fixed-step timing contract explicit and safe.
  - Handle invalid fixed_hz, huge dt spikes, and accumulator edge cases.
- Tasks:
  - Clamp `fixed_hz` to a minimum safe value
  - Guard `alpha()` from invalid fixed delta
  - Keep `delta` clamped to max_delta
  - Add tests for zero/negative fixed_hz
  - Add tests for large frame delta
  - Add tests for accumulator transitions
- Acceptance criteria:
  - `Time::new(0.0)` does not break simulation
  - `alpha()` is always safe
  - `next_fixed_step()` behaves consistently at tick boundaries
  - All timing tests pass

2. [Core timing regression] Add deterministic simulation loop tests
- Description:
  - Validate that same input and same world state produce same fixed-update result.
- Tasks:
  - Add test for fixed-step loop across multiple frames
  - Add test for large dt being clamped
  - Add test for stable accumulator progression
- Acceptance criteria:
  - Repeated same scenario gives same world output
  - No drift from frame spikes
  - Test suite covers timing stability

3. [ECS lifecycle] Define entity lifecycle and validity rules in `src/ecs/world.rs`
- Description:
  - Make the ECS lifecycle explicit and safe for future systems.
- Tasks:
  - Document `spawn`, `despawn`, `is_alive`, `entity_count`
  - Ensure component columns are cleared on despawn
  - Ensure `alive` state and component state remain aligned
  - Add invalid entity protection if needed
- Acceptance criteria:
  - `despawn()` removes all component data for that entity
  - `entity_count()` reflects alive entities only
  - No orphaned component slots remain
  - Tests cover spawn/despawn lifecycle

4. [ECS tests] Add coverage for component columns and world state
- Description:
  - Verify column storage logic and entity lifecycle behavior.
- Tasks:
  - Add test for spawn + insert + remove
  - Add test for columns growing with entity ids
  - Add test for despawn cleaning component data
  - Add test for `is_alive` false after despawn
- Acceptance criteria:
  - ECS world behavior is stable under insertion/removal
  - no stale data remains after despawn

5. [System order] Make system execution order explicit in `src/ecs/system.rs`
- Description:
  - Lock deterministic execution order of world systems.
- Tasks:
  - Define order: input -> movement -> spin -> bounds
  - Ensure systems operate on world data only
  - Avoid hidden cross-system dependencies
  - Add tests for movement, spin and bounds
- Acceptance criteria:
  - Order is fixed and documented
  - No hidden state coupling between systems
  - Repeated simulation yields stable world state

6. [System regression] Add movement/bounds/spin tests
- Description:
  - Validate the first real simulation systems.
- Tasks:
  - Add test for movement integrates velocity
  - Add test for static entities do not move
  - Add test for bounds bounce at playfield edges
  - Add test for rotation wraparound / bounded angle
- Acceptance criteria:
  - Simulation is predictable in common edge cases
  - World state matches expected values after step updates

7. [Input contract] Formalize input semantics in `src/input.rs`
- Description:
  - Make input edge behavior predictable and testable.
- Tasks:
  - Clarify `key_down` vs `key_just_pressed`
  - Clarify `action_down` vs `action_just_pressed`
  - Ensure `end_frame()` clears edge transitions exactly once
  - Add tests for keyboard press/release cycle
  - Add tests for multi-key action binding
- Acceptance criteria:
  - Input transitions are consistent and reset per frame
  - Action binding behavior is correct
  - Keyboard edge tests pass

8. [Input regression] Add action mapping and transition tests
- Description:
  - Ensure action layer works with the current keyboard bindings.
- Tasks:
  - Test key press transitions
  - Test key release transitions
  - Test action mapping for MoveLeft/Right/Up/Down/Boost
  - Test repeated calls to `end_frame()`
- Acceptance criteria:
  - Input state matches expected action transitions
  - “just pressed” semantics are stable

9. [App loop separation] Reduce hardcoded demo logic in `src/app.rs`
- Description:
  - Move from “prototype app logic” toward engine loop pattern.
- Tasks:
  - Separate app shell from game setup
  - Keep fixed-update call site explicit
  - Keep renderer call site explicit
  - Keep title/logging updates independent
  - Guard against missing renderer/window state
- Acceptance criteria:
  - App loop is readable as engine lifecycle
  - game setup is not conflated with core engine behavior
  - no crash on resize/redraw without active renderer

10. [App lifecycle] Add basic loop safety tests or checks
- Description:
  - Validate app startup and redraw logic remains safe.
- Tasks:
  - Ensure no render before window created
  - Ensure no fixed update before world exists
  - Validate resize handler guard
  - Validate redraw loop still requests redraw after waiting
- Acceptance criteria:
  - App startup is stable
  - Window lifecycle is safe under normal event flow

11. [Math contract] Validate math utilities in `src/math`
- Description:
  - Make core math stable and edge-safe before deeper gameplay work.
- Tasks:
  - Check normalization for zero vector
  - Confirm `Vec2`/`Vec3` componentwise ops if needed
  - Validate `Mat4::mul()` and projection math
  - Verify camera transform is consistent with intended clip-space mapping
- Acceptance criteria:
  - Math functions are stable on common edge cases
  - `Mat4::orthographic_2d` and camera transform pass existing tests
  - No hidden instability in vector ops

12. [Math regression] Add direct unit tests for edge cases
- Description:
  - Prevent regressions in core math.
- Tasks:
  - Add zero-vector normalization
  - Add componentwise mul/div tests
  - Add matrix composition / identity tests
  - Add camera offset mapping tests
- Acceptance criteria:
  - Core math is covered by focused regression tests

13. [Project hygiene] Add minimum engineering gates
- Description:
  - Ensure early-stage changes don’t drift from quality baseline.
- Tasks:
  - Add `cargo test`
  - Add `cargo fmt`
  - Add `cargo clippy`
  - Document engine-level constraints in repo docs
- Acceptance criteria:
  - Core branch passes the standard engineering gate
  - no unformatted code
  - no key lint violations in core code

14. [Architecture note] Write the “core rules” doc for the repo
- Description:
  - Make the architecture explicit in project documentation.
- Tasks:
  - Document fixed-timestep contract
  - Document ECS lifecycle rules
  - Document deterministic gameplay precision rule
  - Document allowed unsafe usage
  - Document stage boundaries (prototype vs engine core)
- Acceptance criteria:
  - New contributors can understand the target rules without reading the entire design doc
  - engine constraints are explicit and enforceable

15. [Milestone gate] Close Phase 1 only when all issue criteria pass
- Description:
  - Formal checkpoint before moving to next milestone.
- Tasks:
  - Run full test suite
  - Validate all acceptance criteria
  - Update status / release notes
  - Confirm all core systems are stable for next stage
- Acceptance criteria:
  - Phase 1 is complete and stable
  - No open blocker in time/ECS/input/math subsystems
  - Ready to begin the next stage with clearer architecture

This is the cleanest issue breakdown for a Phase 1 gate. If you want, I can next turn this into:
- a GitHub-ready issue list with titles + labels,
- a shorter “sprint backlog” version,
- or a prioritized implementation order for the first 5 issues.
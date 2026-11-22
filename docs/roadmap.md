# Engine roadmap

> **Reviewed:** 2026-11-21. This is a working plan, not a release schedule or
> a stable API promise.

This page tracks open work and what would count as done. The numbered steps
in the [implementation history](implementation-history.md) record delivered
changes and benchmarks; completing a step does not establish release readiness.

## Current baseline

| Area | Verified today | Limit |
|---|---|---|
| Integration | The CPU smoke and offscreen capture example both ran as separate Cargo projects on Windows. | These are first-party path-dependency checks, not a packaged release or independent adoption. |
| Runtime | Windows renderer and diagnostics smokes passed. The workspace cross-compiles for Linux. | Native Linux runtime results are still open. |
| Determinism | Polygon checks fixed work counts and final state for physics, navigation, and ECS. The headless 3D contact smoke verifies scripted motion after restore. The showcase verifies recorded replays. | No timing threshold is part of the CPU gate. Replay, contact response, and lab policy stay consumer-owned. |
| Rendering | Windowed and offscreen rendering, resizable capture, grouped visibility, and full and sparse picking updates are exercised by consumers. | Windowed and offscreen paths use the same `wgpu` backend. |

## Active validation

### Step 27 — Establish a cross-platform validation baseline

**Status: open.** Ubuntu and Windows CI are configured to check the same
six CPU reference cases. Windows is the recorded reference; native Linux
results are pending.

1. On Linux, run the locked headless consumer smoke and Polygon’s
   `portable_cpu_baseline` test. Compare work counts and checksums, not
   timings. Record the Rust toolchain and target.
2. On a Linux desktop, follow the
   [manual smoke checklist](engine-guide.md#automated-quality-gate).
   Verify resize, minimize/restore, and the unfocused `--dbg` companion.
   Record the display backend and GPU adapter; name any backend left
   untested.

**Done when:** The seeded CPU cases agree on Windows and Linux, and the
Linux desktop smoke has a recorded result. The required CPU gate must not
require a window, GPU adapter, or audio device.

**Windows reference:** Run Polygon with `--headless --set seed=71 --warmup 16
--ticks 64` plus each row below. Both variants in each sweep matched on
`x86_64-pc-windows-gnu` with Rust 1.99.1. The headless consumer, renderer
surface lifecycle smoke, and Polygon’s 120-frame live smoke also passed.

| Preset | Additional options | Final checksum |
|---|---|---|
| `physics` | `--sweep broad_phase=0,1` | `1b4565dcb1275112` |
| `navigation` | `--set nav_goal_ticks=16 --sweep nav_mode=1,2` | `349b9e704332dc5b` |
| `ecs` | `--sweep snapshot_every=0,16` | `90faf31cc4d5c1f3` |

## Next engine milestone

The [contact milestone](implementation-history.md#step-32--verify-deterministic-3d-contact)
and its [face-normal follow-up](implementation-history.md#step-33--report-the-contacted-face-and-slide-along-it)
are complete. They established fixed-point segment-versus-box queries, not a
physics module or controller API.

Choose the next interaction in a consumer before adding another engine
primitive. A new spatial query should have a concrete use, a headless
boundary-case test, and a reproducible trace where state changes. The paused
gameplay projects stay out of scope, and the showcase remains a showcase.

## Later, when justified

- **Packaging and API stability:** choose a license before offering a
  release for external adoption. Verify a packaged consumer separately
  from an out-of-workspace path build.
- **Runtime expansion:** defer a general scheduler, arbitrary ECS components,
  and composed queries until a consumer or measured CPU workload needs them.
- **Renderer expansion:** defer general assets, materials, lighting, scene
  graphs, and extra backends. Keep lazy tile-buffer growth deferred until
  profiling or a concrete memory budget warrants it.
- **Networking:** the current local protocol does not open sockets.
  Production transport, peer identity, clock sync, and reconciliation need
  separate scope and a threat model.

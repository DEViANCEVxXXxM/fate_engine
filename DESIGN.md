---
name: Fate Diagnostics
description: Compact native diagnostics in forest-green surfaces with measured traces.
colors:
  canvas: "#0b1615"
  panel: "#101f1d"
  rail: "#0d1b1a"
  line: "#2b4039"
  text: "#e5eee3"
  muted: "#9cb2a5"
  lime: "#c0e188"
  teal: "#63c8b8"
  amber: "#e8bb70"
  error: "#f4877e"
  selection: "#334c2c"
  control-inactive: "#1b2f28"
  control-hover: "#2b432f"
  control-hover-weak: "#21372c"
  control-active: "#38502f"
  native-caption: "#0b1a15"
typography:
  heading:
    fontFamily: "egui proportional"
    fontSize: "24pt"
  section:
    fontFamily: "egui proportional"
    fontSize: "16pt"
  body:
    fontFamily: "egui proportional"
    fontSize: "14pt"
  button:
    fontFamily: "egui proportional"
    fontSize: "14pt"
  small:
    fontFamily: "egui proportional"
    fontSize: "12pt"
  mono:
    fontFamily: "egui monospace"
    fontSize: "13pt"
  chart-axis:
    fontFamily: "egui monospace"
    fontSize: "10pt"
  chart-direction:
    fontFamily: "egui proportional"
    fontSize: "11pt"
rounded:
  panel: "7pt"
  mark: "5pt"
  phase-bar: "2pt"
spacing:
  item-x: "10pt"
  item-y: "6pt"
  button-x: "12pt"
  button-y: "8pt"
  panel-inset: "14pt"
  rail-inset: "16pt"
  content-inset: "24pt"
  section-gap: "18pt"
  summary-gap: "12pt"
components:
  panel:
    backgroundColor: "{colors.panel}"
    rounded: "{rounded.panel}"
    padding: "{spacing.panel-inset}"
  button:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.text}"
    typography: "{typography.button}"
    padding: "8pt 12pt"
  button-hover:
    backgroundColor: "{colors.control-hover-weak}"
  navigation:
    textColor: "{colors.muted}"
    height: "38pt"
  navigation-selected:
    backgroundColor: "{colors.selection}"
    textColor: "{colors.lime}"
    height: "38pt"
  input:
    backgroundColor: "{colors.canvas}"
    textColor: "{colors.text}"
  session:
    backgroundColor: "{colors.rail}"
    height: "34pt"
    padding: "8pt"
---

# Design System: Fate Diagnostics

## Overview

**Creative North Star: "User-pinned dark green dashboard"**

A compact, workhorse inspection companion: forest-black canvas, green-tinted panels, fine borders and lime selection. Proportional labels establish hierarchy; monospace inspection values make precise readings easy to compare. Charts and real controls carry the identity rather than imagery.

This records the implemented native Rust egui/winit/wgpu UI in `crates/fate_debug/src/ui.rs` and `window.rs`. The reusable companion belongs to the consumer-side toolkit: applications supply inspection data and execute returned commands; core engine crates do not own this presentation. Launch the showcase companion with `cargo run -- --dbg`.

**Key Characteristics:**
- Compact persistent navigation and scrollable inspection content.
- Tonal green layering with restrained, semantic color.
- Measured, bounded traces with explicit live and frozen states.
- Native window behavior and code-drawn graphics.

## Colors

### Primary

`lime` identifies selected navigation, live status, visible entities, successful exports and frame-interval or GPU-scene traces. `selection` supplies the darker selected-control fill.

### Secondary

`teal` distinguishes CPU work, phase bars and informational events. `amber` marks the editable frame budget, frozen status and warnings. `error` marks errors and export failures. Labels accompany these colors; color alone does not identify state.

### Neutral

`canvas` grounds the main workspace and text fields; `panel` raises content tonally; `rail` unifies navigation and session status. `line` defines fine boundaries and chart gridlines. `text` carries primary content; `muted` carries secondary labels and chart annotations.

The theme distinguishes strong widget fills (`control-inactive`, `control-hover`, `control-active`) from weak fills (`panel` at rest, `control-hover-weak` on hover). Preserve that egui distinction instead of applying the strong fill indiscriminately to every button. Selection and hovered/active widget strokes use lime.

## Typography

Use egui's bundled proportional and monospace families, not a browser font stack or an invented display face. Token `pt` values mean egui logical points, not CSS print points. The host scale factor maps them to physical pixels. Unspecified font metrics, weights and widget styling remain egui defaults.

Page headings and summary values use the heading size; summary values are proportional, not monospace. Section headings use the section size with `RichText::strong()`. Body and buttons share a size, supporting dense scanning. Secondary explanations use small text. Facts, positions, timestamps and phase durations use mono. Chart ticks use chart-axis; directional captions use chart-direction. The implementation also uses a 23-point strong brand label and a 28-point GPU reading; these are local treatments rather than a general display scale.

## Layout

The initial client area is 1160 × 800 logical units, with a 720 × 520 minimum. The native titlebar sits outside this client area. A fixed 185-point left panel has the rail inset; the central panel has the content inset. A 34-point session panel spans the bottom. The sidebar is not collapsible. Its navigation buttons fill the available rail width.

The page title, live/frozen status and trace action stay above a vertically scrolling page body. Summary cards, chart and lower inspection groups follow the compact spacing tokens. Panel borders are one point. Inspection grids use 16 × 8-point spacing and wrapping values; their maximum column width is `max(available_width * 0.57, 140)`.

Responsive decisions use **available content width**, not physical window width:
- Below 720 points, four overview summaries become two columns.
- Below 650 points, CPU phases and runtime facts stack; otherwise they share two columns.
- At 760 points or more, inspection and tools sit side by side when controls exist. Below this threshold, tools precede inspection vertically.

Chart controls, event controls and export actions wrap. Entity filters use a horizontal row; the entity table retains five columns with a 65-point minimum cell width. Entity and event rows are virtualized at 28 points. Do not infer a mobile navigation pattern from this desktop layout.

The native window title is `Fate / Diagnostics`. winit requests a dark theme; Windows DWM requests `native-caption`, `text` and `line` for caption, caption text and border. Standard minimize, maximize, close, resize and OS focus behavior remain native. Cosmetic DWM requests may be rejected on older Windows; there is no custom-painted titlebar fallback. Closing the companion does not close its host.

## Elevation & Depth

Main surfaces are flat: inset green fills and fine borders establish separation, without custom panel shadows. egui retains its default transient-widget behavior; this is not a blanket prohibition on tooltip or popup shadows. No bespoke animation timing is defined. Telemetry updates communicate live activity.

## Shapes

Panels use the panel radius and one-point line borders. The brand mark is a 28-point square with the mark radius and three code-drawn strokes. Phase bars are four points high with the phase-bar radius. Ordinary buttons and inputs inherit egui corner treatment; the panel radius must not be presented as their explicit radius.

## Components

### Navigation and shell

Navigation is text-led, full-width and compact. Selected entries use lime text and selection fill; unselected labels are muted. Pages are Overview, Renderer, Simulation, Camera, Entities, Events and Capture. Simulation and Camera appear only when the consumer supplies their facts or controls; Entities appears only when candidates exist. Switching pages clears the text filter. Source selection is a 145-point combo only when multiple sources exist; otherwise the source is plain text.

### Buttons, fields and tools

Ordinary buttons use weak widget fills, the shared padding and fine borders; the theme supplies lime hovered/active strokes. Global interaction height is at least 28 points, with denser read-only inspection rows. Keyboard focus and disabled treatment remain egui-managed; no bespoke focus animation or disabled palette is defined.

Text fields use the canvas fill. Entity and event filters request widths of 220 and 250 points; the export directory fills its container. Host tools map to actions, toggles and bounded draggable numeric fields. Numeric controls take range and speed from the consumer and show up to three decimals. Disabled host controls remain disabled. Commands are returned to the consumer for execution.

### Summary cards and facts

The four summaries show cadence, CPU work, GPU scene and draw submission. Cadence is derived from mean positive frame interval and includes p95 detail. CPU and GPU cards show p50; GPU availability and completed-sample count remain explicit. Missing data reads `Waiting`, `No sample`, or a timestamp-query availability explanation rather than invented readings.

Inspection uses striped key/value rows, muted labels and wrapping monospace values. CPU phase bars represent each latest-frame phase's share of total CPU work. Recording time is stated as included in submit; it is never added twice. Without host timings, the phase breakdown retains only submit/present.

### Trace charts

**The Measurement Rule.** Frame interval, CPU work and GPU pass duration are different measurements; never merge their meanings.

Frame history is 175 points high; GPU history is 180. Plot insets are left 48, top 10, right 8 and bottom 20 points. Five horizontal gridlines carry a shared millisecond axis. The upper bound is 1.15 times the greater of finite samples, optional budget and 0.01. Series strokes are 1.6 points; grid, budget and hover cursor strokes are one point. Samples advance oldest to newest, uniformly by sample index rather than elapsed-time spacing.

Frame interval is lime, CPU work teal and the budget marker amber. Checkboxes independently hide the frame series. Retained-window selectors offer 120, 300 (default), or 1,200 frames. The budget marker is draggable from 1 to 1,000 ms at speed 0.25. Hover selects the nearest sample index, shows exact three-decimal durations, and adds frame/tick/instance detail below frame history. Empty charts say `Waiting for samples`; a line needs at least two finite points.

GPU history shows completed asynchronous scene-pass samples in order. It excludes acquire, present and gaps and is not aligned to CPU frames. Unavailable timestamps and pending readbacks have distinct text states.

### Live, frozen and capture states

**The Trace Rule.** Freeze holds both retained streams; inspection and events remain live.

The header pairs lime `LIVE` with `Freeze trace`, or amber `FROZEN` with `Resume trace`. This is distinct from a consumer's simulation pause command. Each stream is bounded to 1,200 samples and events to 500; oldest entries are evicted.

Capture supports clear trace, copy report, CPU/GPU CSV export and text report export, plus consumer-supplied capture tools. CSV buttons are disabled when their corresponding stream is empty. The default export directory is `diagnostics-captures`. Success displays a lime saved path; failure displays red actionable error text and permits retry. Both outcomes enter event history.

### Entities and events

Entity filtering is case-insensitive by ID with an optional visible-only constraint. Selection exposes precise position and a working copy action. Frustum state reads `Visible` in lime or `Culled` in muted text. Empty results explain that no entities match.

Events are newest first, with monospace elapsed time, labeled severity and truncation plus full-message hover text. Filters support message search and warnings/errors only; clearing events is a real action. Empty results are stated explicitly.

## Do's and Don'ts

### Do:
- **Do** retain the compact green shell, semantic trace colors and native window controls.
- **Do** distinguish measured streams and label unavailable or waiting data honestly.
- **Do** keep presentation consumer-owned and execute host tools through returned commands.
- **Do** preserve available-width stacking and vertical scrolling.

### Don't:
- **Don't** fabricate telemetry or expose inactive placeholder tools.
- **Don't** equate freezing a trace with pausing the host.
- **Don't** replace the native UI with HTML/CSS or generated raster decoration.
- **Don't** turn incidental egui defaults into explicit project-wide tokens.

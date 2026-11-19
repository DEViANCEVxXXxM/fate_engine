# Fate engine tools

<!-- impeccable:product-schema 1 -->

## Platform

Native desktop (Rust, winit and wgpu).

## Users

Engine developers inspecting the live capability showcase.

## Product Purpose

A separate diagnostics companion launched with `cargo run -- --dbg`.
Expose measured statistics and working engine tools while the showcase runs.

## Capabilities and Constraints

The companion is a reusable consumer-side toolkit. Applications supply data
and execute commands. Core engine crates do not own diagnostics presentation.
The showcase remains a capability demonstration. Required CPU tests do not
need a display or GPU. The companion must not display fabricated measurements
or inactive placeholder tools.

## Brand Commitments

Follow the supplied visual reference: dark green surfaces, compact left
navigation, fine panel borders, lime selection and readable time-series charts.

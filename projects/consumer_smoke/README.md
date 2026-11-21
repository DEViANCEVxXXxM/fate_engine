# fate_consumer_smoke

This tiny headless package validates that a separate workspace consumer can
depend on the documented `fate_engine` and `fate_sim` APIs without importing
or starting the demo, window system, or renderer.

Run from the repository root:

```powershell
cargo run -p fate_consumer_smoke
cargo check -p fate_consumer_smoke
```

Its dependencies are local path dependencies declared in this project's
`Cargo.toml`, just as an in-repository application project would use them. For an
external project, adjust those paths relative to its own manifest.

## Fixed-point 3D contact smoke

```sh
cargo run --release --locked -p fate_consumer_smoke --example contact_3d
```

A scripted point actor takes nine fixed ticks around two static boxes.
It rejects blocked movement commands, including ones whose endpoints are
both outside a box. The fixture checks four contacts, a fixed final checksum,
and identical results after restoring from every intermediate checkpoint.
It is headless, and its response policy is not part of `fate_sim`.

## Verify outside the workspace

From the engine checkout, run this PowerShell recipe to build a smoke
consumer with its own manifest and lockfile. Set `$render = $true` for the
offscreen renderer example instead of the CPU-only input and simulation test.
Use a new scratch directory for each run.

```powershell
$render = $false
$engine = (Get-Location).Path.Replace('\', '/')
$consumer = Join-Path ([IO.Path]::GetTempPath()) "fate-external-smoke-$([guid]::NewGuid())"
New-Item -ItemType Directory (Join-Path $consumer 'src') | Out-Null
if ($render) {
    $main = 'examples/offscreen_capture.rs'
    $dependency = 'pollster = "1.0.1"'
} else {
    $main = 'projects/consumer_smoke/src/main.rs'
    $dependency = "fate_sim = { path = `"$engine/crates/fate_sim`" }"
}
Copy-Item $main (Join-Path $consumer 'src/main.rs')
Copy-Item Cargo.lock (Join-Path $consumer 'Cargo.lock')
@"
[package]
name = "fate_external_smoke"
version = "0.1.0"
edition = "2024"

[dependencies]
fate_engine = { path = "$engine" }
$dependency
"@ | Set-Content (Join-Path $consumer 'Cargo.toml')
Push-Location $consumer
try {
    cargo locate-project --workspace
    cargo check
    if ($LASTEXITCODE -ne 0) { throw 'Standalone build failed' }
    if ($render) { cargo run --locked -- wide.ppm tall.ppm }
    else { cargo run --locked }
    if ($LASTEXITCODE -ne 0) { throw 'Standalone smoke failed' }
} finally {
    Pop-Location
}
```

The located manifest should be the scratch project's own `Cargo.toml`. The
first check adapts the copied lockfile to the new project.

- **Simulation:** expect a pass at tick 2, without a window, GPU, or audio.
- **Rendering:** needs a GPU adapter but no window or display connection.
  Expect `wide.ppm` (641×360) and `tall.ppm` (360×641), each reporting two
  instances in one draw. Existing files are not overwritten.

These runs verify path dependencies outside the workspace, not a packaged
release or a second renderer backend.

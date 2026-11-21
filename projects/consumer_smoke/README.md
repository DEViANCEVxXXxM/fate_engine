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

## Verify outside the workspace

From the engine checkout, run this PowerShell example to build and run the
same smoke code as its own Cargo project:

```powershell
$engine = (Get-Location).Path.Replace('\', '/')
$consumer = Join-Path ([IO.Path]::GetTempPath()) "fate-external-smoke-$([guid]::NewGuid())"
New-Item -ItemType Directory (Join-Path $consumer 'src') | Out-Null
Copy-Item projects/consumer_smoke/src/main.rs (Join-Path $consumer 'src/main.rs')
Copy-Item Cargo.lock (Join-Path $consumer 'Cargo.lock')
@"
[package]
name = "fate_external_smoke"
version = "0.1.0"
edition = "2024"

[dependencies]
fate_engine = { path = "$engine" }
fate_sim = { path = "$engine/crates/fate_sim" }
"@ | Set-Content (Join-Path $consumer 'Cargo.toml')
Push-Location $consumer
try {
    cargo locate-project --workspace
    cargo check
    if ($LASTEXITCODE -ne 0) { throw 'Standalone build failed' }
    cargo run --locked
} finally {
    Pop-Location
}
```

The located manifest should be the scratch project's own `Cargo.toml`, and
the program should report a pass at simulation tick 2. The first check adapts
the copied lockfile to the new project. This verifies path dependencies without
relying on engine workspace membership; it does not test a packaged release.

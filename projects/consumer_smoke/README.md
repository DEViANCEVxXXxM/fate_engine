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
`Cargo.toml`, just as an in-repository game project would use them. For an
external project, adjust those paths relative to its own manifest.

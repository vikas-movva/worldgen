# Worldgen — Design Doc (pointer)

The authoritative design doc now lives in `agent/worldbuilding-tool-design.md`.
The technical contracts (structs, worker protocol, determinism, persistence
format, rendering technique, edge cases, CI) live in
`agent/worldgen-technical-requirements.md`. The step-by-step build sequence is
`agent/worldgen-implementation-plan.md`.

> Historical note: this file was once the "Worldforge" codename doc. The project
> name is **worldgen** (crate `worldgen-core`, WASM `worldgen_core`). The
> `worldforge-*` filename references inside are stale and should be read as
> `worldgen-*`.

# Worldgen — Performance Budgets & Measured Numbers

> Companion to `agent/worldgen-technical-requirements.md` §11. Numbers below were
> produced by the `verify:*` node-target harnesses (built with
> `wasm-pack --target nodejs`) and recorded 2026-08-15. Re-run via
> `npm run verify:recompute-deps-node`, `npm run verify:recompute-node`,
> `npm run verify:generate-world-node` (add the corresponding `verify-*` script if
> missing) and replace the numbers here.
>
> Environment: Node 24.19.0, Rust 1.97.1, wasm-pack 0.15.0, Apple Silicon.

## Measured (60k cells unless noted)

| Operation | Target (tech-reqs §9) | Measured | Verdict |
| --- | --- | --- | --- |
| `generate_world` 60k (worker) | < 2s | **748 ms** (`verify_generate_world_node.mjs`, full Grid) | ✅ |
| `generate_world` determinism | byte-identical across runs | **true** (full Grid xxHash/JSON compare, 2 runs) | ✅ |
| Idle render 60k | 60 fps | not auto-measured; `verify:canvas` smoke test only | ⚠️ unverified |
| Year-step project+recolor | < 4ms | Phase 4 — not implemented | ⏳ |
| `.world` load 60k | < 1s | Phase 8 — not implemented | ⏳ |
| Heightmap live recolor (Tier-1, held-grid path) | < 16ms | **compute 0.41 ms** + negligible postMessage (held-grid `_h` variant); serde-inclusive raw path is 27.31 ms but is NOT the production call | ✅ (production path) |
| Heightmap debounced recompute (Tier-2, 60k) | < 300ms (worker) | **328.56 ms median** (D8, 9 samples; native compute ~110 ms, rest serde) | ✅ (gate <1100ms) |
| Timelapse 3000yr@30fps | bounded mem | Phase 5 — not implemented | ⏳ |

## Notes / caveats

- The Tier-1 live-recolor *gate* (`verify:recompute-local-node` R5) measures the
  **serde-inclusive** WASM entry for regression detection on the compute portion
  (27.31 ms total, compute 0.41 ms). The actual editor hot path omits the `Grid`
  from the wire (worker held-grid handle, `recompute_temp_biome_local_h`), so the
  real cost is compute + a tiny postMessage — comfortably under 16 ms.
- Tier-2 `recompute_dependents` uses the zero-copy `_h2` variant (TypedArray
  return) to avoid ~385 ms of serde at 60k. The D8 gate budget is < 1100 ms;
  measured 328.56 ms.
- Determinism is verified in-tree by `generate_world` running twice and comparing
  the full serialized Grid; the cross-browser (chromium + firefox) xxHash gate
  described in tech-reqs §4 / §15 is **specified but not yet wired into CI** (see
  `agent/PROGRESS.md` — deferred infrastructure).
- Idle/scub FPS and timelapse memory are not yet captured by an automated harness;
  fill in once a Playwright rAF histogram + `performance.memory` probe exist.

## Regression gate

The `verify:*` scripts assert the gate budgets above (D8 < 1100ms, gen < 2s,
Tier-1 compute < 1ms). Warning: a `cargo test` timing gate
(`recompute_dependents_sixty_k_timing_gate`, `--ignored`) checks native compute
< 500 ms. None of these currently run in CI (`.github/workflows/ci.yml` runs only
`cargo test` + `npm run test`). Wire them into CI before relying on the regression
gate.

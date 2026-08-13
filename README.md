# Worldgen (codename: Worldforge)

A **local-first worldbuilding tool** — an FMG-style (Azgaar's Fantasy Map
Generator) procedural map generator **geared to worldbuilding**, running
100% locally and staying smooth at **≤60k cells / 60fps**.

The differentiator from FMG: a **deterministic Rust → WASM compute core** that
runs off-thread, a **GPU (PixiJS/WebGL2) renderer** instead of SVG DOM nodes,
and an **event-sourced timeline** that animates a time-varying world with
procedurally generated history. Terrain is author-editable; rivers, lakes,
coastline, temperature, precipitation, and biomes recompute from it.

> Everything runs on your machine. There is no server, and the optional LLM
> polish is strictly opt-in (you bring your own provider/key).

---

## Features (current status)

This is an active, early-stage build (implementation is tracked step-by-step in
[]()). Landed so far:

- **Deterministic Rust → WASM core** (`core/`) — seeded, byte-identical output
  across runs for a given seed.
  - Voronoi mesh generation (`mesh.rs`)
  - Heightmap generation with user authoring (`heightmap.rs`, `heightmap_edit.rs`)
  - Climate: temperature + precipitation (`climate.rs`)
  - Biome classification (`biomes.rs`)
  - Rivers (`rivers.rs`)
  - Heightmap editing with **dependent recompute** (temperature/biome
    divergence reporting)
- **Web Worker bridge** (`app/src/workers/core.worker.ts`) — core runs off the
  main thread; UI stays responsive during generation.
- **PixiJS v8 renderer** (`app/src/render/`) — merged geometry per layer,
  viewport culling, layer toggles, pan/zoom at 60fps/60k cells.
- **React + TypeScript UI** (`app/src/`) with Zustand state stores.
- **Node-side verification scripts** for determinism, climate, biomes, and
  heightmap-edit recompute.

Planned (see design doc): event engine / timeline scrubber, TipTap entity
chronicles, IndexedDB/OPFS persistence (`.world` files), optional Tauri native
backend.

---

## Tech stack

| Layer | Technology |
| --- | --- |
| Compute / simulation core | **Rust → WASM** (`wasm-pack`, `wasm-bindgen`), `spade` for triangulation |
| Renderer | **PixiJS v8** (WebGL2) |
| UI | **React + TypeScript** |
| Rich-text history | **TipTap** + `tiptap-markdown` (planned) |
| Build / tooling | **Vite** + `vite-plugin-wasm` + Biome |
| Persistence | **IndexedDB + OPFS** (planned) |

---

## Repository layout

```
worldgen/
├── core/          # Rust → WASM compute core (cargo crate: worldgen-core)
│   └── src/       # mesh, heightmap, heightmap_edit, grid, climate, biomes, rivers
├── app/           # React + Vite + TypeScript front-end
│   ├── src/core/  # committed wasm artifacts (worldgen_core_bg.wasm + JS glue)
│   ├── src/render/# PixiJS renderer (geometry, layers, palette, MapCanvas)
│   ├── src/state/ # Zustand stores
│   ├── src/workers/# Web Worker bridge
│   └── scripts/   # node-side verification harnesses
└──           # design doc, implementation plan, progress, FMG research
```

> **Note:** the compiled WASM artifact (`app/src/core/worldgen_core_bg.wasm`)
> is **committed** so the app builds without a Rust toolchain. Regenerate it
> with `npm run build:core` after changing the core.

---

## Prerequisites

- **Rust** ≥ 1.97 (edition 2021) with the `wasm32-unknown-unknown` target
  (`rustup target add wasm32-unknown-unknown`)
- **[wasm-pack](https://rustwasm.github.io/wasm-pack/)** ≥ 0.13
- **Node.js** ≥ 20 (developed on v24) and npm

---

## Getting started

```bash
# 1. Install front-end dependencies
cd app
npm install

# 2. (Optional) Rebuild the WASM core from Rust — requires Rust + wasm-pack
npm run build:core

# 3. Run the dev server
npm run dev

# 4. Build for production
npm run build
```

The app is served by Vite; open the printed local URL.

---

## Testing & verification

```bash
# Rust core unit tests (native, fast)
npm run test:core
#   → cargo test --manifest-path ../core/Cargo.toml

# Front-end unit tests (Vitest, jsdom)
npm run test
#   → vitest run

# Everything
npm run test:all
```

Node-side verification harnesses (determinism, climate, biomes, heightmap-edit
recompute) live in `app/scripts/` and are wired as `npm run verify:*` scripts:

```bash
npm run verify:recompute-deps-node   # heightmap edit → dependent recompute
npm run verify:recompute-local-node  # local temperature/biome recompute
npm run verify:edit-node             # heightmap edit bridge
npm run verify:canvas                # PixiJS canvas smoke test
```

CI (`.github/workflows/ci.yml`) runs `cargo test` and the Vitest suite on every
push/PR.

---

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). Key rules: keep the four-layer
separation (core never imports UI; renderer never mutates state; state has no
render code), preserve determinism (seeded RNG only), and never commit secrets
or `.env` files.

---

## Documentation

- [``]() — the
  design doc (architecture, data model, scope decisions).
- [``]()
  — step-by-step, agent-ready implementation plan with verification gates.
- [``]()
  — technical requirements.
- [``]() — current step status.
- [``]() — source study of
  Azgaar's FMG that informed the design.

---

## License

[MIT](LICENSE) © Vikas Movva

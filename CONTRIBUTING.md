# Contributing

Thanks for taking a look at Worldgen. This is an early-stage, local-first
worldbuilding tool. Contributions are welcome, but please read the rules below
so the build stays deterministic and the architecture stays clean.

## Architecture rules (non-negotiable)

The project enforces a strict four-layer separation:

- **Core (`core/`) never imports UI.** The Rust → WASM core is pure compute
  (mesh, heightmap, climate, biomes, rivers, event engine). It has no knowledge
  of React, PixiJS, or the DOM.
- **Renderer never mutates state.** `app/src/render/` only reads world state
  and draws it.
- **State has no render code.** `app/src/state/` stores hold world data; they
  don't touch the canvas.

## Determinism

Output must be **byte-identical** for a given seed. Rules:

- Use the seeded RNG only (`StdRng::seed_from_u64`). Never `Math.random`,
  `Date.now`, or any other non-deterministic source in generation paths.
- Don't reorder parallel passes in a way that changes results.

## Commits

Follow the existing convention used throughout the history:

```
step N.M: <short summary in present tense>
```

e.g. `step 2.5.2: Tier-1 local temperature + biome recompute`. Reference the
implementation plan step when applicable.

## Before you open a PR

```bash
npm run test:core     # Rust unit tests
npm run test          # Vitest suite
npm run lint          # Biome
```

CI runs `cargo test` and the Vitest suite on every push/PR. Make sure both
pass locally first.

## Rebuilding the WASM core

If you change anything under `core/src/`, regenerate the committed WASM
artifact:

```bash
cd app && npm run build:core
```

Commit the regenerated `app/src/core/worldgen_core_bg.wasm` (and its JS glue)
so the app builds without a Rust toolchain.

## Secrets

This project is 100% local. **Never** commit `.env` files, API keys, or
credentials. The optional LLM integration is opt-in and uses the user's own
provider/key at runtime — nothing is stored in the repo.

## Docs

Keep progress notes in sync when you complete or start a step. Design changes
belong in the design documentation.

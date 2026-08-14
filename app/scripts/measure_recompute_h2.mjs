// Measure Track B: recompute_dependents_h2 (TypedArray return) vs
// recompute_dependents_h (serde return) vs original recompute_dependents
// (full serde round-trip).

import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const wasm = require("/tmp/world_node/worldgen_core.js");
await wasm.init();

const { generate_world, store_grid_h, recompute_dependents_h, recompute_dependents_h2 } = wasm;

const N = 60000;
const SEED = 42;

function median(arr) {
  return arr.slice().sort((a, b) => a - b)[Math.floor(arr.length / 2)];
}

async function main() {
  const grid = generate_world(SEED, N, {});
  store_grid_h(grid);
  console.log(`Generated ${grid.cells.h.length}-cell world, stored grid in Rust handle\n`);

  // Warm up all paths.
  recompute_dependents_h({});
  recompute_dependents_h2({});

  // Measure _h (serde return).
  const hTimes = [];
  for (let i = 0; i < 9; i++) {
    const t0 = performance.now();
    recompute_dependents_h({});
    hTimes.push(performance.now() - t0);
  }

  // Measure _h2 (TypedArray return).
  const h2Times = [];
  for (let i = 0; i < 9; i++) {
    const t0 = performance.now();
    const result = recompute_dependents_h2({});
    h2Times.push(performance.now() - t0);
  }

  // Verify _h2 returns TypedArrays.
  const r2 = recompute_dependents_h2({});
  const typedFields = ["temp", "prec", "biome", "state", "province", "culture", "religion", "burg", "fl", "r", "conf", "coastline"];
  const allTyped = typedFields.every((f) => ArrayBuffer.isView(r2[f]));
  const lengths = typedFields.map((f) => r2[f].length);
  const allCorrectLen = lengths.every((l) => l === N);

  console.log(`recompute_dependents_h  (serde return):     ${median(hTimes).toFixed(2)}ms median`);
  console.log(`recompute_dependents_h2 (TypedArray return): ${median(h2Times).toFixed(2)}ms median`);
  console.log(`\nVerification:`);
  console.log(`  All 12 arrays are TypedArrays: ${allTyped}`);
  console.log(`  All 12 arrays have length ${N}:  ${allCorrectLen}`);
  console.log(`  temp is Int8Array:    ${r2.temp instanceof Int8Array}`);
  console.log(`  state is Int32Array:  ${r2.state instanceof Int32Array}`);
  console.log(`  fl is Uint16Array:    ${r2.fl instanceof Uint16Array}`);
  console.log(`  coastline is Uint8Array: ${r2.coastline instanceof Uint8Array}`);
  console.log(`  rivers is Array:     ${Array.isArray(r2.rivers)}`);
  console.log(`  lakes is Array:      ${Array.isArray(r2.lakes)}`);

  const speedup = median(hTimes) / median(h2Times);
  console.log(`\nSpeedup: ${speedup.toFixed(1)}x (serde return -> TypedArray return)`);
  console.log(`Saved: ${(median(hTimes) - median(h2Times)).toFixed(2)}ms per recompute_dependents call`);
}

main().catch((err) => { console.error(err); process.exit(1); });

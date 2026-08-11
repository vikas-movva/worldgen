// Real end-to-end climate verification through the WASM boundary (node target).
// Mirrors the heightmap verification recipe from the skill's
// references/heightmap-generation.md §6. Exercises generate_mesh →
// generate_heightmap → generate_climate (the node-target WASM path, no browser).
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const wasm = require("/tmp/climate_node/worldgen_core.js");
// Node-target wasm-pack runs its init (instantiates the WASM) as a side effect
// of the import for the `nodejs` target, so no `.default()` call is needed.

const N = 8000;
const seed = 42 >>> 0;

const mesh = wasm.generate_mesh(N, seed);
const h = wasm.generate_heightmap(mesh, seed);

// Check heightmap shape/ranges.
let land = 0;
for (let i = 0; i < h.length; i++) if (h[i] >= 20) land++;
console.log(`heightmap: len=${h.length} landFrac=${(land / h.length).toFixed(3)}`);

// Climate.
const climate = wasm.generate_climate(mesh, h, {});
const T = climate.temp;
const P = climate.prec;
let minT = 127, maxT = -128, minP = 255, maxP = 0, tempOOB = 0, precOOB = 0;
for (let i = 0; i < T.length; i++) {
  if (T[i] < -128 || T[i] > 127) tempOOB++;
  minT = Math.min(minT, T[i]); maxT = Math.max(maxT, T[i]);
}
for (let i = 0; i < P.length; i++) {
  if (P[i] < 0 || P[i] > 255) precOOB++;
  minP = Math.min(minP, P[i]); maxP = Math.max(maxP, P[i]);
}
console.log(`temp: len=${T.length} range=[${minT},${maxT}] OOB=${tempOOB}`);
console.log(`prec: len=${P.length} range=[${minP},${maxP}] OOB=${precOOB}`);

// Latitude structure: equatorial sea-level cells warmer than polar.
const pts = mesh.points;
const H = mesh.world_h;
function bandTemp(frac, poles=false) {
  let s = 0, n = 0;
  for (let i = 0; i < pts.length; i++) {
    if (h[i] >= 20) continue;
    const rel = pts[i][1] / H;
    const inBand = poles ? (rel < frac || rel > 1 - frac) : (Math.abs(rel - 0.5) < frac / 2);
    if (inBand) { s += T[i]; n++; }
  }
  return n ? s / n : 0;
}
const eqT = bandTemp(0.5);
const poleT = bandTemp(0.1, true);
console.log(`equatorialMeanT=${eqT.toFixed(2)} polarMeanT=${poleT.toFixed(2)}`);

// Determinism: identical inputs → identical bytes.
const c2 = wasm.generate_climate(mesh, h, {});
let identical = true;
for (let i = 0; i < T.length; i++) {
  if (T[i] !== c2.temp[i] || P[i] !== c2.prec[i]) { identical = false; break; }
}

// Verdict.
const rangesOk = T.length === N && P.length === N && tempOOB === 0 && precOOB === 0;
const structureOk = eqT > poleT && (maxT - minT) >= 10 && (maxP - minP) >= 10;
const PASS = rangesOk && structureOk && identical;
console.log(`DETERMINISTIC=${identical}`);
console.log(`VERDICT: ${PASS ? "PASS ✅" : "FAIL ❌"}`);
process.exit(PASS ? 0 : 1);

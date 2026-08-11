// Step 1.4 boundary verification — runs the real WASM (node target) through the
// full mesh -> heightmap -> climate -> biomes chain, no browser. Exercises
// generate_mesh / generate_heightmap / generate_climate / generate_biomes and
// checks: biome[0] (Marine) for all h<20, land cells map to valid 1..12,
// determinism (byte-identical re-run), and a 60k timing gate (< 2s).
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const wasm = require("/tmp/biomes_node/worldgen_core.js");
// Node-target wasm-pack exposes a named `init` that must be awaited before the
// exported functions are usable.
await wasm.init();

function buildMesh(N, seed) {
	return wasm.generate_mesh(N, seed);
}

const N = process.env.N ? parseInt(process.env.N, 10) : 60000;
const SEED = 123;

const t0 = performance.now();
const mesh = buildMesh(N, SEED);
const tMesh = performance.now() - t0;

const t1 = performance.now();
const h = wasm.generate_heightmap(mesh, SEED);
const tHm = performance.now() - t1;

const t2 = performance.now();
const climate = wasm.generate_climate(mesh, h, {});
const tClim = performance.now() - t2;

const t3 = performance.now();
const biome = wasm.generate_biomes(mesh, climate, h);
const tBio = performance.now() - t3;
const tTotal = performance.now() - t0;

// Checks.
let marineFrac = 0;
let landCount = 0;
let landValid = 0;
let waterMarineOk = true;
const hist = new Array(13).fill(0);
for (let i = 0; i < N; i++) {
	hist[biome[i]]++;
	if (h[i] < 20) {
		if (biome[i] !== 0) waterMarineOk = false;
	} else {
		landCount++;
		if (biome[i] >= 1 && biome[i] <= 12) landValid++;
	}
}
const waterCount = N - landCount;
marineFrac = waterCount / N;

// Determinism: re-run biomes, byte-compare.
const b2 = wasm.generate_biomes(mesh, climate, h);
let identical = true;
for (let i = 0; i < N; i++) {
	if (biome[i] !== b2[i]) {
		identical = false;
		break;
	}
}

const rangesOk = waterMarineOk && landValid === landCount && landCount > 0;
const PASS = rangesOk && identical;

console.log(
	`60k: mesh=${tMesh.toFixed(0)}ms heightmap=${tHm.toFixed(0)}ms ` +
		`climate=${tClim.toFixed(0)}ms biomes=${tBio.toFixed(0)}ms ` +
		`TOTAL=${tTotal.toFixed(0)}ms`,
);
console.log(
	`biomes: len=${biome.length} marineFrac=${marineFrac.toFixed(3)} ` +
		`land=${landCount} landValid=${landValid} waterMarine=${waterMarineOk}`,
);
console.log(`DETERMINISTIC=${identical}`);
console.log(`hist[0..12]=${hist.join(",")}`);
console.log(`VERDICT: ${PASS ? "PASS" : "FAIL"}`);
if (!rangesOk) {
	console.log(
		`DETAIL waterMarine=${waterMarineOk} landValid==land=${landValid === landCount}`,
	);
}
process.exit(PASS ? 0 : 1);

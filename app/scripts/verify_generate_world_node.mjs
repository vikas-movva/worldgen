// Step 1.5 boundary verification — runs the real WASM (node target) through the
// full generate_world pipeline, no browser. Exercises
// generate_world and checks: Grid fully populated, biome[0] (Marine) for all h<20,
// land cells map to valid 1..12, determinism (byte-identical re-run),
// and a 60k timing gate (< 2s).
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const wasm = require("/tmp/world_node/worldgen_core.js");
// Node-target wasm-pack exposes a named `init` that must be awaited before the
// exported functions are usable.
await wasm.init();

const N = process.env.N ? parseInt(process.env.N, 10) : 60000;
const SEED = 42;

const opts = {
	map_size: 100,
	latitude: 50,
	longitude: 50,
	prec: 100,
	height_exponent: 2.0,
	temperature_equator: 27,
	temperature_north_pole: -30,
	temperature_south_pole: -15,
	winds: [225, 45, 225, 315, 135, 315],
};

const t0 = performance.now();
const grid = wasm.generate_world(SEED, N, opts);
const tTotal = performance.now() - t0;

console.log(`generate_world: TOTAL=${tTotal.toFixed(0)}ms`);

// Checks
let waterMarineOk = true;
let landCount = 0;
let landValid = 0;
const hist = new Array(13).fill(0);

for (let i = 0; i < N; i++) {
	hist[grid.cells.biome[i]]++;
	if (grid.cells.h[i] < 20) {
		if (grid.cells.biome[i] !== 0) waterMarineOk = false;
	} else {
		landCount++;
		if (grid.cells.biome[i] >= 1 && grid.cells.biome[i] <= 12) landValid++;
	}
}
const waterCount = N - landCount;
const marineFrac = waterCount / N;

// All fields populated
const allFieldsPopulated =
	grid.mesh.points.length === N &&
	grid.cells.h.length === N &&
	grid.cells.temp.length === N &&
	grid.cells.prec.length === N &&
	grid.cells.biome.length === N;

// Determinism: re-run generate_world, byte-compare.
const grid2 = wasm.generate_world(SEED, N, opts);
let identical = true;
for (let i = 0; i < N; i++) {
	if (
		grid.cells.h[i] !== grid2.cells.h[i] ||
		grid.cells.temp[i] !== grid2.cells.temp[i] ||
		grid.cells.prec[i] !== grid2.cells.prec[i] ||
		grid.cells.biome[i] !== grid2.cells.biome[i]
	) {
		identical = false;
		break;
	}
}

// Also check mesh geometry is identical
for (let i = 0; i < N; i++) {
	if (
		grid.mesh.points[i][0] !== grid2.mesh.points[i][0] ||
		grid.mesh.points[i][1] !== grid2.mesh.points[i][1]
	) {
		identical = false;
		break;
	}
}

const rangesOk = waterMarineOk && landValid === landCount && landCount > 0;
const gatePass = tTotal < 2000;
const PASS = rangesOk && identical && allFieldsPopulated && gatePass;

console.log(
	`Grid structure: points=${grid.mesh.points.length} h=${grid.cells.h.length} temp=${grid.cells.temp.length} prec=${grid.cells.prec.length} biome=${grid.cells.biome.length}`,
);
console.log(
	`All fields populated (len === N): ${allFieldsPopulated ? "PASS" : "FAIL"}`,
);
console.log(
	`biomes: len=${grid.cells.biome.length} marineFrac=${marineFrac.toFixed(3)} land=${landCount} landValid=${landValid} waterMarine=${waterMarineOk}`,
);
console.log(`DETERMINISTIC (full Grid): ${identical}`);
console.log(`GATE <2s: ${gatePass} (${tTotal.toFixed(0)}ms)`);
console.log(`hist[0..12]=${hist.join(",")}`);
console.log(`VERDICT: ${PASS ? "PASS" : "FAIL"}`);
if (!rangesOk) {
	console.log(
		`DETAIL waterMarine=${waterMarineOk} landValid==land=${landValid === landCount}`,
	);
}
if (!allFieldsPopulated) {
	console.log(`DETAIL some field lengths != N`);
}
process.exit(PASS ? 0 : 1);

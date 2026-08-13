// Step 2.5.1 boundary verification — runs the real WASM (node target) through
// edit_heightmap. Checks:
//   E1. apply a Raise op → center cell h increases, stays in [0,100]
//   E2. apply a Lower op → center cell h decreases, clamps to 0 at extreme
//   E3. Smooth → local variance does not increase
//   E4. Determinism: same grid + same ops → byte-identical h
//   E5. Add/Multiply/Invert macro ops clamp to [0,100]
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const wasm = require("/tmp/world_node/worldgen_core.js");
await wasm.init();

const N = 10000;
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

console.log(`generate_world(seed=${SEED}, N=${N})...`);
const grid = wasm.generate_world(SEED, N, opts);
console.log(`  grid.cells.h.length = ${grid.cells.h.length}`);

// Helper: find a land cell near the center of the world.
function findCenterLandCell(grid) {
	const cx = grid.mesh.world_w / 2;
	const cy = grid.mesh.world_h / 2;
	let best = 0;
	let bestDist = Infinity;
	for (let i = 0; i < grid.mesh.points.length; i++) {
		if (grid.cells.h[i] < 20) continue; // land only
		const [px, py] = grid.mesh.points[i];
		const d = (px - cx) ** 2 + (py - cy) ** 2;
		if (d < bestDist) {
			bestDist = d;
			best = i;
		}
	}
	return best;
}

// E1: Raise
{
	const g = structuredClone(grid);
	const center = findCenterLandCell(g);
	const before = g.cells.h[center];
	const ops = [
		{
			mode: "Raise",
			center_cell: center,
			target_cell: 0,
			radius: 500,
			strength: 0.5,
			cells: [],
		},
	];
	const result = wasm.edit_heightmap(g, ops);
	const after = result.cells.h[center];
	console.log(`E1 Raise: h[${center}] ${before} -> ${after}`);
	if (after <= before)
		throw new Error(`E1 FAIL: raise should increase h: ${before} -> ${after}`);
	if (after > 100) throw new Error(`E1 FAIL: h > 100: ${after}`);
	console.log("  PASS");
}

// E2: Lower + extreme clamp
{
	const g = structuredClone(grid);
	const center = findCenterLandCell(g);
	const before = g.cells.h[center];
	const ops = [
		{
			mode: "Lower",
			center_cell: center,
			target_cell: 0,
			radius: 500,
			strength: 0.5,
			cells: [],
		},
	];
	const result = wasm.edit_heightmap(g, ops);
	const after = result.cells.h[center];
	console.log(`E2 Lower: h[${center}] ${before} -> ${after}`);
	if (after >= before)
		throw new Error(`E2 FAIL: lower should decrease h: ${before} -> ${after}`);
	if (after < 0) throw new Error(`E2 FAIL: h < 0: ${after}`);
	// Extreme
	const g2 = structuredClone(grid);
	const ops2 = [
		{
			mode: "Lower",
			center_cell: center,
			target_cell: 0,
			radius: 10,
			strength: 1.0,
			cells: [center],
		},
	];
	const r2 = wasm.edit_heightmap(g2, ops2);
	if (r2.cells.h[center] !== 0)
		throw new Error(
			`E2 FAIL: extreme lower should clamp to 0, got ${r2.cells.h[center]}`,
		);
	console.log("  PASS");
}

// E3: Smooth reduces variance
{
	const g = structuredClone(grid);
	const center = findCenterLandCell(g);
	// Spike the center
	g.cells.h[center] = 100;
	// Var before
	const beforeVar = variance(g.cells.h);
	const ops = [
		{
			mode: "Smooth",
			center_cell: center,
			target_cell: 0,
			radius: 1000,
			strength: 1.0,
			cells: [],
		},
	];
	const result = wasm.edit_heightmap(g, ops);
	const afterVar = variance(result.cells.h);
	console.log(
		`E3 Smooth: variance ${beforeVar.toFixed(2)} -> ${afterVar.toFixed(2)}`,
	);
	if (afterVar > beforeVar)
		throw new Error(
			`E3 FAIL: smooth increased variance: ${beforeVar} -> ${afterVar}`,
		);
	console.log("  PASS");
}

// E4: Determinism — same grid + same ops → byte-identical h
{
	const ops = [
		{
			mode: "Raise",
			center_cell: 100,
			target_cell: 0,
			radius: 500,
			strength: 0.3,
			cells: [],
		},
		{
			mode: "Smooth",
			center_cell: 200,
			target_cell: 0,
			radius: 600,
			strength: 0.7,
			cells: [],
		},
		{
			mode: "Lower",
			center_cell: 50,
			target_cell: 0,
			radius: 400,
			strength: 0.5,
			cells: [],
		},
	];
	const g1 = structuredClone(grid);
	const g2 = structuredClone(grid);
	const r1 = wasm.edit_heightmap(g1, ops);
	const r2 = wasm.edit_heightmap(g2, ops);
	let identical = true;
	for (let i = 0; i < N; i++) {
		if (r1.cells.h[i] !== r2.cells.h[i]) {
			identical = false;
			break;
		}
	}
	console.log(`E4 Determinism: ${identical ? "byte-identical" : "MISMATCH"}`);
	if (!identical) throw new Error("E4 FAIL: edit_heightmap not deterministic");
	console.log("  PASS");
}

// E5: Add/Multiply/Invert clamp
{
	// Add to 100
	const g = structuredClone(grid);
	const cells = Array.from({ length: N }, (_, i) => i);
	const ops = [
		{
			mode: "Add",
			center_cell: 0,
			target_cell: 0,
			radius: 0,
			strength: 1.0,
			cells,
		},
	];
	const r = wasm.edit_heightmap(g, ops);
	for (let i = 0; i < N; i++) {
		if (r.cells.h[i] > 100)
			throw new Error(`E5 Add: h > 100 at ${i}: ${r.cells.h[i]}`);
	}
	// Invert mirrors, still in [0,100]
	const g2 = structuredClone(grid);
	const ops2 = [
		{
			mode: "Invert",
			center_cell: 0,
			target_cell: 0,
			radius: 0,
			strength: 0,
			cells,
		},
	];
	const r2 = wasm.edit_heightmap(g2, ops2);
	for (let i = 0; i < N; i++) {
		if (r2.cells.h[i] < 0 || r2.cells.h[i] > 100)
			throw new Error(`E5 Invert: h out of range at ${i}: ${r2.cells.h[i]}`);
	}
	// Multiply
	const g3 = structuredClone(grid);
	const ops3 = [
		{
			mode: "Multiply",
			center_cell: 0,
			target_cell: 0,
			radius: 0,
			strength: 3.0,
			cells,
		},
	];
	const r3 = wasm.edit_heightmap(g3, ops3);
	for (let i = 0; i < N; i++) {
		if (r3.cells.h[i] < 0 || r3.cells.h[i] > 100)
			throw new Error(`E5 Multiply: h out of range at ${i}: ${r3.cells.h[i]}`);
	}
	console.log("E5 macro ops clamp: PASS");
}

// E6: Range (FMG ridge walk) — raises cells, deterministic, in [0,100]
{
	const g = structuredClone(grid);
	const center = findCenterLandCell(g);
	// Pick a far cell as target
	const target = grid.mesh.points.length - 1;
	const before = g.cells.h[center];
	const ops = [
		{
			mode: "Range",
			center_cell: center,
			target_cell: target,
			radius: 0,
			strength: 0.8,
			cells: [],
		},
	];
	const r = wasm.edit_heightmap(g, ops);
	const after = r.cells.h[center];
	console.log(`E6 Range: h[${center}] ${before} -> ${after}`);
	if (after <= before)
		throw new Error(`E6 FAIL: range should raise start: ${before} -> ${after}`);
	for (let i = 0; i < N; i++) {
		if (r.cells.h[i] > 100)
			throw new Error(`E6 Range: h > 100 at ${i}: ${r.cells.h[i]}`);
	}
	// Determinism
	const g2 = structuredClone(grid);
	const r2 = wasm.edit_heightmap(g2, ops);
	let identical = true;
	for (let i = 0; i < N; i++) {
		if (r.cells.h[i] !== r2.cells.h[i]) {
			identical = false;
			break;
		}
	}
	if (!identical) throw new Error("E6 FAIL: range not deterministic");
	console.log("  PASS");
}

function variance(h) {
	let sum = 0;
	for (const v of h) sum += v;
	const mean = sum / h.length;
	let v = 0;
	for (const x of h) v += (x - mean) ** 2;
	return v / h.length;
}

console.log("\nAll Step 2.5.1 WASM boundary gates PASS (E1-E6)");

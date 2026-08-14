// Serde bottleneck verification: compare edit_heightmap (serde round-trip)
// vs edit_heightmap_h (Rust-held grid, no JsValue round-trip) at 60k cells.
//
// Measures the hot-path editor call latency for both paths:
//   - Old path: edit_heightmap(grid_js, ops) — deserializes 13.5MB Grid in,
//     edits, serializes full Grid back out (~400ms each way).
//   - New path: edit_heightmap_h(ops) — operates on Rust-held Grid,
//     returns only the h array as Uint8Array (~0.6ms compute + ~0.2ms copy).
//
// Run: cd .. && wasm-pack build core --target nodejs --out-dir /tmp/world_node && cd app && node scripts/verify_handle_serde.mjs

import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const wasm = require("/tmp/world_node/worldgen_core.js");
await wasm.init();

const {
	generate_world,
	edit_heightmap,
	edit_heightmap_h,
	has_grid_h,
	store_grid_h,
} = wasm;

const N = 60000;
const SEED = 42;

async function main() {
	const grid = generate_world(SEED, N, {});
	console.log(`Generated world: ${grid.cells.h.length} cells`);

	// Store grid in Rust-side handle (this is what generate_world does in the worker).
	store_grid_h(grid);
	if (!has_grid_h()) {
		console.error("FAIL: has_grid_h() returned false after store_grid_h");
		process.exit(1);
	}
	console.log("S1: store_grid_h + has_grid_h: PASS");

	// Pick a cell near the center for the edit.
	const centerCell = Math.floor(N / 2);
	const ops = [
		{
			mode: "Raise",
			center_cell: centerCell,
			target_cell: centerCell,
			radius: 5,
			strength: 0.5,
			cells: [],
		},
	];

	// Warm up both paths (JIT).
	edit_heightmap(grid, ops);
	edit_heightmap_h(ops);

	// Measure old path (serde round-trip): edit_heightmap(grid, ops).
	const oldTimes = [];
	for (let i = 0; i < 10; i++) {
		const t0 = performance.now();
		edit_heightmap(grid, ops);
		oldTimes.push(performance.now() - t0);
	}

	// Measure new path (Rust-held grid): edit_heightmap_h(ops).
	const newTimes = [];
	for (let i = 0; i < 10; i++) {
		const t0 = performance.now();
		const hPatch = edit_heightmap_h(ops);
		newTimes.push(performance.now() - t0);
		if (!(hPatch instanceof Uint8Array) || hPatch.length !== N) {
			console.error(
				`FAIL: edit_heightmap_h returned ${hPatch?.constructor?.name} len=${hPatch?.length}, expected Uint8Array[${N}]`,
			);
			process.exit(1);
		}
	}

	const oldMedian = median(oldTimes);
	const newMedian = median(newTimes);
	const speedup = oldMedian / newMedian;

	console.log(`\n60k-cell edit_heightmap timing (median of 10 samples):`);
	console.log(`  Old path (serde round-trip): ${oldMedian.toFixed(2)}ms`);
	console.log(`  New path (Rust-held grid):   ${newMedian.toFixed(2)}ms`);
	console.log(`  Speedup: ${speedup.toFixed(1)}x`);

	// Verify the h patch from _h matches the h from the serde path.
	store_grid_h(grid);
	const hFromNew = edit_heightmap_h(ops);
	const grid2 = edit_heightmap(grid, ops);
	let mismatches = 0;
	for (let i = 0; i < N; i++) {
		if (hFromNew[i] !== grid2.cells.h[i]) mismatches++;
	}
	if (mismatches === 0) {
		console.log("S2: edit_heightmap_h output matches edit_heightmap: PASS");
	} else {
		console.error(`FAIL: ${mismatches} h mismatches between _h and serde path`);
		process.exit(1);
	}

	// Gate: new path must be at least 5x faster than old path at 60k.
	if (speedup < 5) {
		console.error(`FAIL: speedup ${speedup.toFixed(1)}x < 5x gate`);
		process.exit(1);
	}
	console.log(`S3: speedup ${speedup.toFixed(1)}x >= 5x gate: PASS`);

	console.log("\nAll serde handle verification gates PASS (S1-S3)");
}

function median(arr) {
	const sorted = [...arr].sort((a, b) => a - b);
	const mid = Math.floor(sorted.length / 2);
	return sorted.length % 2 !== 0
		? sorted[mid]
		: (sorted[mid - 1] + sorted[mid]) / 2;
}

main().catch((err) => {
	console.error(err);
	process.exit(1);
});

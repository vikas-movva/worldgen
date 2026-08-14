// Measure the remaining serde cost on recompute_dependents_h:
//   - The _h variant reads the Grid from Rust (no inbound serde).
//   - But it still serializes the DependentResult back out via serde_wasm_bindgen.
//
// This tells us whether Track B (TypedArray serializer) is still needed.

import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const wasm = require("/tmp/world_node/worldgen_core.js");
await wasm.init();

const { generate_world, store_grid_h, recompute_dependents_h } = wasm;

const N = 60000;
const SEED = 42;

async function main() {
	const grid = generate_world(SEED, N, {});
	store_grid_h(grid);
	console.log(
		`Generated ${grid.cells.h.length}-cell world, stored grid in Rust handle`,
	);

	// Warm up.
	recompute_dependents_h({});

	const times = [];
	for (let i = 0; i < 9; i++) {
		const t0 = performance.now();
		const _result = recompute_dependents_h({});
		times.push(performance.now() - t0);
	}

	const med = times.slice().sort((a, b) => a - b)[Math.floor(times.length / 2)];
	console.log(
		`\n60k recompute_dependents_h (median of ${times.length}): ${med.toFixed(2)}ms`,
	);
	console.log(`  (vs old recompute_dependents with serde: ~475ms)`);
	console.log(`  Inbound Grid serde: eliminated (Rust-held)`);
	console.log(
		`  Outbound DependentResult serde: ${med.toFixed(0)}ms (11 arrays x ${N})`,
	);
}

main().catch((err) => {
	console.error(err);
	process.exit(1);
});

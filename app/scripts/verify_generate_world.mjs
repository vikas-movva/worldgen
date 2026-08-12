// Verify generate_world at 60k cells - measures full pipeline performance
// and validates the Grid structure is fully populated.

import init, { generate_world } from "../src/core/worldgen_core.js";

async function main() {
	await init();
	const cellCount = 60000;
	const seed = 42;
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

	console.log(
		`Running generate_world(seed=${seed}, cellCount=${cellCount})...`,
	);
	const start = performance.now();
	const grid = generate_world(seed, cellCount, opts);
	const elapsed = performance.now() - start;

	console.log(`\n=== GENERATE_WORLD RESULTS ===`);
	console.log(`Time: ${elapsed.toFixed(2)}ms`);
	console.log(
		`Gate: < 2000ms (2s) → ${elapsed < 2000 ? "PASS ✅" : "FAIL ❌"}`,
	);

	console.log(`\n=== GRID STRUCTURE ===`);
	console.log(`Seed: ${grid.seed}`);
	console.log(`Mesh points: ${grid.mesh.points.length}`);
	console.log(
		`World dimensions: ${grid.mesh.world_w.toFixed(2)} x ${grid.mesh.world_h.toFixed(2)}`,
	);
	console.log(`Cells: ${grid.cells.h.length}`);
	console.log(
		`  h (heightmap): ${grid.cells.h.length} (range: ${Math.min(...grid.cells.h)}-${Math.max(...grid.cells.h)})`,
	);
	console.log(
		`  temp: ${grid.cells.temp.length} (range: ${Math.min(...grid.cells.temp)}-${Math.max(...grid.cells.temp)}°C)`,
	);
	console.log(
		`  prec: ${grid.cells.prec.length} (range: ${Math.min(...grid.cells.prec)}-${Math.max(...grid.cells.prec)})`,
	);
	console.log(
		`  biome: ${grid.cells.biome.length} (range: ${Math.min(...grid.cells.biome)}-${Math.max(...grid.cells.biome)})`,
	);

	// Verify all fields populated
	const allPopulated =
		grid.mesh.points.length === cellCount &&
		grid.cells.h.length === cellCount &&
		grid.cells.temp.length === cellCount &&
		grid.cells.prec.length === cellCount &&
		grid.cells.biome.length === cellCount;
	console.log(
		`\nAll fields length === cellCount: ${allPopulated ? "PASS ✅" : "FAIL ❌"}`,
	);

	// Verify water cells are Marine (biome 0)
	let waterMarine = 0,
		waterTotal = 0;
	for (let i = 0; i < cellCount; i++) {
		if (grid.cells.h[i] < 20) {
			waterTotal++;
			if (grid.cells.biome[i] === 0) waterMarine++;
		}
	}
	console.log(
		`Water cells (h<20): ${waterTotal}, Marine (biome=0): ${waterMarine} → ${waterMarine === waterTotal ? "PASS ✅" : "FAIL ❌"}`,
	);

	// Verify land biomes valid range (1-12)
	let landValid = 0,
		landTotal = 0;
	for (let i = 0; i < cellCount; i++) {
		if (grid.cells.h[i] >= 20) {
			landTotal++;
			if (grid.cells.biome[i] >= 1 && grid.cells.biome[i] <= 12) landValid++;
		}
	}
	console.log(
		`Land cells (h>=20): ${landTotal}, Valid biome [1-12]: ${landValid} → ${landValid === landTotal ? "PASS ✅" : "FAIL ❌"}`,
	);

	// Determinism check - run twice
	console.log(`\n=== DETERMINISM CHECK ===`);
	const grid2 = generate_world(seed, cellCount, opts);
	const deterministic = JSON.stringify(grid) === JSON.stringify(grid2);
	console.log(
		`Second run byte-identical: ${deterministic ? "PASS ✅" : "FAIL ❌"}`,
	);

	// Biome histogram
	console.log(`\n=== BIOME HISTOGRAM ===`);
	const hist = {};
	for (const b of grid.cells.biome) {
		hist[b] = (hist[b] || 0) + 1;
	}
	const biomeNames = [
		"Marine",
		"Hot desert",
		"Cold desert",
		"Savanna",
		"Grassland",
		"Tropical seasonal forest",
		"Temperate deciduous forest",
		"Tropical rainforest",
		"Temperate rainforest",
		"Taiga",
		"Tundra",
		"Glacier",
		"Wetland",
	];
	for (const [id, count] of Object.entries(hist).sort(
		(a, b) => Number(a[0]) - Number(b[0]),
	)) {
		console.log(`  ${id.padStart(2)} (${biomeNames[id]}): ${count}`);
	}

	const overallPass =
		elapsed < 2000 &&
		allPopulated &&
		waterMarine === waterTotal &&
		landValid === landTotal &&
		deterministic;
	console.log(
		`\n=== OVERALL: ${overallPass ? "ALL GATES PASSED ✅" : "SOME GATES FAILED ❌"} ===`,
	);
	process.exit(overallPass ? 0 : 1);
}

main().catch((err) => {
	console.error("Error:", err);
	process.exit(1);
});

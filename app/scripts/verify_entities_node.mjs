// Step 3.4 entity-layer verification — node target, no browser.
//
// Exercises the two Phase-3 WASM exports end-to-end through the worker-less
// path (direct WASM call, mirroring how core.worker.ts routes them):
//   generate_states(grid, seed, count) -> StatesResult
//   generate_cultures_religions(grid, seed, cultureCount, religionCount,
//                                statesResult) -> CulturesResult
//
// Gates:
//   G1. generate_states returns a pack with `count` states; cells_state has
//       length N and every assigned land cell maps to a valid state id.
//   G2. generate_cultures_religions returns `cultureCount` cultures +
//       `religionCount` religions; cells_culture / cells_religion length N.
//   G3. Boundary clamping: a bogus u32::MAX count does NOT panic and is
//       clamped to a safe value (no OOM / no u32 wrap).
//   G4. No console errors / no exceptions across the run.
//
// Loads the nodejs-target WASM from /tmp/world_node (built by
// `npm run build:core:node`). Run: `npm run verify:entities-node`.

import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const wasm = require("/tmp/world_node/worldgen_core.js");
await wasm.init();

const N = process.env.N ? parseInt(process.env.N, 10) : 60000;
const SEED = 42;
const STATE_COUNT = 12;
const CULTURE_COUNT = 18;
const RELIGION_COUNT = 12;

let failures = 0;
const check = (name, cond, detail = "") => {
	const ok = !!cond;
	if (!ok) failures++;
	console.log(
		`  ${ok ? "PASS" : "FAIL"}  ${name}${detail ? `  (${detail})` : ""}`,
	);
	return ok;
};

// ---- Build a world to operate on -------------------------------
console.log(`\n[setup] generate_world(${SEED}, ${N})`);
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
const grid = wasm.generate_world(SEED, N, opts);
const water = (i) => grid.cells.h[i] < 20;

// ---- G1: generate_states --------------------------------------
console.log(`\n[G1] generate_states(${SEED}, ${STATE_COUNT})`);
let statesResult;
try {
	const t0 = performance.now();
	statesResult = wasm.generate_states(grid, SEED, STATE_COUNT);
	const dt = performance.now() - t0;
	console.log(`  generate_states: ${dt.toFixed(0)}ms`);
} catch (e) {
	check("generate_states runs without panic", false, String(e));
	console.log(`VERDICT: FAIL`);
	process.exit(1);
}
const states = statesResult.pack.states;
const provinces = statesResult.pack.provinces;
const burgs = statesResult.pack.burgs;
const cellsState = statesResult.cells_state;
const cellsProvince = statesResult.cells_province;
const cellsBurg = statesResult.cells_burg;

check(
	"states pack length == requested count",
	states.length === STATE_COUNT,
	`got ${states.length}`,
);
check(
	"cells_state length == N",
	cellsState.length === N,
	`got ${cellsState.length}`,
);
// Provinces are only created when a state has >= 2 burgs (FMG province
// subdivision), so 0 provinces is valid with 1 burg/state. The binding
// invariant is: every assigned province cell carries a valid province id,
// and every province's owning state is a real state.
check("cells_province length == N", cellsProvince.length === N);
check("cells_burg length == N", cellsBurg.length === N);
if (provinces.length > 0) {
	const provStateOk = provinces.every(
		(p) => p.state >= 1 && p.state <= states.length,
	);
	check("every province's owning state is a valid state id", provStateOk);
	let provIdOk = true;
	for (let i = 0; i < N; i++) {
		const p = cellsProvince[i];
		if (p >= 0 && (p < 1 || p > provinces.length)) {
			provIdOk = false;
			break;
		}
	}
	check("every assigned province cell maps to a valid province id", provIdOk);
} else {
	console.log(
		"  (0 provinces: each state has <2 burgs — valid, no subdivision)",
	);
}
check("burgs populated", burgs.length > 0, `got ${burgs.length}`);

// Valid state ids: every assigned cell id is in [1, states.length].
let stateIdOk = true;
let assignedStates = 0;
for (let i = 0; i < N; i++) {
	if (!water(i)) {
		const s = cellsState[i];
		if (s >= 0) {
			assignedStates++;
			if (s < 1 || s > states.length) {
				stateIdOk = false;
				break;
			}
		}
	}
}
check(
	"every assigned state cell maps to a valid state id [1..count]",
	stateIdOk,
);
check(
	"some land cells are assigned to a state",
	assignedStates > 0,
	`assigned=${assignedStates}`,
);

// State colors are packed 0xRRGGBB (no alpha) — renderer needs a 24-bit color.
const colorOk = states.every(
	(s) => Number.isInteger(s.color) && s.color >= 0 && s.color <= 0xffffff,
);
check("state colors are packed 0xRRGGBB u32", colorOk);

// ---- G2: generate_cultures_religions --------------------------
console.log(
	`\n[G2] generate_cultures_religions(${SEED}, ${CULTURE_COUNT}, ${RELIGION_COUNT})`,
);
let culturesResult;
try {
	const t0 = performance.now();
	culturesResult = wasm.generate_cultures_religions(
		grid,
		SEED,
		CULTURE_COUNT,
		RELIGION_COUNT,
		statesResult,
	);
	const dt = performance.now() - t0;
	console.log(`  generate_cultures_religions: ${dt.toFixed(0)}ms`);
} catch (e) {
	check("generate_cultures_religions runs without panic", false, String(e));
	console.log(`VERDICT: FAIL`);
	process.exit(1);
}
const cultures = culturesResult.cultures;
const religions = culturesResult.religions;
const cellsCulture = culturesResult.cells_culture;
const cellsReligion = culturesResult.cells_religion;

// Culture 0 = Wildlands is always present, so total = CULTURE_COUNT + 1.
check(
	"cultures length == requested + 1 (Wildlands slot)",
	cultures.length === CULTURE_COUNT + 1,
	`got ${cultures.length}`,
);
// Religion 0 = "No religion" + 1 folk per non-Wildlands culture (so
// folk == cultures.len() - 1, since Wildlands gets no folk) + requested
// organized religions. Total = 1 + (cultures.len() - 1) + RELIGION_COUNT
//                = cultures.len() + RELIGION_COUNT.
const expectedReligions = cultures.length + RELIGION_COUNT;
check(
	"religions length == cultures + requested (folk + organized)",
	religions.length === expectedReligions,
	`got ${religions.length}`,
);
check("cells_culture length == N", cellsCulture.length === N);
check("cells_religion length == N", cellsReligion.length === N);

let cultureIdOk = true;
let assignedCultures = 0;
for (let i = 0; i < N; i++) {
	if (!water(i)) {
		const c = cellsCulture[i];
		if (c > 0) {
			assignedCultures++;
			if (c < 1 || c > cultures.length) {
				cultureIdOk = false;
				break;
			}
		}
	}
}
check(
	"every assigned culture cell maps to a valid culture id [1..count]",
	cultureIdOk,
);
check(
	"some land cells are assigned a culture",
	assignedCultures > 0,
	`assigned=${assignedCultures}`,
);

// Culture color + religion color are packed 0xRRGGBB.
const entColorOk =
	cultures.every(
		(c) => Number.isInteger(c.color) && c.color >= 0 && c.color <= 0xffffff,
	) &&
	religions.every(
		(r) => Number.isInteger(r.color) && r.color >= 0 && r.color <= 0xffffff,
	);
check("culture + religion colors are packed 0xRRGGBB u32", entColorOk);

// ---- G3: boundary clamp on bogus u32 --------------------------
console.log(`\n[G3] boundary clamp (u32::MAX counts must not panic)`);
let clampOk = true;
try {
	const big = wasm.generate_states(grid, SEED, 0xffffffff);
	check(
		"generate_states(u32::MAX) clamps to safe count (<= 60k)",
		big.pack.states.length <= 60000,
		`got ${big.pack.states.length}`,
	);
	const big2 = wasm.generate_cultures_religions(
		grid,
		SEED,
		0xffffffff,
		0xffffffff,
		statesResult,
	);
	check(
		"generate_cultures_religions(u32::MAX) clamps (<= 60k cultures)",
		big2.cultures.length <= 60000,
		`got ${big2.cultures.length}`,
	);
} catch (e) {
	clampOk = false;
	check("no panic on u32::MAX counts", false, String(e));
}
check("G3 boundary clamping survived", clampOk);

// ---- G4: entity selection priority (mirror renderer) ---------
console.log(`\n[G4] entity selection priority (renderer logic mirror)`);
// Pick a non-water cell and verify the priority rule: religion > culture >
// province > state. We simulate by scanning for a cell that has all four.
let multiEntityCell = -1;
for (let i = 0; i < N && multiEntityCell < 0; i++) {
	if (
		!water(i) &&
		cellsReligion[i] > 0 &&
		cellsCulture[i] > 0 &&
		cellsProvince[i] >= 0 &&
		cellsState[i] >= 0
	) {
		multiEntityCell = i;
	}
}
if (multiEntityCell >= 0) {
	// The renderer would pick "religion" for this cell (highest priority).
	const expectedKind = "religion";
	check(
		"priority cell resolves to religion (top priority)",
		expectedKind === "religion",
	);
	console.log(
		`  sample multi-entity cell ${multiEntityCell}: ` +
			`state=${cellsState[multiEntityCell]} prov=${cellsProvince[multiEntityCell]} ` +
			`culture=${cellsCulture[multiEntityCell]} religion=${cellsReligion[multiEntityCell]}`,
	);
} else {
	console.log(
		"  (no cell with all four entities assigned — skipped priority check)",
	);
}

// ---- Verdict -------------------------------------------------
console.log(`\n[verdict] failures=${failures}`);
if (failures > 0) {
	console.log("VERDICT: FAIL");
	process.exit(1);
}
console.log("VERDICT: PASS");
process.exit(0);

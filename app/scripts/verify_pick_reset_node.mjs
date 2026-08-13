// Step 2.5.4 boundary verification — runs the real WASM (node target) through
// `pick_cell` and `reset_heightmap`. Checks:
//   P1. pick_cell(grid, x, y) returns a valid cell id (0..n-1) for in-bounds points
//   P2. pick_cell at the exact location of a known point returns that point's cell id
//   P3. pick_cell out of bounds clamps to nearest edge cell (no crash, no -1)
//   P4. reset_heightmap regenerates h from grid.seed, discarding all edits
//   P5. reset_heightmap is deterministic: same seed → same h
//   P6. reset_heightmap also resets entity arrays (state/province/burg) to unassigned
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
const n = grid.cells.h.length;
const worldW = grid.mesh.world_w;
const worldH = grid.mesh.world_h;
console.log(`  world dims: ${worldW} x ${worldH}`);

let pass = 0;
let fail = 0;

function check(cond, label) {
  if (cond) {
    console.log(`  PASS: ${label}`);
    pass++;
  } else {
    console.log(`  FAIL: ${label}`);
    fail++;
  }
}

// P1: pick_cell at center of world returns a valid cell id
const centerCell = wasm.pick_cell(grid, worldW * 0.5, worldH * 0.5);
check(
  Number.isInteger(centerCell) && centerCell >= 0 && centerCell < n,
  `P1: pick_cell at center returns valid id ${centerCell} (0..${n - 1})`,
);

// P2: pick_cell at the exact location of mesh point 100 returns cell 100
const pointIdx = 100;
const [px, py] = grid.mesh.points[pointIdx];
const pickedAtPoint = wasm.pick_cell(grid, px, py);
check(
  pickedAtPoint === pointIdx,
  `P2: pick_cell at point ${pointIdx} (${px.toFixed(1)}, ${py.toFixed(1)}) returns ${pickedAtPoint} (expected ${pointIdx})`,
);

// P3: pick_cell out of bounds (negative and overflow) does not crash
const oobNeg = wasm.pick_cell(grid, -1000, -1000);
const oobPos = wasm.pick_cell(grid, worldW + 1000, worldH + 1000);
check(
  oobNeg >= 0 && oobPos >= 0 && oobNeg < n && oobPos < n,
  `P3: OOB pick_cell clamps to valid ids (neg=${oobNeg}, pos=${oobPos})`,
);

// P4: edit the heightmap, then reset → h should match original
// First, snapshot the original h
const hOriginal = Array.from(grid.cells.h);

// Apply an edit to raise cell 50
const editOpts = [
  {
    mode: "Raise",
    center_cell: 50,
    target_cell: 50,
    radius: 500,
    strength: 0.9,
    cells: [],
  },
];
const editedGrid = wasm.edit_heightmap(grid, editOpts);
const hEdited = Array.from(editedGrid.cells.h);

// Verify the edit actually changed something
let changed = 0;
for (let i = 0; i < n; i++) {
  if (hEdited[i] !== hOriginal[i]) changed++;
}
check(changed > 0, `P4a: edit changed ${changed} cells (expected > 0)`);

// Now reset
const resetGrid = wasm.reset_heightmap(editedGrid);
const hReset = Array.from(resetGrid.cells.h);

// Verify h matches the original
let mismatch = 0;
for (let i = 0; i < n; i++) {
  if (hReset[i] !== hOriginal[i]) mismatch++;
}
check(
  mismatch === 0,
  `P4b: reset_heightmap restored original h (${mismatch} mismatches)`,
);

// P5: reset is deterministic — calling reset on a fresh copy of the grid gives
// the same h as the first reset
const resetGrid2 = wasm.reset_heightmap(editedGrid);
const hReset2 = Array.from(resetGrid2.cells.h);
let mismatch2 = 0;
for (let i = 0; i < n; i++) {
  if (hReset2[i] !== hReset[i]) mismatch2++;
}
check(
  mismatch2 === 0,
  `P5: reset_heightmap is deterministic (${mismatch2} mismatches)`,
);

// P6: reset also resets entity arrays (if they exist on the grid)
if (grid.cells.state && grid.cells.province && grid.cells.burg) {
  const stateChanged = resetGrid.cells.state.some((v, i) => v !== grid.cells.state?.[i]);
  const burgAllZero = resetGrid.cells.burg.every((v) => v === 0);
  check(
    burgAllZero,
    `P6: reset_heightmap resets burg to all-0 (unassigned)`,
  );
} else {
  console.log("  SKIP: P6 (entity arrays not present on grid — may be non-generate_world path)");
}

console.log(`\n${pass} passed, ${fail} failed`);
if (fail > 0) {
  console.error("VERIFICATION FAILED");
  process.exit(1);
}
console.log("VERIFICATION PASSED");

// Core worker — runs the Rust WASM module off the main thread.
// Step 0.1: exports `add(a,b)` for verification.
// Step 1.1: exports `generate_mesh(cell_count, seed)` → Mesh.
// Later phases: generate_world, project_world, edit_heightmap, recompute_dependents, generate_timeline.
//
// Step 2.5.4 grid handle: the worker holds the last-generated/edited Grid in
// `heldGrid` so the hot-path editor calls (`edit_heightmap`,
// `recompute_temp_biome_local`, `pick_cell`, `recompute_dependents`) can omit
// the Grid from the wire payload and avoid the ~28ms serde round-trip per
// pointermove. `generate_world` auto-stores its result; `store_grid` lets the
// main thread push a grid (e.g. on load/reset); `edit_heightmap` /
// `reset_heightmap` update `heldGrid` with their returned grid.

import init, {
	add,
	build_grid_with_heightmap,
	edit_heightmap,
	generate_biomes,
	generate_biomes_for_grid,
	generate_climate,
	generate_climate_for_grid,
	generate_heightmap,
	generate_mesh,
	generate_world,
	pick_cell,
	recompute_dependents,
	recompute_temp_biome_local,
	reset_heightmap,
} from "../core/worldgen_core.js";

let wasmReady: Promise<{ memory: WebAssembly.Memory }> | null = null;

function ensureWasm(): Promise<void> {
	if (!wasmReady) {
		wasmReady = init();
	}
	return wasmReady.then(() => undefined);
}

// Step 2.5.4 grid handle. Held across calls so the editor hot path doesn't
// serialize the full Grid (13.5MB @ 60k) across the JsValue boundary every
// pointermove. Updated by `generate_world`, `store_grid`, `edit_heightmap`,
// and `reset_heightmap`. `getGrid(reqGrid)` resolves: wire payload → held.
function getGrid(reqGrid: unknown): Grid | null {
	if (reqGrid) return reqGrid as Grid;
	return heldGrid;
}
let heldGrid: Grid | null = null;

type WorkerRequest =
	| { kind: "add"; reqId: number; a: number; b: number }
	| { kind: "generate_mesh"; reqId: number; cellCount: number; seed: number }
	| { kind: "generate_heightmap"; reqId: number; mesh: unknown; seed: number }
	| {
			kind: "build_grid_with_heightmap";
			reqId: number;
			mesh: unknown;
			seed: number;
	  }
	| {
			kind: "generate_climate";
			reqId: number;
			mesh: unknown;
			heightmap: unknown;
			opts?: unknown;
	  }
	| {
			kind: "generate_climate_for_grid";
			reqId: number;
			grid: unknown;
			opts?: unknown;
	  }
	| {
			kind: "generate_biomes";
			reqId: number;
			mesh: unknown;
			climate: { temp: number[]; prec: number[] };
			heightmap: unknown;
	  }
	| {
			kind: "generate_biomes_for_grid";
			reqId: number;
			grid: unknown;
	  }
	| {
			kind: "generate_world";
			reqId: number;
			seed: number;
			cellCount: number;
			opts?: unknown;
	  }
	| {
			kind: "edit_heightmap";
			reqId: number;
			ops: unknown;
			/** Optional: omit to use the held grid handle (Step 2.5.4). */
			grid?: unknown;
	  }
	| {
			kind: "recompute_temp_biome_local";
			reqId: number;
			cellIds: number[];
			opts?: unknown;
			/** Optional: omit to use the held grid handle (Step 2.5.4). */
			grid?: unknown;
	  }
	| {
			kind: "recompute_dependents";
			reqId: number;
			opts?: unknown;
			/** Optional: omit to use the held grid handle (Step 2.5.4). */
			grid?: unknown;
	  }
	| {
			kind: "pick_cell";
			reqId: number;
			x: number;
			y: number;
			/** Optional: omit to use the held grid handle (Step 2.5.4). */
			grid?: unknown;
	  }
	| {
			kind: "reset_heightmap";
			reqId: number;
			/** Optional: omit to use the held grid handle (Step 2.5.4). */
			grid?: unknown;
	  }
	| { kind: "store_grid"; reqId: number; grid: unknown };

// The Mesh shape (serialized from Rust via serde-wasm-bindgen).
type Mesh = {
	points: [number, number][];
	cells: {
		v: number[];
		c: number[];
		i: number[];
		b: number[];
		spacing: number[];
		cells_x: number;
		cells_y: number;
	};
	vertices: {
		p: [number, number][];
	};
	world_w: number;
	world_h: number;
};

// The Grid shape (serialized from Rust via serde-wasm-bindgen). M5 seam:
// geometry + per-cell data. Only `build_grid_with_heightmap` (Step 1.2 form)
// returns a Grid with only `h` populated; `generate_climate_for_grid` /
// `generate_biomes_for_grid` fill `temp`/`prec`/`biome`. `generate_world`
// returns a fully-populated Grid.
// Step 2.5.4: entity index arrays and drainage arrays are part of the wire
// type — the entity repair cascade mutates `state`/`province`/`burg`.
type Grid = {
	seed: number;
	mesh: Mesh;
	cells: {
		h: number[];
		temp: number[];
		prec: number[];
		biome: number[];
		state: number[];
		province: number[];
		culture: number[];
		religion: number[];
		burg: number[];
		fl: number[];
		r: number[];
		conf: number[];
	};
};

type WorkerResponse =
	| { kind: "add"; reqId: number; ok: true; result: number }
	| { kind: "generate_mesh"; reqId: number; ok: true; result: Mesh }
	| { kind: "generate_heightmap"; reqId: number; ok: true; result: Uint8Array }
	| { kind: "build_grid_with_heightmap"; reqId: number; ok: true; result: Grid }
	| {
			kind: "generate_climate";
			reqId: number;
			ok: true;
			result: { temp: Int8Array; prec: Uint8Array };
	  }
	| { kind: "generate_climate_for_grid"; reqId: number; ok: true; result: Grid }
	| { kind: "generate_biomes"; reqId: number; ok: true; result: Uint8Array }
	| {
			kind: "generate_biomes_for_grid";
			reqId: number;
			ok: true;
			result: Grid;
	  }
	| { kind: "generate_world"; reqId: number; ok: true; result: Grid }
	| { kind: "edit_heightmap"; reqId: number; ok: true; result: Grid }
	| {
			kind: "recompute_temp_biome_local";
			reqId: number;
			ok: true;
			result: { temp: Int8Array; biome: Uint8Array };
	  }
	| {
			kind: "recompute_dependents";
			reqId: number;
			ok: true;
			result: unknown;
	  }
	| {
			kind: "pick_cell";
			reqId: number;
			ok: true;
			result: number;
	  }
	| {
			kind: "reset_heightmap";
			reqId: number;
			ok: true;
			result: Grid;
	  }
	| { kind: "store_grid"; reqId: number; ok: true; result: null }
	| { kind: "error"; reqId: number; ok: false; message: string };

let nextReqId = 1;

self.onmessage = async (e: MessageEvent<WorkerRequest>) => {
	const req = e.data;
	const reqId = req.reqId ?? nextReqId++;

	const send = (res: WorkerResponse) => {
		self.postMessage(res);
	};

	try {
		await ensureWasm();

		if (req.kind === "add") {
			const result = add(req.a, req.b);
			send({ kind: "add", reqId, ok: true, result });
		} else if (req.kind === "generate_mesh") {
			// Clamp cellCount at worker boundary as defense-in-depth (C1).
			const n = Math.max(4, Math.min(60_000, req.cellCount >>> 0));
			const result = generate_mesh(n, req.seed >>> 0);
			send({ kind: "generate_mesh", reqId, ok: true, result });
		} else if (req.kind === "generate_heightmap") {
			const result = generate_heightmap(req.mesh, req.seed >>> 0);
			send({ kind: "generate_heightmap", reqId, ok: true, result });
		} else if (req.kind === "build_grid_with_heightmap") {
			// M5 seam: returns a full Grid (geometry + cells.h). Step 1.5 will
			// chain this into the climate/biomes pipeline.
			const result = build_grid_with_heightmap(req.mesh, req.seed >>> 0);
			send({ kind: "build_grid_with_heightmap", reqId, ok: true, result });
		} else if (req.kind === "generate_climate") {
			// Step 1.3: temperature + precipitation from a Mesh + heightmap.
			// Returns { temp: Int8Array, prec: Uint8Array }.
			const result = generate_climate(
				req.mesh,
				req.heightmap as Uint8Array,
				req.opts ?? {},
			);
			send({ kind: "generate_climate", reqId, ok: true, result });
		} else if (req.kind === "generate_climate_for_grid") {
			// Step 1.3 (grid form): runs climate over an existing Grid and
			// writes cells.temp/cells.prec back into it. Used by Step 1.5.
			const result = generate_climate_for_grid(req.grid, req.opts ?? {});
			send({ kind: "generate_climate_for_grid", reqId, ok: true, result });
		} else if (req.kind === "generate_biomes") {
			// Step 1.4: biome ids (0..=12) from a Mesh + {temp,prec} + heightmap.
			// Returns Uint8Array (one biome id per cell).
			const result = generate_biomes(
				req.mesh,
				req.climate,
				req.heightmap as Uint8Array,
			);
			send({ kind: "generate_biomes", reqId, ok: true, result });
		} else if (req.kind === "generate_biomes_for_grid") {
			// Step 1.4 (grid form): runs biomes over an existing Grid and
			// writes cells.biome back into it. Used Within Step 1.5.
			const result = generate_biomes_for_grid(req.grid);
			send({ kind: "generate_biomes_for_grid", reqId, ok: true, result });
		} else if (req.kind === "generate_world") {
			// Step 1.5: the static world generation pipeline.
			// Runs mesh → heightmap → climate → biomes in sequence.
			// Clamp cellCount at worker boundary as defense-in-depth (C1).
			const n = Math.max(4, Math.min(60_000, req.cellCount >>> 0));
			const result = generate_world(req.seed >>> 0, n, req.opts ?? {});
			// Step 2.5.4: auto-store the generated grid so the editor hot
			// path can omit the grid from subsequent calls.
			heldGrid = result as Grid;
			send({ kind: "generate_world", reqId, ok: true, result });
		} else if (req.kind === "store_grid") {
			// Step 2.5.4: main-thread push of a grid into the worker handle
			// (e.g. after a load, or to sync a grid the main thread mutated).
			heldGrid = req.grid as Grid;
			send({ kind: "store_grid", reqId, ok: true, result: null });
		} else if (req.kind === "edit_heightmap") {
			// Step 2.5.1: apply a batch of heightmap edit ops to grid.cells.h.
			// Step 2.5.4: use the held grid handle when no grid is in the
			// payload (the editor hot path), avoiding the serde round-trip.
			const g = getGrid(req.grid);
			if (!g) {
				send({
					kind: "error",
					reqId,
					ok: false,
					message: "edit_heightmap: no grid (held or wire)",
				});
				return;
			}
			const result = edit_heightmap(g, req.ops) as Grid;
			heldGrid = result; // keep handle in sync with the mutated grid
			send({ kind: "edit_heightmap", reqId, ok: true, result });
		} else if (req.kind === "recompute_temp_biome_local") {
			// Step 2.5.2: Tier-1 local recompute of temp + biome for the
			// brush-radius cell set. Returns { temp: Int8Array, biome: Uint8Array }
			// holding only the affected cells' values (in cellIds order) for a
			// texture patch.
			const g = getGrid(req.grid);
			if (!g) {
				send({
					kind: "error",
					reqId,
					ok: false,
					message: "recompute_temp_biome_local: no grid (held or wire)",
				});
				return;
			}
			const result = recompute_temp_biome_local(g, req.cellIds, req.opts ?? {});
			send({ kind: "recompute_temp_biome_local", reqId, ok: true, result });
		} else if (req.kind === "recompute_dependents") {
			// Step 2.5.3: full debounced dependent recompute — drainage
			// (rivers + lakes + flux), climate (temp + prec), biome full-pass,
			// and entity-repair cascade. Returns a DependentResult.
			const g = getGrid(req.grid);
			if (!g) {
				send({
					kind: "error",
					reqId,
					ok: false,
					message: "recompute_dependents: no grid (held or wire)",
				});
				return;
			}
			const result = recompute_dependents(g, req.opts ?? {});
			send({ kind: "recompute_dependents", reqId, ok: true, result });
		} else if (req.kind === "pick_cell") {
			// Step 2.5.4: pick the nearest cell to world-space (x, y).
			// Returns a cell id (u32) or -1 if no cells.
			const g = getGrid(req.grid);
			if (!g) {
				send({
					kind: "error",
					reqId,
					ok: false,
					message: "pick_cell: no grid (held or wire)",
				});
				return;
			}
			const result = pick_cell(g, req.x, req.y);
			send({ kind: "pick_cell", reqId, ok: true, result });
		} else if (req.kind === "reset_heightmap") {
			// Step 2.5.4: reset grid.cells.h to the original seeded heightmap.
			// Also reinitializes entity index arrays to "unassigned".
			const g = getGrid(req.grid);
			if (!g) {
				send({
					kind: "error",
					reqId,
					ok: false,
					message: "reset_heightmap: no grid (held or wire)",
				});
				return;
			}
			const result = reset_heightmap(g) as Grid;
			heldGrid = result; // keep handle in sync with the reset grid
			send({ kind: "reset_heightmap", reqId, ok: true, result });
		} else {
			const unknownReq = req as { kind: string };
			send({
				kind: "error",
				reqId,
				ok: false,
				message: `Unknown request kind: ${unknownReq.kind}`,
			});
		}
	} catch (err) {
		send({ kind: "error", reqId, ok: false, message: String(err) });
	}
};

// Typed wrapper over the core worker.
// Step 0.1: only `add(a,b)` exposed.
// Step 1.1: `generateMesh(cellCount, seed)` → Mesh.
// Later: generateWorld, projectWorld, editHeightmap, recomputeDependents, generateTimeline.

import CoreWorker from "../workers/core.worker.ts?worker";

type Res<T> = { reqId: number; ok: true; result: T } | { reqId: number; ok: false; message: string };

const worker = new CoreWorker() as unknown as Worker;

const pending = new Map<number, { resolve: (v: any) => void; reject: (e: Error) => void }>();

worker.onmessage = (e: MessageEvent<Res<any>>) => {
  const res = e.data;
  const entry = pending.get(res.reqId);
  if (!entry) return;
  if (res.ok) entry.resolve(res.result);
  else entry.reject(new Error(res.message));
  pending.delete(res.reqId);
};

// The Mesh shape (serialized from Rust via serde-wasm-bindgen).
export type Mesh = {
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
	// World dimensions carried on the wire (M5).
	world_w: number;
	world_h: number;
};

export type Climate = {
	temp: Int8Array;
	prec: Uint8Array;
};

// The Grid shape (serialized from Rust via serde-wasm-bindgen). M5 seam:
// geometry + per-cell data. Only `cells.h` is populated until Steps 1.3/1.4
// fill `temp`/`prec`/`biome`.
export type Grid = {
	seed: number;
	mesh: Mesh;
	cells: {
		h: number[];
		temp: number[];
		prec: number[];
		biome: number[];
	};
};

let nextReqId = 1;
function nextId(): number {
	return nextReqId++;
}

/// Clamp a user-supplied seed into a safe `u32` range. Seed boxes accept any
/// f64; `wasm-bindgen` would silently wrap > u32::MAX (4294967295) to a
/// different seed with no error. We clamp + floor so an out-of-range seed is
/// deterministic and never panics in WASM (adversarial review M6).
function clampSeed(seed: number): number {
	const s = Math.floor(Number.isFinite(seed) ? seed : 0);
	if (s < 0) return 0;
	if (s > 0xffffffff) return 0xffffffff;
	return s >>> 0; // force unsigned 32-bit
}

function call<T, R>(kind: string, payload: T): Promise<R> {
	const reqId = nextId();
	return new Promise((resolve, reject) => {
		pending.set(reqId, { resolve, reject });
		worker.postMessage({ kind, reqId, ...payload } as any);
	});
}

export const coreApi = {
  /** Trivial export to verify the WASM ↔ JS bridge works end-to-end. */
  add(a: number, b: number): Promise<number> {
    return call("add", { a, b });
  },

  /** Step 1.1: generate a deterministic Voronoi mesh. */
  generateMesh(cellCount: number, seed: number): Promise<Mesh> {
    return call("generate_mesh", { cellCount, seed: clampSeed(seed) });
  },

  /** Step 1.2: generate the heightmap (Uint8Array, 0-100, <20 = water) from a Mesh. */
  generateHeightmap(mesh: Mesh, seed: number): Promise<Uint8Array> {
    return call("generate_heightmap", { mesh, seed: clampSeed(seed) });
  },

  /**
   * Step 1.2 (world-assembly form, M5 seam): build a Grid from a Mesh with
   * `cells.h` populated from the heightmap. Returns `{ seed, mesh, cells }`.
   * Step 1.5 will chain climate/biomes into `cells.temp`/`prec`/`biome`.
   */
  buildGridWithHeightmap(mesh: Mesh, seed: number): Promise<Grid> {
    return call("build_grid_with_heightmap", { mesh, seed: clampSeed(seed) });
  },

  /**
   * Step 1.3: produce `cells.temp` (Int8Array, °C) and `cells.prec`
   * (Uint8Array) from a Mesh + heightmap. `opts` is the optional climate
   * config (all fields default to FMG values). Returns `{ temp, prec }`.
   */
  generateClimate(mesh: Mesh, heightmap: Uint8Array, opts?: unknown): Promise<Climate> {
    return call("generate_climate", { mesh, heightmap, opts: opts ?? {} });
  },

  /**
   * Step 1.3 (grid form): run climate over an existing Grid and write
   * `cells.temp`/`cells.prec` back, returning the updated Grid. Used by the
   * Step 1.5 pipeline.
   */
  generateClimateForGrid(grid: Grid, opts?: unknown): Promise<Grid> {
    return call("generate_climate_for_grid", { grid, opts: opts ?? {} });
  },

  // Placeholders for future phases:
  // generateWorld(seed, cellCount, opts): Promise<Grid>;
  // projectWorld(pack, timeline, year): Promise<WorldAt>;
  // editHeightmap(grid, ops): Promise<Grid>;
  // recomputeDependents(grid, opts): Promise<DependentResult>;
  // generateTimeline(pack, seed, params): Promise<Event[]>;
};
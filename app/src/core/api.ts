// Typed wrapper over the core worker.
// Step 0.1: only `add(a,b)` exposed.
// Step 1.1: `generateMesh(cellCount, seed)` → Mesh.
// Later: generateWorld, projectWorld, editHeightmap, recomputeDependents, generateTimeline.

import CoreWorker from "../workers/core.worker.ts?worker";

type Res<T> = { reqId: number; ok: true; result: T } | { reqId: number; ok: false; message: string };

// The real worker is created lazily (on first `call`) so merely importing this
// module in a non-browser (test) environment does not eagerly construct a
// `Worker`. Unit tests inject a fake worker via `__setWorkerForTest` (below).
let worker: Worker | null = null;

const pending = new Map<number, { resolve: (v: any) => void; reject: (e: Error) => void }>();

function handleWorkerMessage(e: MessageEvent<Res<any>>) {
  const res = e.data;
  const entry = pending.get(res.reqId);
  if (!entry) return;
  if (res.ok) entry.resolve(res.result);
  else entry.reject(new Error(res.message));
  pending.delete(res.reqId);
}

function getWorker(): Worker {
  if (!worker) {
    worker = new CoreWorker() as unknown as Worker;
    worker.onmessage = handleWorkerMessage;
  }
  return worker;
}

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
// geometry + per-cell data. Only `build_grid_with_heightmap` (Step 1.2 form) returns a
// Grid with only `h` populated; `generate_climate_for_grid`/`generate_biomes_for_grid`
// fill `temp`/`prec`/`biome`. `generate_world` returns a fully-populated Grid.
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
export function clampSeed(seed: number): number {
	const s = Math.floor(Number.isFinite(seed) ? seed : 0);
	if (s < 0) return 0;
	if (s > 0xffffffff) return 0xffffffff;
	return s >>> 0; // force unsigned 32-bit
}

/// Clamp a user-supplied cell_count into a safe range. The MVP caps at 60k
/// (worldforge-technical-requirements.md); the Rust mesh clamps to [4, 1_000_000].
/// Clamping at the JS boundary prevents a negative/overlarge value from
/// coercing to u32::MAX and capacity-overflow-panicking the WASM module
/// (adversarial review Phase 1.5 C1).
export function clampCellCount(n: number): number {
	const v = Math.floor(Number.isFinite(n) ? n : 0);
	if (v < 1) return 4; // minimum sane mesh for spade
	if (v > 60_000) return 60_000; // MVP cap
	return v >>> 0;
}

function call<T, R>(kind: string, payload: T): Promise<R> {
	const reqId = nextId();
	return new Promise((resolve, reject) => {
		pending.set(reqId, { resolve, reject });
		getWorker().postMessage({ kind, reqId, ...payload } as any);
	});
}

/// Test-only hook: inject a fake worker (e.g. a `postMessage` spy + manual
/// `onmessage` invocation) so the bridge's request/response contract can be
/// unit-tested without a real Web Worker or the WASM module. Not part of the
/// app surface.
export function __setWorkerForTest(fake: Worker | null): void {
	if (worker && fake === null) {
		// Detach the real worker's listener so a disposed test worker can't
		// fire stray messages into the pending map.
		worker.onmessage = null;
	}
	worker = fake;
	// Attach the real message handler to the injected worker so it behaves
	// exactly like the lazily-created one (tests don't have to wire it).
	if (worker) {
		worker.onmessage = handleWorkerMessage;
	}
}

export const coreApi = {
  /** Trivial export to verify the WASM ↔ JS bridge works end-to-end. */
  add(a: number, b: number): Promise<number> {
    return call("add", { a, b });
  },

  /** Step 1.1: generate a deterministic Voronoi mesh. */
  generateMesh(cellCount: number, seed: number): Promise<Mesh> {
    return call("generate_mesh", { cellCount: clampCellCount(cellCount), seed: clampSeed(seed) });
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
   * Phase 2.5 heightmap editor's `recompute_dependents`.
   */
  generateClimateForGrid(grid: Grid, opts?: unknown): Promise<Grid> {
    return call("generate_climate_for_grid", { grid, opts: opts ?? {} });
  },

  /**
   * Step 1.4: produce `cells.biome` (Uint8Array, 0..=12, 0 = Marine) from a
   * Mesh + the climate `{ temp, prec }` + the heightmap. Returns one biome id
   * per cell.
   */
  generateBiomes(
    mesh: Mesh,
    climate: Climate,
    heightmap: Uint8Array,
  ): Promise<Uint8Array> {
    return call("generate_biomes", {
      mesh,
      climate: { temp: Array.from(climate.temp), prec: Array.from(climate.prec) },
      heightmap,
    }) as Promise<Uint8Array>;
  },

  /**
   * Step 1.4 (grid form): run biomes over an existing Grid and write
   * `cells.biome` back, returning the updated Grid. Used by the Phase 2.5
   * heightmap editor's `recompute_dependents`.
   */
  generateBiomesForGrid(grid: Grid): Promise<Grid> {
    return call("generate_biomes_for_grid", { grid }) as Promise<Grid>;
  },

  /**
   * Step 1.5: the static world generation pipeline.
   * Runs mesh → heightmap → climate → biomes in sequence and returns a fully
   * populated Grid (geometry + cells.h + cells.temp + cells.prec + cells.biome).
   */
  generateWorld(seed: number, cellCount: number, opts?: unknown): Promise<Grid> {
    return call("generate_world", { seed: clampSeed(seed), cellCount: clampCellCount(cellCount), opts: opts ?? {} }) as Promise<Grid>;
  },

  // Placeholders for future phases:
  // projectWorld(pack, timeline, year): Promise<WorldAt>;
  // editHeightmap(grid, ops): Promise<Grid>;
  // recomputeDependents(grid, opts): Promise<DependentResult>;
  // generateTimeline(pack, seed, params): Promise<Event[]>;
};
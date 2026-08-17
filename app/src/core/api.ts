// Typed wrapper over the core worker.
// Step 0.1: only `add(a,b)` exposed.
// Step 1.1: `generateMesh(cellCount, seed)` → Mesh.
// Later: generateWorld, projectWorld, editHeightmap, recomputeDependents, generateTimeline.

import CoreWorker from "../workers/core.worker.ts?worker";
import type {
	Army,
	Burg,
	Culture,
	Pack,
	Province,
	Religion,
	State,
} from "../state/types";
export type { Army, Burg, Culture, Pack, Province, Religion, State };

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
// Step 2.5.4: entity index arrays (`state`/`province`/`culture`/`religion`/`burg`)
// and drainage arrays (`fl`/`r`/`conf`) are now part of the wire type — the entity
// repair cascade mutates `state`/`province`/`burg` on land↔water flips.
export type Grid = {
	seed: number;
	mesh: Mesh;
	cells: {
		h: number[];
		temp: number[];
		prec: number[];
		biome: number[];
		/** Entity index arrays (Phase 3). -1 (or 0 for burg) == unassigned. */
		state: number[];
		province: number[];
		culture: number[];
		religion: number[];
		burg: number[];
		/** Drainage arrays (Step 2.5.3). fl = flux, r = river id, conf = confluence. */
		fl: number[];
		r: number[];
		conf: number[];
	};
};

/** Serde fix: thin patch returned by `editHeightmap` / `resetHeightmap` when
 * no explicit Grid is passed. The Rust side operates on its held Grid (no
 * JsValue round-trip) and returns only the mutated `h` array as a zero-copy
 * `Uint8Array`. The caller splices this into its local `cells.h` reference
 * instead of receiving the full 13.5MB Grid back. */
export type HeightmapPatch = {
	/** Full `cells.h` array (length N, 0-100, <20 = water) after the edit. */
	h: Uint8Array;
};

/** Step 2.5.3: river geometry (FMG `pack.rivers` entry, compute-core subset). */
export type RiverGeo = {
	id: number;
	source: number;
	mouth: number;
	discharge: number;
	cells: number[];
	points: [number, number][];
};

/** Step 2.5.3: lake geometry (FMG `pack.features` lake entry, compute-core subset). */
export type LakeGeo = {
	id: number;
	height: number;
	cells: number[];
	shoreline: number[];
	closed: boolean;
};

/** Step 2.5.3: full dependent recompute result. `removed_burgs` is a list of
 * burg NAMES (strings) for the warning toast — matches Rust `Vec<String>`.
 * `dissolved_states` is a list of state ids. `state`/`province`/`burg` are
 * post-repair entity index arrays (length N, `-1` = unassigned). `fl`/`r`/`conf`
 * are drainage arrays from `rivers::compute_drainage`. `coastline` is the
 * land-water boundary mask (1 = land cell adjacent to water). All are empty /
 * `-1`-filled until Phase 3 entities are generated. */
export type DependentResult = {
	temp: Int8Array | number[];
	prec: Uint8Array | number[];
	biome: Uint8Array | number[];
	state: Int32Array | number[];
	province: Int32Array | number[];
	culture: Int32Array | number[];
	religion: Int32Array | number[];
	burg: Int16Array | number[];
	fl: Uint16Array | number[];
	r: Uint16Array | number[];
	conf: Uint16Array | number[];
	coastline: Uint8Array | number[];
	removed_burgs: string[];
	dissolved_states: Uint32Array | number[];
	rivers: RiverGeo[];
	lakes: LakeGeo[];
};

/**
 * Step 3.2 result: `pack` (states + provinces + burgs) plus the per-cell
 * `state`/`province`/`burg` index arrays. The cell arrays mirror what gets
 * written into `grid.cells.*` and are returned separately so the worker can
 * splice them into its held Grid without re-serializing the whole Grid back
 * across the wire. `pack.cultures`/`pack.religions`/`pack.armies` are empty at
 * this stage (populated by `generateCulturesReligions`).
 */
export type StatesResult = {
	pack: Pack;
	cells_state: number[];
	cells_province: number[];
	cells_burg: number[];
};

/**
 * Step 3.3 result: culture + religion entity vectors plus the per-cell
 * `culture`/`religion` index arrays (mirroring `grid.cells.*`). The renderer
 * builds its data-texture atlases by indexing `cultures[i].color` /
 * `religions[i].color` against these cell arrays.
 */
export type CulturesResult = {
	cultures: Culture[];
	religions: Religion[];
	cells_culture: number[];
	cells_religion: number[];
};

/**
 * Step 2.5.5 (adversarial review Issue 7): shared 12-field dependent-splice.
 * Takes the current store Grid and a `DependentResult` (from
 * `recomputeDependents`), returns a NEW Grid with the recomputed arrays
 * spliced in. Uses `Array.from` to copy TypedArrays into regular `number[]`
 * (the Grid type contract) so React/zustand subscribers detect the change via
 * reference inequality. If a field is missing on `dep`, falls back to the
 * current store value (keeps the stale array rather than dropping the field).
 *
 * Used by both `HeightmapEditor` (brush stroke-end) and `CellInspector`
 * (per-cell edit dependent recompute) so a schema change is a one-line edit.
 */
export function spliceDependentResult(
	grid: Grid,
	dep: DependentResult,
): Grid {
	return {
		...grid,
		cells: {
			...grid.cells,
			temp: Array.from(dep.temp ?? grid.cells.temp),
			prec: Array.from(dep.prec ?? grid.cells.prec),
			biome: Array.from(dep.biome ?? grid.cells.biome),
			state: Array.from(dep.state ?? grid.cells.state),
			province: Array.from(dep.province ?? grid.cells.province),
			culture: Array.from(dep.culture ?? grid.cells.culture),
			religion: Array.from(dep.religion ?? grid.cells.religion),
			burg: Array.from(dep.burg ?? grid.cells.burg),
			fl: Array.from(dep.fl ?? grid.cells.fl),
			r: Array.from(dep.r ?? grid.cells.r),
			conf: Array.from(dep.conf ?? grid.cells.conf),
		},
	};
}

let nextReqId = 1;
function nextId(): number {
	return nextReqId++;
}

/// Clamp a user-supplied seed into a safe `u32` range. Seed boxes accept any
/// f64; `wasm-bindgen` would silently wrap > u32::MAX (4294967295) to a
/// different seed with no error. We clamp + floor so an out-of-range seed is
/// deterministic and never panics in WASM (adversarial review M6).
// ---------------------------------------------------------------------------//
// Phase 4.1: timeline data model + projection types (mirror Rust timeline.rs).
// These cross the WASM boundary via serde-wasm-bindgen — field names must match
// the Rust structs exactly.
// ---------------------------------------------------------------------------//

export type EventKind =
	| "Found"
	| "Conquer"
	| "Disband"
	| "Raise"
	| "Plague"
	| "GoldenAge"
	| "Schism"
	| "Secession"
	| "Dissolve";

export type EntityType = "State" | "Province" | "Culture" | "Religion" | "Burg";

export type EventPayload =
	| { kind: "none" }
	| { kind: "found"; cells: number[] }
	| { kind: "conquer"; from_state: number; to_state: number; cells: number[] }
	| { kind: "disband"; army: number }
	| { kind: "raise"; army: number }
	| { kind: "plague"; target_state: number; mortality: number }
	| { kind: "golden_age"; target_state: number; decade: number }
	| { kind: "schism"; parent_religion: number; child_religion: number }
	| { kind: "secession"; cells: number[] };

export type TimelineEvent = {
	id: number; // u64
	year: number;
	entity_id: number;
	entity_type: EntityType;
	kind: EventKind;
	payload: EventPayload;
	narrative: string | null;
};

export type Timeline = TimelineEvent[];

/** Phase 4.1: the projected world state at year Y (design §3.4 `WorldAt(Y)`).
 * `cells_*` arrays are `u32` per cell (`0` = unassigned), ready for the
 * renderer's data-texture upload. `pack` carries the entity snapshots with
 * pop-scalar overrides and dissolved flags applied. */
export type WorldAt = {
	year: number;
	cells_state: number[];
	cells_culture: number[];
	cells_religion: number[];
	cells_burg: number[];
	pack: Pack;
};

/** Phase 4.2: parameters for deterministic timeline generation. All fields
 * are optional — omitted fields use deterministic engine defaults. */
export type TimelineParams = {
	eraStart?: number;
	eraEnd?: number;
	foundingRate?: number;
	warRate?: number;
	plagueProbability?: number;
	schismProbability?: number;
	migrationRate?: number;
	successionRate?: number;
	goldenAgeProbability?: number;
};

export function clampSeed(seed: number): number {
	const s = Math.floor(Number.isFinite(seed) ? seed : 0);
	if (s < 0) return 0;
	if (s > 0xffffffff) return 0xffffffff;
	return s >>> 0; // force unsigned 32-bit
}

/// Clamp a user-supplied cell_count into a safe range. The MVP caps at 60k
/// (worldgen-technical-requirements.md); the Rust mesh clamps to [4, 1_000_000].
/// Clamping at the JS boundary prevents a negative/overlarge value from
/// coercing to u32::MAX and capacity-overflow-panicking the WASM module
/// (adversarial review Phase 1.5 C1).
export function clampCellCount(n: number): number {
	const v = Math.floor(Number.isFinite(n) ? n : 0);
	if (v < 1) return 4; // minimum sane mesh for spade
	if (v > 60_000) return 60_000; // MVP cap
	return v >>> 0;
}

/// Clamp a user-supplied culture count into a safe range. The Rust generator
/// bounds it by available land cells, but we clamp at the JS boundary to
/// prevent u32::MAX coercion (pitfall #1: every u32 crossing JS↔WASM needs a
/// clamp — adversarial review Phase 3.3 L6).
export function clampCultureCount(n: number): number {
	const v = Math.floor(Number.isFinite(n) ? n : 0);
	if (v < 0) return 0;
	if (v > 60_000) return 60_000; // cannot exceed cell count
	return v >>> 0;
}

/// Clamp a user-supplied religion count into a safe range. The Rust generator
/// bounds it by available burgs/cultures; we clamp at the JS boundary for the
/// same u32::MAX defense (pitfall #1).
export function clampReligionCount(n: number): number {
	const v = Math.floor(Number.isFinite(n) ? n : 0);
	if (v < 0) return 0;
	if (v > 10_000) return 10_000; // practical upper bound
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

  /**
   * Step 2.5.1: apply a batch of heightmap edit ops (brush + macro tools) to
   * `grid.cells.h` in place (in the worker). Deterministic: same grid + same
   * ops yields byte-identical `h`.
   *
   * Step 2.5.4 + serde fix: if `grid` is omitted, the worker uses its held
   * grid handle (set by `generateWorld` / `storeGrid`). In this case the Rust
   * side operates on a Rust-held Grid (no JsValue round-trip at all) and
   * returns only the mutated `h` array as a `Uint8Array` patch — the caller
   * can splice it into its local mesh/`cells.h` reference without receiving
   * the full 13.5MB Grid back. If `grid` IS passed (backward compat / loaded
   * grid), the full Grid is returned as before.
   */
  editHeightmap(ops: EditOp[], grid?: Grid): Promise<Grid | HeightmapPatch> {
    return call("edit_heightmap", grid ? { grid, ops } : { ops }) as Promise<Grid | HeightmapPatch>;
  },

  /**
   * Step 2.5.2: Tier-1 local recompute of temp + biome for an affected cell
   * set (the brush-radius cells). Runs `recompute_temp_local` (altitude lapse)
   * then `recompute_biome_local` (h/temp/prec + neighbor mean) in the worker,
   * returning `{ temp: Int8Array, biome: Uint8Array }` holding ONLY the
   * requested cells' values (in cellIds order) so the renderer can patch just
   * those texels during a brush drag without a full texture re-upload. Both
   * are pure functions → deterministic.
   *
   * Step 2.5.4: if `grid` is omitted, the worker uses its held grid handle,
   * avoiding the serde round-trip on the hot drag path.
   */
  recomputeTempBiomeLocal(
    cellIds: number[],
    opts?: unknown,
    grid?: Grid,
  ): Promise<{ temp: Int8Array; biome: Uint8Array }> {
    return call("recompute_temp_biome_local", grid ? { grid, cellIds, opts: opts ?? {} } : { cellIds, opts: opts ?? {} }) as Promise<{ temp: Int8Array; biome: Uint8Array }>;
  },

  /**
   * Step 2.5.3: full debounced dependent recompute after a heightmap edit
   * stroke. Runs drainage (rivers + lakes + flux), climate (temp + prec), and
   * biome full-pass on the edited grid, returning a `DependentResult` with the
   * fresh `temp`/`prec`/`biome` arrays plus `rivers` + `lakes` geometry. The
   * renderer swaps data textures from this; entity repair fills
   * `removed_burgs`/`dissolved_states` (empty for now — no Burgs/States yet).
   *
   * This is the debounced counterpart to `recomputeTempBiomeLocal`: the local
   * patch runs on every pointermove; this runs once after the stroke ends (or
   * after a >=300ms idle window) to reconcile precipitation, biomes, and
   * drainage that the local patch can't reach.
   *
   * Step 2.5.4: if `grid` is omitted, the worker uses its held grid handle.
   */
  recomputeDependents(
    opts?: unknown,
    grid?: Grid,
  ): Promise<DependentResult> {
    return call("recompute_dependents", grid ? { grid, opts: opts ?? {} } : { opts: opts ?? {} }) as Promise<DependentResult>;
  },

  /**
   * Step 2.5.4: pick the nearest cell to world-space `(x, y)`. Uses the
   * `cells.spacing` spatial grid + neighbor refinement. Returns the cell id
   * (number >= 0) or -1 if the grid has no cells. O(1)-ish, deterministic.
   *
   * If `grid` is omitted, the worker uses its held grid handle.
   */
  pickCell(x: number, y: number, grid?: Grid): Promise<number> {
    return call("pick_cell", grid ? { grid, x, y } : { x, y }) as Promise<number>;
  },

  /**
   * Step 2.5.4: reset `grid.cells.h` to the original seeded heightmap,
   * discarding all edits. Also reinitializes entity index arrays to
   * "unassigned".
   *
   * Serde fix: if `grid` is omitted, the worker uses its held grid handle
   * and returns only the reset `h` array as a `HeightmapPatch` (zero-copy
   * `Uint8Array`). If passed, returns the full updated Grid.
   */
  resetHeightmap(grid?: Grid): Promise<Grid | HeightmapPatch> {
    return call("reset_heightmap", grid ? { grid } : {}) as Promise<Grid | HeightmapPatch>;
  },

  /**
   * Step 2.5.4: push a Grid into the worker's held grid handle. The editor
   * hot path (`editHeightmap`/`recomputeTempBiomeLocal`/`pickCell`/...) then
   * omits the Grid from the wire payload and the worker uses this held copy,
   * avoiding the ~28ms serde round-trip per pointermove. `generateWorld`
   * auto-stores its result; call this only to sync a grid the main thread
   * holds independently (e.g. after a load).
   */
  storeGrid(grid: Grid): Promise<null> {
    return call("store_grid", { grid }) as Promise<null>;
  },

  /**
   * Step 2.5.6: fetch river + lake geometry for the held grid (the
   * initial-load counterpart to `recomputeDependents`). `generateWorld`
   * populates `cells.r` but NOT the `RiverGeo`/`LakeGeo` polylines/polygons;
   * this runs `compute_drainage` on the held grid and returns just the
   * geometry the renderer draws. On heightmap edits, `recomputeDependents`
   * carries the same geometry in its `DependentResult`.
   *
   * Always operates on the held grid (no Grid arg). Returns
   * `{ rivers: RiverGeo[], lakes: LakeGeo[] }`.
   */
  getDrainageGeometry(): Promise<{ rivers: RiverGeo[]; lakes: LakeGeo[] }> {
    return call("get_drainage_geometry", {}) as Promise<{
      rivers: RiverGeo[];
      lakes: LakeGeo[];
    }>;
  },

  /**
   * Step 3.2: generate states, provinces, and burgs for an existing Grid
   * (post-`generateWorld`). Returns a `StatesResult` with the `pack` entity
   * vectors and per-cell `state`/`province`/`burg` arrays. The worker also
   * splices the cell arrays into its held Grid so subsequent
   * `generateCulturesReligions` / `pickCell` calls see them.
   *
   * `count` is clamped at the JS boundary (pitfall #1: every u32 crossing
   * JS↔WASM needs a clamp) to prevent u32::MAX coercion from a stray NaN/
   * Infinity from a UI control.
   */
  generateStates(grid: Grid, seed: number, count: number): Promise<StatesResult> {
    return call("generate_states", {
      grid,
      seed: clampSeed(seed),
      count: clampCellCount(count),
    }) as Promise<StatesResult>;
  },

  /**
   * Step 3.3: generate cultures and religions for a Grid that already has
   * states + burgs (from `generateStates`). Returns a `CulturesResult` with
   * the culture/religion entity vectors and per-cell arrays. The worker
   * splices the cell arrays into its held Grid.
   *
   * `cultureCount` / `religionCount` are clamped at the JS boundary via
   * `clampCultureCount` / `clampReligionCount` (pitfall #1). `statesResult`
   * MUST be the full `StatesResult` returned by `generateStates` — the Rust
   * side deserializes it to read `pack.burgs`/`pack.states` for culture
   * origins and religion schism parents.
   */
  generateCulturesReligions(
    grid: Grid,
    seed: number,
    cultureCount: number,
    religionCount: number,
    statesResult: StatesResult,
  ): Promise<CulturesResult> {
    return call("generate_cultures_religions", {
      grid,
      seed: clampSeed(seed),
      cultureCount: clampCultureCount(cultureCount),
      religionCount: clampReligionCount(religionCount),
      statesResult,
    }) as Promise<CulturesResult>;
  },

  /**
   * Phase 4.1: full timeline projection — computes `WorldAt(target_year)` from
   * the base `Pack` + year-0 cell arrays + timeline. O(events ≤ Y), allocates
   * a fresh WorldAt. Use this for the initial scrub target or backward jumps.
   *
   * `cells_state`/`cells_culture`/`cells_religion` use the `i32` convention
   * (`-1` = unassigned); `cells_burg` uses `i16` (`0` = none). The WASM layer
   * normalizes them to `u32` internally.
   *
   * `target_year` is clamped to `i32` range to avoid silent wraparound.
   */
  projectWorld(
    pack: Pack,
    cells_state: number[] | Int32Array,
    cells_culture: number[] | Int32Array,
    cells_religion: number[] | Int32Array,
    cells_burg: number[] | Int16Array,
    timeline: Timeline,
    target_year: number,
  ): Promise<WorldAt> {
    return call("project_world", {
      pack,
      cells_state: new Int32Array(cells_state),
      cells_culture: new Int32Array(cells_culture),
      cells_religion: new Int32Array(cells_religion),
      cells_burg: new Int16Array(cells_burg),
      timeline,
      target_year: Math.floor(target_year),
    }) as Promise<WorldAt>;
  },

  /**
   * Phase 4.1: incremental forward scrubbing (prev_year → target_year).
   * Applies only events in `(prev_year, target_year]` to a `WorldAt`, avoiding
   * the full re-projection. **Backward jumps are a no-op** — the caller must
   * call `projectWorld` to re-project from base when scrubbing backward.
   *
   * The `world` argument is the previous projection (from the last
   * `projectWorld` or `projectDelta` call on the same timeline).
   */
  projectDelta(
    world: WorldAt,
    timeline: Timeline,
    prev_year: number,
    target_year: number,
  ): Promise<WorldAt> {
    return call("project_delta", {
      world,
      timeline,
      prev_year: Math.floor(prev_year),
      target_year: Math.floor(target_year),
    }) as Promise<WorldAt>;
  },

  /**
   * Phase 4.1 do #7: checkpoint-based scrubbing. The worker decides whether
   * to full-reproject (project_world from base) or delta-apply (project_delta
   * from its cached heldWorld), transparently handling checkpoint intervals.
   *
   * For forward scrubbing within the checkpoint interval, the worker uses the
   * cached WorldAt delta. For backward scrubbing or crossing a checkpoint
   * boundary, it reprojects from base. The caller just provides the from/to
   * years and the base Pack + cell arrays (needed for full reprojection).
   *
   * `from_year` is the year the caller's `world` was projected at (0 if
   * this is the first scrub). `target_year` is the destination.
   *
   * `world` is optional on first call (when heldWorld is null on the worker);
   * the worker will full-reproject via project_world.
   */
  scrubWorld(
    pack: Pack,
    cells_state: number[] | Int32Array,
    cells_culture: number[] | Int32Array,
    cells_religion: number[] | Int32Array,
    cells_burg: number[] | Int16Array,
    timeline: Timeline,
    from_year: number,
    target_year: number,
    world?: WorldAt,
  ): Promise<WorldAt> {
    return call("scrub_world", {
      pack,
      cells_state: new Int32Array(cells_state),
      cells_culture: new Int32Array(cells_culture),
      cells_religion: new Int32Array(cells_religion),
      cells_burg: new Int16Array(cells_burg),
      timeline,
      from_year: Math.floor(from_year),
      target_year: Math.floor(target_year),
      world,
    }) as Promise<WorldAt>;
  },
  /**
   * Phase 4.2: generate a deterministic timeline from a base Pack + year-0 cell
   * arrays + era bounds + seed. Returns a sorted Timeline.
   */
  generateTimeline(
    pack: Pack,
    cells_state: number[] | Int32Array,
    cells_culture: number[] | Int32Array,
    cells_religion: number[] | Int32Array,
    cells_burg: number[] | Int16Array,
    cells_h: number[] | Uint8Array,
    seed: number,
    era_start: number,
    era_end: number,
    params?: TimelineParams,
  ): Promise<Timeline> {
    return call("generate_timeline", {
      pack,
      cells_state: new Int32Array(cells_state),
      cells_culture: new Int32Array(cells_culture),
      cells_religion: new Int32Array(cells_religion),
      cells_burg: new Int16Array(cells_burg),
      cells_h: new Uint8Array(cells_h),
      seed,
      era_start: Math.floor(era_start),
      era_end: Math.floor(era_end),
      params: params ?? {},
    }) as Promise<Timeline>;
  },
};

/**
 * Step 2.5.1: an edit operation. Brush modes (Raise/Lower/Flatten/Smooth) use
 * `center_cell` + `radius` + `strength`; `cells` is the pre-gathered radius
 * set (empty = gather at runtime). Macro modes use `cells` as the ordered path
 * and `strength` as a multiplier/offset. `target_cell` is used by Range/Trough
 * as the ridge walk endpoint (FMG `addRange`/`addTrough`).
 */
export type EditMode =
  | "Raise" | "Lower" | "Flatten" | "Smooth"
  | "Range" | "Trough" | "Strait" | "Mask" | "Invert" | "Add" | "Multiply";

export type EditOp = {
  mode: EditMode;
  center_cell: number;
  target_cell: number;
  radius: number;
  strength: number;
  cells: number[];
};
/* tslint:disable */
/* eslint-disable */

/**
 * Trivial export to verify the WASM ↔ JS bridge works end-to-end.
 * Returns `a + b`. Used by Step 0.1 verification (`add(2, 3) === 5`).
 */
export function add(a: number, b: number): number;

/**
 * Step 1.2 (world-assembly form): build a `Grid` from a deserialized `Mesh`
 * and store the generated heightmap into `grid.cells.h`. Returns a `Grid`
 * with only `cells.h` populated (the other `CellData` fields are zeroed).
 *
 * **Note:** `generate_world` (Step 1.5) does NOT call this — it inlines the
 * sub-step logic to avoid the extra `Grid` serde round-trips. This entry is
 * kept for the Phase 2.5 heightmap editor's `recompute_dependents` path.
 * Exposed as `build_grid_with_heightmap(mesh, seed)` to JS.
 */
export function build_grid_with_heightmap(mesh_js: any, seed: number): any;

/**
 * Step 2.5.1: apply a batch of heightmap edit ops (brush + macro tools) to
 * `grid.cells.h` in place. Deterministic: same `grid` + same `ops` yields
 * byte-identical `h`. Exposed as `edit_heightmap(grid, ops)` to JS.
 */
export function edit_heightmap(grid_js: any, ops_js: any): any;

/**
 * Edit the heightmap on the Rust-side held grid. No Grid serde
 * round-trip. Returns only the updated `cells.h` as a `Uint8Array` (zero-copy
 * view into WASM memory). The held grid is mutated in place; JS should update
 * its `heldGrid.cells.h` from the returned array (or just use the array
 * directly for the texture upload).
 *
 * Exposed as `edit_heightmap_h(ops)` to JS.
 */
export function edit_heightmap_h(ops_js: any): Uint8Array;

/**
 * Step 1.4: produce `cells.biome` (Uint8Array, `0..=12`, `0` = Marine/water)
 * from a deserialized `Mesh` + the climate `{ temp, prec }` + the heightmap
 * `cells.h` (Uint8Array, 0..=100, `< 20` == water). Port of FMG
 * `biomes-generator.ts` (`BiomesGenerator.define`/`getId`) adapted to the
 * irregular Voronoi mesh. Returns a `Uint8Array` of one biome id per cell.
 */
export function generate_biomes(mesh_js: any, climate_js: any, heightmap: Uint8Array): Uint8Array;

/**
 * Step 1.4 (grid form): run the biome pipeline over an already-built `Grid`
 * (which carries the mesh, `cells.h`, `cells.temp`, `cells.prec`) and write
 * `cells.biome` back into the same `Grid`, returning the updated `Grid` as
 * `JsValue`.
 *
 * **Note:** `generate_world` (Step 1.5) does NOT call this — it inlines the
 * biome step. This entry is kept for the Phase 2.5 heightmap editor's
 * `recompute_dependents` path, which will call it on an edited `Grid`.
 */
export function generate_biomes_for_grid(grid_js: any): any;

/**
 * Step 1.3: produce `cells.temp` (Int8Array, °C) and `cells.prec` (Uint8Array)
 * from a deserialized `Mesh` plus the heightmap `cells.h` (Uint8Array, 0..=100,
 * `< 20` == water). Climate options are passed as a `JsValue` object whose
 * fields are all optional (defaults mirror FMG). Returns
 * `{ temp: Int8Array, prec: Uint8Array }`. Port of FMG `calculateTemperatures`
 * + `generatePrecipitation` (see `climate.rs`).
 */
export function generate_climate(mesh_js: any, heightmap: Uint8Array, opts_js: any): any;

/**
 * Step 1.3 (grid form): run the climate pipeline over an already-built `Grid`
 * (which carries both the mesh and `cells.h`) and write `cells.temp` /
 * `cells.prec` back into the same `Grid`, returning the updated `Grid` as
 * `JsValue`.
 *
 * **Note:** `generate_world` (Step 1.5) does NOT call this — it inlines the
 * climate step. This entry is kept for the Phase 2.5 heightmap editor's
 * `recompute_dependents` path, which will call it incrementally on an
 * edited `Grid` without re-running the full pipeline.
 */
export function generate_climate_for_grid(grid_js: any, opts_js: any): any;

/**
 * Phase 3 Step 3.3 — Generate cultures + religions for a grid that already
 * has states + burgs (from `generate_states`). Returns a `CulturesResult`
 * with culture/religion entity vectors and per-cell culture/religion arrays.
 */
export function generate_cultures_religions(grid_js: any, seed: number, culture_count: number, religion_count: number, states_result_js: any): any;

/**
 * Step 1.2: generate the heightmap `cells.h` (Uint8Array, `0..=100`,
 * `< 20` == water) from a deserialized `Mesh`. Seeded blob/pit/range/trough
 * floods ported from FMG's `heightmap-generator.ts`. Exposed as
 * `generate_heightmap(mesh, seed)` to JS.
 */
export function generate_heightmap(mesh_js: any, seed: number): Uint8Array;

/**
 * Step 1.1: generate a deterministic Voronoi mesh from `cell_count` seeded
 * points. Returns a `JsValue` with fields `{ points, cells, vertices }`
 * matching the wire format defined in `mesh::Mesh`.
 */
export function generate_mesh(cell_count: number, seed: number): any;

/**
 * Phase 3 Step 3.2: generate states, provinces, and burgs for a fully-built
 * `Grid` (mesh + heightmap + climate + biomes + drainage). Returns a
 * `StatesResult` carrying the `Pack` + per-cell index arrays
 * (`cells_state`, `cells_province`, `cells_burg`). JS splices the cell arrays
 * into its `grid.cells` and stores the `Pack` separately for the Phase 4
 * timeline projector.
 *
 * `seed` should match the grid's seed for consistency. `count` is the
 * requested number of states (capitals); actual count may be lower if too
 * few suitable land cells exist.
 */
export function generate_states(grid_js: any, seed: number, count: number): any;

/**
 * Runs mesh → heightmap → climate → biomes in sequence and returns a fully
 * populated `Grid` (geometry + cells.h + cells.temp + cells.prec + cells.biome).
 * This is the single entry point the browser/worker calls for a complete world.
 *
 * - `seed`: u32, the world seed (clamped to u32::MAX at the JS boundary).
 * - `cell_count`: u32, target cell count for the Voronoi mesh.
 * - `opts_js`: optional `ClimateOpts` object (all fields optional, defaults mirror FMG).
 * Returns the `Grid` serialized as `JsValue` via `serde_wasm_bindgen`.
 *
 * Also stores the grid into the Rust-side handle (`HELD_GRID`) so
 * subsequent `_h` calls can operate without serde round-trips.
 */
export function generate_world(seed: number, cell_count: number, opts_js: any): any;

/**
 * Step 2.5.6: compute river + lake geometry from the held Grid and return it
 * as a serde-encoded `{ rivers: RiverGeo[], lakes: LakeGeo[] }` object.
 *
 * `generate_world` populates `cells.r`/`fl`/`conf` (the per-cell arrays) so
 * downstream generators (biome moisture's river-flux bonus, Phase 3
 * entities) can read them, but it does NOT export the
 * [`grid::RiverGeo`]/[`grid::LakeGeo`] polyline/polygon geometry. This call
 * runs `rivers::compute_drainage` on the held grid (cheap: ~13ms at 60k) and
 * returns just the geometry the renderer needs to draw rivers + lakes on a
 * fresh world. `recompute_dependents` returns the same geometry inside its
 * `DependentResult` (alongside the climate/biome arrays); this call is the
 * initial-load counterpart.
 *
 * Also assigns sequential 1-based lake ids for renderer stability (mirrors
 * `recompute_dependents_inner`).
 *
 * Exposed as `get_drainage_geometry_h()` to JS.
 */
export function get_drainage_geometry_h(): any;

/**
 * Check whether the Rust side is currently holding a grid.
 */
export function has_grid_h(): boolean;

/**
 * Initialize the panic hook so Rust panics surface in the browser console
 * instead of silently failing. Called once on startup.
 */
export function init(): void;

/**
 * Step 2.5.4: pick the nearest cell to world-space `(x, y)`. Uses the
 * `cells.spacing` spatial grid + neighbor refinement. Returns the cell id
 * as a `u32`, or `-1` if the grid has no cells. O(1)-ish, deterministic.
 *
 * Exposed as `pick_cell(grid, x, y)` to JS.
 */
export function pick_cell(grid_js: any, x: number, y: number): number;

/**
 * Edit the heightmap on the Rust-side held grid. No Grid serde.
 *
 * Exposed as `pick_cell_h(x, y)` to JS.
 */
export function pick_cell_h(x: number, y: number): number;

/**
 * Phase 4.1: incremental forward scrubbing. Applies only the events in
 * `(prev_year, target_year]` to a `WorldAt`, mutating it in place and
 * returning the updated `WorldAt` (serialized via serde).
 *
 * **Backward jumps** (`target_year <= prev_year`) are a no-op on cell arrays
 * — the caller must call `project_world` to re-project from base for those.
 * This fn only bumps `world.year` on a backward target.
 */
export function project_delta(world_js: any, timeline_js: any, prev_year: number, target_year: number): any;

/**
 * Phase 4.1: project `WorldAt(target_year)` from a base `Pack` + year-0 cell
 * arrays + `timeline`. This is O(events ≤ Y) and allocates a fresh `WorldAt`.
 *
 * `pack_js`, `cells_state`, `cells_culture`, `cells_religion`,
 * `cells_burg`, and `timeline` are all deserialized from JsValue. The cell
 * arrays use the `i32` (`-1` = unassigned) and `i16` (`0` = none) conventions;
 * this fn normalizes them to the `u32` (`0` = unassigned) form `WorldAt`
 * returns to JS.
 */
export function project_world(pack_js: any, cells_state: Int32Array, cells_culture: Int32Array, cells_religion: Int32Array, cells_burg: Int16Array, timeline_js: any, target_year: number): any;

/**
 * Step 2.5.3: full dependent recompute after a heightmap edit stroke.
 *
 * Runs the complete drainage → climate → biome → entity-repair cascade on an
 * edited `Grid` and returns a [`grid::DependentResult`] carrying the freshly
 * recomputed `temp`/`prec`/`biome` arrays plus the new river + lake geometry.
 * The renderer swaps data textures from this; the entity repair cascade fills
 * `removed_burgs`/`dissolved_states` for the warning toast (Phase 3 — arrays
 * are empty for now since no Burgs/States have been generated yet).
 *
 * This is the debounced counterpart to `recompute_temp_biome_local`: the local
 * patch runs on every pointermove (instant feedback), this runs once after the
 * stroke ends (or after a ≥300ms idle window) to reconcile the diverged
 * precipitation, biomes, and drainage that the local patch cannot reach.
 *
 * Determinism: a pure function of `(grid, opts)` — byte-identical across runs.
 *
 * Exposed as `recompute_dependents(grid, opts)` to JS.
 */
export function recompute_dependents(grid_js: any, opts_js: any): any;

/**
 * Edit the heightmap on the Rust-side held grid. No inbound Grid
 * serde. The outbound `DependentResult` is still serialized (it carries the
 * recomputed arrays + river/lake geometry the renderer needs) — will
 * replace this with TypedArray encoding.
 *
 * Exposed as `recompute_dependents_h(opts)` to JS.
 */
export function recompute_dependents_h(opts_js: any): any;

/**
 * Track B: zero-copy DependentResult return. Same as `recompute_dependents_h`
 * but returns the 12 numeric arrays as TypedArrays (zero-copy views into WASM
 * linear memory via `js_sys::*Array::from(&slice)`) instead of serde-encoding
 * them as JS Arrays of boxed Numbers. The 4 small collections (`removed_burgs`,
 * `dissolved_states`, `rivers`, `lakes`) are still serde-encoded (they are
 * tiny relative to the 60k-element numeric arrays). This eliminates ~385ms of
 * serde overhead at 60k cells.
 *
 * Returns a JS object:
 * ```text
 * { temp: Int8Array, prec: Uint8Array, biome: Uint8Array,
 *   state: Int32Array, province: Int32Array, culture: Int32Array,
 *   religion: Int32Array, burg: Int16Array,
 *   fl: Uint16Array, r: Uint16Array, conf: Uint16Array,
 *   coastline: Uint8Array,
 *   removed_burgs: string[], dissolved_states: Uint32Array,
 *   rivers: RiverGeo[], lakes: LakeGeo[] }
 * ```
 *
 * Exposed as `recompute_dependents_h2(opts)` to JS.
 */
export function recompute_dependents_h2(opts_js: any): any;

/**
 * Step 2.5.2: Tier-1 local recompute of temp + biome for an affected cell
 * set. Runs `recompute_temp_local` then `recompute_biome_local` in place on
 * `grid.cells`, and returns `{ temp: Int8Array, biome: Uint8Array }` holding
 * ONLY the values for the requested `cellIds` (in the same order), so the
 * renderer can patch just those texels during a brush drag without a full
 * texture re-upload. Temp uses altitude lapse; biome uses h/temp/prec +
 * land-neighbor mean. Both are pure functions → deterministic.
 *
 * Exposed as `recompute_temp_biome_local(grid, cellIds, opts)` to JS.
 */
export function recompute_temp_biome_local(grid_js: any, cell_ids_js: any, opts_js: any): any;

/**
 * Edit the heightmap on the Rust-side held grid. No Grid serde.
 * Returns only the affected cells' temp (Int8Array) + biome (Uint8Array).
 *
 * Exposed as `recompute_temp_biome_local_h(cellIds, opts)` to JS.
 */
export function recompute_temp_biome_local_h(cell_ids_js: any, opts_js: any): any;

/**
 * Release the held grid (drops it). Called when the worker is done with a
 * world or before loading a new one.
 */
export function release_grid_h(): void;

/**
 * Step 2.5.4: reset `grid.cells.h` back to the original seeded heightmap.
 * Regenerates `h` from `grid.seed` + `grid.mesh` using the same
 * `heightmap::generate` used by `generate_world`. Also reinitializes the
 * entity index arrays (`state`/`province`/`culture`/`religion`/`burg`) to
 * their "unassigned" sentinels, since Reset means "discard all edits".
 * Returns the updated `Grid` as `JsValue`.
 *
 * Exposed as `reset_heightmap(grid)` to JS.
 */
export function reset_heightmap(grid_js: any): any;

/**
 * Edit the heightmap on the Rust-side held grid. No Grid serde.
 * Returns only the new `cells.h` as a `Uint8Array`.
 *
 * Exposed as `reset_heightmap_h()` to JS.
 */
export function reset_heightmap_h(): Uint8Array;

/**
 * Store a Grid (deserialized from JS) into the Rust-side handle slot.
 * Replaces any previously held grid. The held grid is owned by Rust after
 * this call — JS should not mutate its copy.
 */
export function store_grid_h(grid_js: any): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly add: (a: number, b: number) => number;
    readonly build_grid_with_heightmap: (a: any, b: number) => any;
    readonly edit_heightmap: (a: any, b: any) => any;
    readonly edit_heightmap_h: (a: any) => any;
    readonly generate_biomes: (a: any, b: any, c: any) => any;
    readonly generate_biomes_for_grid: (a: any) => any;
    readonly generate_climate: (a: any, b: any, c: any) => any;
    readonly generate_climate_for_grid: (a: any, b: any) => any;
    readonly generate_cultures_religions: (a: any, b: number, c: number, d: number, e: any) => any;
    readonly generate_heightmap: (a: any, b: number) => any;
    readonly generate_mesh: (a: number, b: number) => any;
    readonly generate_states: (a: any, b: number, c: number) => any;
    readonly generate_world: (a: number, b: number, c: any) => any;
    readonly get_drainage_geometry_h: () => any;
    readonly has_grid_h: () => number;
    readonly init: () => void;
    readonly pick_cell: (a: any, b: number, c: number) => number;
    readonly pick_cell_h: (a: number, b: number) => number;
    readonly project_delta: (a: any, b: any, c: number, d: number) => any;
    readonly project_world: (a: any, b: any, c: any, d: any, e: any, f: any, g: number) => any;
    readonly recompute_dependents: (a: any, b: any) => any;
    readonly recompute_dependents_h: (a: any) => any;
    readonly recompute_dependents_h2: (a: any) => any;
    readonly recompute_temp_biome_local: (a: any, b: any, c: any) => any;
    readonly recompute_temp_biome_local_h: (a: any, b: any) => any;
    readonly release_grid_h: () => void;
    readonly reset_heightmap: (a: any) => any;
    readonly reset_heightmap_h: () => any;
    readonly store_grid_h: (a: any) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;

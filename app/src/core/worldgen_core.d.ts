/* tslint:disable */
/* eslint-disable */

/**
 * Trivial export to verify the WASM ↔ JS bridge works end-to-end.
 * Returns `a + b`. Used by Step 0.1 verification (`add(2, 3) === 5`).
 */
export function add(a: number, b: number): number;

/**
 * Step 1.2 (world-assembly form): build a `Grid` from a deserialized `Mesh`
 * and store the generated heightmap into `grid.cells.h`. This is the shape
 * Step 1.5 (`generate_world`) will chain: mesh → heightmap → climate → biomes,
 * each writing into the same `CellData` (adversarial review M5). Returns the
 * `Grid` serialized as `JsValue` (just the geometry + `h` for now; the other
 * `CellData` fields are zeroed until Steps 1.3/1.4 land). Exposed as
 * `build_grid_with_heightmap(mesh, seed)` to JS for the Step 1.5 pipeline.
 */
export function build_grid_with_heightmap(mesh_js: any, seed: number): any;

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
 * `JsValue`. This is the form Step 1.5 (`generate_world`) will call to chain
 * 1.1→1.4 into one `Grid`.
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
 * `JsValue`. This is the form Step 1.5 (`generate_world`) will call to chain
 * 1.1→1.4 into one `Grid`.
 */
export function generate_climate_for_grid(grid_js: any, opts_js: any): any;

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
 * Initialize the panic hook so Rust panics surface in the browser console
 * instead of silently failing. Called once on startup.
 */
export function init(): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly add: (a: number, b: number) => number;
    readonly build_grid_with_heightmap: (a: any, b: number) => any;
    readonly generate_biomes: (a: any, b: any, c: any) => any;
    readonly generate_biomes_for_grid: (a: any) => any;
    readonly generate_climate: (a: any, b: any, c: any) => any;
    readonly generate_climate_for_grid: (a: any, b: any) => any;
    readonly generate_heightmap: (a: any, b: number) => any;
    readonly generate_mesh: (a: number, b: number) => any;
    readonly init: () => void;
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

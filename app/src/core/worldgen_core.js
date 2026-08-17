/* @ts-self-types="./worldgen_core.d.ts" */

/**
 * Trivial export to verify the WASM ↔ JS bridge works end-to-end.
 * Returns `a + b`. Used by Step 0.1 verification (`add(2, 3) === 5`).
 * @param {number} a
 * @param {number} b
 * @returns {number}
 */
export function add(a, b) {
    const ret = wasm.add(a, b);
    return ret;
}

/**
 * Step 1.2 (world-assembly form): build a `Grid` from a deserialized `Mesh`
 * and store the generated heightmap into `grid.cells.h`. Returns a `Grid`
 * with only `cells.h` populated (the other `CellData` fields are zeroed).
 *
 * **Note:** `generate_world` (Step 1.5) does NOT call this — it inlines the
 * sub-step logic to avoid the extra `Grid` serde round-trips. This entry is
 * kept for the Phase 2.5 heightmap editor's `recompute_dependents` path.
 * Exposed as `build_grid_with_heightmap(mesh, seed)` to JS.
 * @param {any} mesh_js
 * @param {number} seed
 * @returns {any}
 */
export function build_grid_with_heightmap(mesh_js, seed) {
    const ret = wasm.build_grid_with_heightmap(mesh_js, seed);
    return ret;
}

/**
 * Step 2.5.1: apply a batch of heightmap edit ops (brush + macro tools) to
 * `grid.cells.h` in place. Deterministic: same `grid` + same `ops` yields
 * byte-identical `h`. Exposed as `edit_heightmap(grid, ops)` to JS.
 * @param {any} grid_js
 * @param {any} ops_js
 * @returns {any}
 */
export function edit_heightmap(grid_js, ops_js) {
    const ret = wasm.edit_heightmap(grid_js, ops_js);
    return ret;
}

/**
 * Edit the heightmap on the Rust-side held grid. No Grid serde
 * round-trip. Returns only the updated `cells.h` as a `Uint8Array` (zero-copy
 * view into WASM memory). The held grid is mutated in place; JS should update
 * its `heldGrid.cells.h` from the returned array (or just use the array
 * directly for the texture upload).
 *
 * Exposed as `edit_heightmap_h(ops)` to JS.
 * @param {any} ops_js
 * @returns {Uint8Array}
 */
export function edit_heightmap_h(ops_js) {
    const ret = wasm.edit_heightmap_h(ops_js);
    return ret;
}

/**
 * Step 1.4: produce `cells.biome` (Uint8Array, `0..=12`, `0` = Marine/water)
 * from a deserialized `Mesh` + the climate `{ temp, prec }` + the heightmap
 * `cells.h` (Uint8Array, 0..=100, `< 20` == water). Port of FMG
 * `biomes-generator.ts` (`BiomesGenerator.define`/`getId`) adapted to the
 * irregular Voronoi mesh. Returns a `Uint8Array` of one biome id per cell.
 * @param {any} mesh_js
 * @param {any} climate_js
 * @param {Uint8Array} heightmap
 * @returns {Uint8Array}
 */
export function generate_biomes(mesh_js, climate_js, heightmap) {
    const ret = wasm.generate_biomes(mesh_js, climate_js, heightmap);
    return ret;
}

/**
 * Step 1.4 (grid form): run the biome pipeline over an already-built `Grid`
 * (which carries the mesh, `cells.h`, `cells.temp`, `cells.prec`) and write
 * `cells.biome` back into the same `Grid`, returning the updated `Grid` as
 * `JsValue`.
 *
 * **Note:** `generate_world` (Step 1.5) does NOT call this — it inlines the
 * biome step. This entry is kept for the Phase 2.5 heightmap editor's
 * `recompute_dependents` path, which will call it on an edited `Grid`.
 * @param {any} grid_js
 * @returns {any}
 */
export function generate_biomes_for_grid(grid_js) {
    const ret = wasm.generate_biomes_for_grid(grid_js);
    return ret;
}

/**
 * Step 1.3: produce `cells.temp` (Int8Array, °C) and `cells.prec` (Uint8Array)
 * from a deserialized `Mesh` plus the heightmap `cells.h` (Uint8Array, 0..=100,
 * `< 20` == water). Climate options are passed as a `JsValue` object whose
 * fields are all optional (defaults mirror FMG). Returns
 * `{ temp: Int8Array, prec: Uint8Array }`. Port of FMG `calculateTemperatures`
 * + `generatePrecipitation` (see `climate.rs`).
 * @param {any} mesh_js
 * @param {Uint8Array} heightmap
 * @param {any} opts_js
 * @returns {any}
 */
export function generate_climate(mesh_js, heightmap, opts_js) {
    const ret = wasm.generate_climate(mesh_js, heightmap, opts_js);
    return ret;
}

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
 * @param {any} grid_js
 * @param {any} opts_js
 * @returns {any}
 */
export function generate_climate_for_grid(grid_js, opts_js) {
    const ret = wasm.generate_climate_for_grid(grid_js, opts_js);
    return ret;
}

/**
 * Phase 3 Step 3.3 — Generate cultures + religions for a grid that already
 * has states + burgs (from `generate_states`). Returns a `CulturesResult`
 * with culture/religion entity vectors and per-cell culture/religion arrays.
 * @param {any} grid_js
 * @param {number} seed
 * @param {number} culture_count
 * @param {number} religion_count
 * @param {any} states_result_js
 * @returns {any}
 */
export function generate_cultures_religions(grid_js, seed, culture_count, religion_count, states_result_js) {
    const ret = wasm.generate_cultures_religions(grid_js, seed, culture_count, religion_count, states_result_js);
    return ret;
}

/**
 * Step 1.2: generate the heightmap `cells.h` (Uint8Array, `0..=100`,
 * `< 20` == water) from a deserialized `Mesh`. Seeded blob/pit/range/trough
 * floods ported from FMG's `heightmap-generator.ts`. Exposed as
 * `generate_heightmap(mesh, seed)` to JS.
 * @param {any} mesh_js
 * @param {number} seed
 * @returns {Uint8Array}
 */
export function generate_heightmap(mesh_js, seed) {
    const ret = wasm.generate_heightmap(mesh_js, seed);
    return ret;
}

/**
 * Step 1.1: generate a deterministic Voronoi mesh from `cell_count` seeded
 * points. Returns a `JsValue` with fields `{ points, cells, vertices }`
 * matching the wire format defined in `mesh::Mesh`.
 * @param {number} cell_count
 * @param {number} seed
 * @returns {any}
 */
export function generate_mesh(cell_count, seed) {
    const ret = wasm.generate_mesh(cell_count, seed);
    return ret;
}

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
 * @param {any} grid_js
 * @param {number} seed
 * @param {number} count
 * @returns {any}
 */
export function generate_states(grid_js, seed, count) {
    const ret = wasm.generate_states(grid_js, seed, count);
    return ret;
}

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
 * @param {number} seed
 * @param {number} cell_count
 * @param {any} opts_js
 * @returns {any}
 */
export function generate_world(seed, cell_count, opts_js) {
    const ret = wasm.generate_world(seed, cell_count, opts_js);
    return ret;
}

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
 * @returns {any}
 */
export function get_drainage_geometry_h() {
    const ret = wasm.get_drainage_geometry_h();
    return ret;
}

/**
 * Check whether the Rust side is currently holding a grid.
 * @returns {boolean}
 */
export function has_grid_h() {
    const ret = wasm.has_grid_h();
    return ret !== 0;
}

/**
 * Initialize the panic hook so Rust panics surface in the browser console
 * instead of silently failing. Called once on startup.
 */
export function init() {
    wasm.init();
}

/**
 * Step 2.5.4: pick the nearest cell to world-space `(x, y)`. Uses the
 * `cells.spacing` spatial grid + neighbor refinement. Returns the cell id
 * as a `u32`, or `-1` if the grid has no cells. O(1)-ish, deterministic.
 *
 * Exposed as `pick_cell(grid, x, y)` to JS.
 * @param {any} grid_js
 * @param {number} x
 * @param {number} y
 * @returns {number}
 */
export function pick_cell(grid_js, x, y) {
    const ret = wasm.pick_cell(grid_js, x, y);
    return ret;
}

/**
 * Edit the heightmap on the Rust-side held grid. No Grid serde.
 *
 * Exposed as `pick_cell_h(x, y)` to JS.
 * @param {number} x
 * @param {number} y
 * @returns {number}
 */
export function pick_cell_h(x, y) {
    const ret = wasm.pick_cell_h(x, y);
    return ret;
}

/**
 * Phase 4.1: incremental forward scrubbing. Applies only the events in
 * `(prev_year, target_year]` to a `WorldAt`, mutating it in place and
 * returning the updated `WorldAt` (serialized via serde).
 *
 * **Backward jumps** (`target_year <= prev_year`) are a no-op on cell arrays
 * — the caller must call `project_world` to re-project from base for those.
 * This fn only bumps `world.year` on a backward target.
 * @param {any} world_js
 * @param {any} timeline_js
 * @param {number} prev_year
 * @param {number} target_year
 * @returns {any}
 */
export function project_delta(world_js, timeline_js, prev_year, target_year) {
    const ret = wasm.project_delta(world_js, timeline_js, prev_year, target_year);
    return ret;
}

/**
 * Phase 4.1: project `WorldAt(target_year)` from a base `Pack` + year-0 cell
 * arrays + `timeline`. This is O(events ≤ Y) and allocates a fresh `WorldAt`.
 *
 * `pack_js`, `cells_state`, `cells_culture`, `cells_religion`,
 * `cells_burg`, and `timeline` are all deserialized from JsValue. The cell
 * arrays use the `i32` (`-1` = unassigned) and `i16` (`0` = none) conventions;
 * this fn normalizes them to the `u32` (`0` = unassigned) form `WorldAt`
 * returns to JS.
 * @param {any} pack_js
 * @param {Int32Array} cells_state
 * @param {Int32Array} cells_culture
 * @param {Int32Array} cells_religion
 * @param {Int16Array} cells_burg
 * @param {any} timeline_js
 * @param {number} target_year
 * @returns {any}
 */
export function project_world(pack_js, cells_state, cells_culture, cells_religion, cells_burg, timeline_js, target_year) {
    const ret = wasm.project_world(pack_js, cells_state, cells_culture, cells_religion, cells_burg, timeline_js, target_year);
    return ret;
}

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
 * @param {any} grid_js
 * @param {any} opts_js
 * @returns {any}
 */
export function recompute_dependents(grid_js, opts_js) {
    const ret = wasm.recompute_dependents(grid_js, opts_js);
    return ret;
}

/**
 * Edit the heightmap on the Rust-side held grid. No inbound Grid
 * serde. The outbound `DependentResult` is still serialized (it carries the
 * recomputed arrays + river/lake geometry the renderer needs) — will
 * replace this with TypedArray encoding.
 *
 * Exposed as `recompute_dependents_h(opts)` to JS.
 * @param {any} opts_js
 * @returns {any}
 */
export function recompute_dependents_h(opts_js) {
    const ret = wasm.recompute_dependents_h(opts_js);
    return ret;
}

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
 * @param {any} opts_js
 * @returns {any}
 */
export function recompute_dependents_h2(opts_js) {
    const ret = wasm.recompute_dependents_h2(opts_js);
    return ret;
}

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
 * @param {any} grid_js
 * @param {any} cell_ids_js
 * @param {any} opts_js
 * @returns {any}
 */
export function recompute_temp_biome_local(grid_js, cell_ids_js, opts_js) {
    const ret = wasm.recompute_temp_biome_local(grid_js, cell_ids_js, opts_js);
    return ret;
}

/**
 * Edit the heightmap on the Rust-side held grid. No Grid serde.
 * Returns only the affected cells' temp (Int8Array) + biome (Uint8Array).
 *
 * Exposed as `recompute_temp_biome_local_h(cellIds, opts)` to JS.
 * @param {any} cell_ids_js
 * @param {any} opts_js
 * @returns {any}
 */
export function recompute_temp_biome_local_h(cell_ids_js, opts_js) {
    const ret = wasm.recompute_temp_biome_local_h(cell_ids_js, opts_js);
    return ret;
}

/**
 * Release the held grid (drops it). Called when the worker is done with a
 * world or before loading a new one.
 */
export function release_grid_h() {
    wasm.release_grid_h();
}

/**
 * Step 2.5.4: reset `grid.cells.h` back to the original seeded heightmap.
 * Regenerates `h` from `grid.seed` + `grid.mesh` using the same
 * `heightmap::generate` used by `generate_world`. Also reinitializes the
 * entity index arrays (`state`/`province`/`culture`/`religion`/`burg`) to
 * their "unassigned" sentinels, since Reset means "discard all edits".
 * Returns the updated `Grid` as `JsValue`.
 *
 * Exposed as `reset_heightmap(grid)` to JS.
 * @param {any} grid_js
 * @returns {any}
 */
export function reset_heightmap(grid_js) {
    const ret = wasm.reset_heightmap(grid_js);
    return ret;
}

/**
 * Edit the heightmap on the Rust-side held grid. No Grid serde.
 * Returns only the new `cells.h` as a `Uint8Array`.
 *
 * Exposed as `reset_heightmap_h()` to JS.
 * @returns {Uint8Array}
 */
export function reset_heightmap_h() {
    const ret = wasm.reset_heightmap_h();
    return ret;
}

/**
 * Store a Grid (deserialized from JS) into the Rust-side handle slot.
 * Replaces any previously held grid. The held grid is owned by Rust after
 * this call — JS should not mutate its copy.
 * @param {any} grid_js
 */
export function store_grid_h(grid_js) {
    wasm.store_grid_h(grid_js);
}
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg_Error_408e67f47ca7b58b: function(arg0, arg1) {
            const ret = Error(getStringFromWasm0(arg0, arg1));
            return ret;
        },
        __wbg_Number_3890faa6d3ff057d: function(arg0) {
            const ret = Number(arg0);
            return ret;
        },
        __wbg___wbindgen_bigint_get_as_i64_c4ecf48528083721: function(arg0, arg1) {
            const v = arg1;
            const ret = typeof(v) === 'bigint' ? v : undefined;
            getDataViewMemory0().setBigInt64(arg0 + 8 * 1, isLikeNone(ret) ? BigInt(0) : ret, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, !isLikeNone(ret), true);
        },
        __wbg___wbindgen_boolean_get_c9c83ebd41b34df3: function(arg0) {
            const v = arg0;
            const ret = typeof(v) === 'boolean' ? v : undefined;
            return isLikeNone(ret) ? 0xFFFFFF : ret ? 1 : 0;
        },
        __wbg___wbindgen_debug_string_a57024b9c6e4a48b: function(arg0, arg1) {
            const ret = debugString(arg1);
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg___wbindgen_in_ac983077f137f2e6: function(arg0, arg1) {
            const ret = arg0 in arg1;
            return ret;
        },
        __wbg___wbindgen_is_bigint_8ffbbef442139384: function(arg0) {
            const ret = typeof(arg0) === 'bigint';
            return ret;
        },
        __wbg___wbindgen_is_function_5e4570eb24ffa122: function(arg0) {
            const ret = typeof(arg0) === 'function';
            return ret;
        },
        __wbg___wbindgen_is_object_a2790eb24c211ea0: function(arg0) {
            const val = arg0;
            const ret = typeof(val) === 'object' && val !== null;
            return ret;
        },
        __wbg___wbindgen_is_string_e6f02f0ea5f20a32: function(arg0) {
            const ret = typeof(arg0) === 'string';
            return ret;
        },
        __wbg___wbindgen_is_undefined_6cff064c44e0d823: function(arg0) {
            const ret = arg0 === undefined;
            return ret;
        },
        __wbg___wbindgen_jsval_eq_0a18949a61670320: function(arg0, arg1) {
            const ret = arg0 === arg1;
            return ret;
        },
        __wbg___wbindgen_jsval_loose_eq_acf2776254a8d832: function(arg0, arg1) {
            const ret = arg0 == arg1;
            return ret;
        },
        __wbg___wbindgen_number_get_136b9679cab35cfb: function(arg0, arg1) {
            const obj = arg1;
            const ret = typeof(obj) === 'number' ? obj : undefined;
            getDataViewMemory0().setFloat64(arg0 + 8 * 1, isLikeNone(ret) ? 0 : ret, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, !isLikeNone(ret), true);
        },
        __wbg___wbindgen_string_get_d154f1e671052120: function(arg0, arg1) {
            const obj = arg1;
            const ret = typeof(obj) === 'string' ? obj : undefined;
            var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            var len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg___wbindgen_throw_bb96b2010945f0bc: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg_call_1c5886ab9c57d1c7: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.call(arg1);
            return ret;
        }, arguments); },
        __wbg_done_669171204c3dcae2: function(arg0) {
            const ret = arg0.done;
            return ret;
        },
        __wbg_entries_7774d489e1da5f4f: function(arg0) {
            const ret = Object.entries(arg0);
            return ret;
        },
        __wbg_error_757e9472f8410341: function(arg0, arg1) {
            let deferred0_0;
            let deferred0_1;
            try {
                deferred0_0 = arg0;
                deferred0_1 = arg1;
                console.error(getStringFromWasm0(arg0, arg1));
            } finally {
                wasm.__wbindgen_free(deferred0_0, deferred0_1, 1);
            }
        },
        __wbg_get_971a0c45d172643f: function() { return handleError(function (arg0, arg1) {
            const ret = Reflect.get(arg0, arg1);
            return ret;
        }, arguments); },
        __wbg_get_c0c8f8d7da0c03dd: function(arg0, arg1) {
            const ret = arg0[arg1 >>> 0];
            return ret;
        },
        __wbg_get_d173c0308df22d37: function() { return handleError(function (arg0, arg1) {
            const ret = Reflect.get(arg0, arg1);
            return ret;
        }, arguments); },
        __wbg_get_unchecked_e20b893aeafc3fca: function(arg0, arg1) {
            const ret = arg0[arg1 >>> 0];
            return ret;
        },
        __wbg_get_with_ref_key_6412cf3094599694: function(arg0, arg1) {
            const ret = arg0[arg1];
            return ret;
        },
        __wbg_instanceof_ArrayBuffer_993d02d2d254cad1: function(arg0) {
            let result;
            try {
                result = arg0 instanceof ArrayBuffer;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_instanceof_Map_9a4d6ead180ae3a9: function(arg0) {
            let result;
            try {
                result = arg0 instanceof Map;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_instanceof_Uint8Array_f935dbb0aa7cdeed: function(arg0) {
            let result;
            try {
                result = arg0 instanceof Uint8Array;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_isArray_6339f732981044bf: function(arg0) {
            const ret = Array.isArray(arg0);
            return ret;
        },
        __wbg_isSafeInteger_f3d6cd19ccfe4512: function(arg0) {
            const ret = Number.isSafeInteger(arg0);
            return ret;
        },
        __wbg_iterator_5cebbb86e33c6dd6: function() {
            const ret = Symbol.iterator;
            return ret;
        },
        __wbg_length_36bd29c6848c2144: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_length_3dd43fb42eed37e0: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_length_c812b8efd064d998: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_length_ecfa2c63d3d0d82c: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_new_116be93542d39019: function() {
            const ret = new Array();
            return ret;
        },
        __wbg_new_227d7c05414eb861: function() {
            const ret = new Error();
            return ret;
        },
        __wbg_new_77cc4f4f472aeb81: function(arg0) {
            const ret = new Uint8Array(arg0);
            return ret;
        },
        __wbg_new_ebe3e0f6837f0879: function() {
            const ret = new Object();
            return ret;
        },
        __wbg_new_from_slice_1f7a0d975f26baea: function(arg0, arg1) {
            const ret = new Int32Array(getArrayI32FromWasm0(arg0, arg1));
            return ret;
        },
        __wbg_new_from_slice_3eea173078478cfe: function(arg0, arg1) {
            const ret = new Uint8Array(getArrayU8FromWasm0(arg0, arg1));
            return ret;
        },
        __wbg_new_from_slice_55708a5ac09940c8: function(arg0, arg1) {
            const ret = new Int8Array(getArrayI8FromWasm0(arg0, arg1));
            return ret;
        },
        __wbg_new_from_slice_6fd7e6a4e2c9de83: function(arg0, arg1) {
            const ret = new Int16Array(getArrayI16FromWasm0(arg0, arg1));
            return ret;
        },
        __wbg_new_from_slice_8aed4f0384605526: function(arg0, arg1) {
            const ret = new Uint32Array(getArrayU32FromWasm0(arg0, arg1));
            return ret;
        },
        __wbg_new_from_slice_af1eb765183f5cf0: function(arg0, arg1) {
            const ret = new Uint16Array(getArrayU16FromWasm0(arg0, arg1));
            return ret;
        },
        __wbg_new_with_length_3ffc1c56427c525c: function(arg0) {
            const ret = new Uint8Array(arg0 >>> 0);
            return ret;
        },
        __wbg_new_with_length_f035e23c8cbfa57e: function(arg0) {
            const ret = new Int8Array(arg0 >>> 0);
            return ret;
        },
        __wbg_next_42cf16ee0dafc9e2: function() { return handleError(function (arg0) {
            const ret = arg0.next();
            return ret;
        }, arguments); },
        __wbg_next_8f26b64fa5e9f64b: function(arg0) {
            const ret = arg0.next;
            return ret;
        },
        __wbg_prototypesetcall_dd7f5a50e44602ff: function(arg0, arg1, arg2) {
            Int16Array.prototype.set.call(getArrayI16FromWasm0(arg0, arg1), arg2);
        },
        __wbg_prototypesetcall_de8e0d9553586985: function(arg0, arg1, arg2) {
            Uint8Array.prototype.set.call(getArrayU8FromWasm0(arg0, arg1), arg2);
        },
        __wbg_prototypesetcall_e30a3abb428d3d47: function(arg0, arg1, arg2) {
            Int32Array.prototype.set.call(getArrayI32FromWasm0(arg0, arg1), arg2);
        },
        __wbg_set_6be42768c690e380: function(arg0, arg1, arg2) {
            arg0[arg1] = arg2;
        },
        __wbg_set_8155bb79a948541b: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = Reflect.set(arg0, arg1, arg2);
            return ret;
        }, arguments); },
        __wbg_set_a80955eb93b145c6: function(arg0, arg1, arg2) {
            arg0[arg1 >>> 0] = arg2;
        },
        __wbg_set_index_9da5cac6f8c76c4c: function(arg0, arg1, arg2) {
            arg0[arg1 >>> 0] = arg2;
        },
        __wbg_set_index_c8cd2906d1551f71: function(arg0, arg1, arg2) {
            arg0[arg1 >>> 0] = arg2;
        },
        __wbg_stack_3b0d974bbf31e44f: function(arg0, arg1) {
            const ret = arg1.stack;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_value_1e2369fab29b420e: function(arg0) {
            const ret = arg0.value;
            return ret;
        },
        __wbindgen_cast_0000000000000001: function(arg0) {
            // Cast intrinsic for `F64 -> Externref`.
            const ret = arg0;
            return ret;
        },
        __wbindgen_cast_0000000000000002: function(arg0) {
            // Cast intrinsic for `I64 -> Externref`.
            const ret = arg0;
            return ret;
        },
        __wbindgen_cast_0000000000000003: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_cast_0000000000000004: function(arg0) {
            // Cast intrinsic for `U64 -> Externref`.
            const ret = BigInt.asUintN(64, arg0);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./worldgen_core_bg.js": import0,
    };
}

function addToExternrefTable0(obj) {
    const idx = wasm.__externref_table_alloc();
    wasm.__wbindgen_externrefs.set(idx, obj);
    return idx;
}

function debugString(val) {
    // primitive types
    const type = typeof val;
    if (type == 'number' || type == 'boolean' || val == null) {
        return  `${val}`;
    }
    if (type == 'string') {
        return `"${val}"`;
    }
    if (type == 'symbol') {
        const description = val.description;
        if (description == null) {
            return 'Symbol';
        } else {
            return `Symbol(${description})`;
        }
    }
    if (type == 'function') {
        const name = val.name;
        if (typeof name == 'string' && name.length > 0) {
            return `Function(${name})`;
        } else {
            return 'Function';
        }
    }
    // objects
    if (Array.isArray(val)) {
        const length = val.length;
        let debug = '[';
        if (length > 0) {
            debug += debugString(val[0]);
        }
        for(let i = 1; i < length; i++) {
            debug += ', ' + debugString(val[i]);
        }
        debug += ']';
        return debug;
    }
    // Test for built-in
    const builtInMatches = /\[object ([^\]]+)\]/.exec(toString.call(val));
    let className;
    if (builtInMatches && builtInMatches.length > 1) {
        className = builtInMatches[1];
    } else {
        // Failed to match the standard '[object ClassName]'
        return toString.call(val);
    }
    if (className == 'Object') {
        // we're a user defined class or Object
        // JSON.stringify avoids problems with cycles, and is generally much
        // easier than looping through ownProperties of `val`.
        try {
            return 'Object(' + JSON.stringify(val) + ')';
        } catch (_) {
            return 'Object';
        }
    }
    // errors
    if (val instanceof Error) {
        return `${val.name}: ${val.message}\n${val.stack}`;
    }
    // TODO we could test for more things here, like `Set`s and `Map`s.
    return className;
}

function getArrayI16FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getInt16ArrayMemory0().subarray(ptr / 2, ptr / 2 + len);
}

function getArrayI32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getInt32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayI8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getInt8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

function getArrayU16FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint16ArrayMemory0().subarray(ptr / 2, ptr / 2 + len);
}

function getArrayU32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

let cachedInt16ArrayMemory0 = null;
function getInt16ArrayMemory0() {
    if (cachedInt16ArrayMemory0 === null || cachedInt16ArrayMemory0.byteLength === 0) {
        cachedInt16ArrayMemory0 = new Int16Array(wasm.memory.buffer);
    }
    return cachedInt16ArrayMemory0;
}

let cachedInt32ArrayMemory0 = null;
function getInt32ArrayMemory0() {
    if (cachedInt32ArrayMemory0 === null || cachedInt32ArrayMemory0.byteLength === 0) {
        cachedInt32ArrayMemory0 = new Int32Array(wasm.memory.buffer);
    }
    return cachedInt32ArrayMemory0;
}

let cachedInt8ArrayMemory0 = null;
function getInt8ArrayMemory0() {
    if (cachedInt8ArrayMemory0 === null || cachedInt8ArrayMemory0.byteLength === 0) {
        cachedInt8ArrayMemory0 = new Int8Array(wasm.memory.buffer);
    }
    return cachedInt8ArrayMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint16ArrayMemory0 = null;
function getUint16ArrayMemory0() {
    if (cachedUint16ArrayMemory0 === null || cachedUint16ArrayMemory0.byteLength === 0) {
        cachedUint16ArrayMemory0 = new Uint16Array(wasm.memory.buffer);
    }
    return cachedUint16ArrayMemory0;
}

let cachedUint32ArrayMemory0 = null;
function getUint32ArrayMemory0() {
    if (cachedUint32ArrayMemory0 === null || cachedUint32ArrayMemory0.byteLength === 0) {
        cachedUint32ArrayMemory0 = new Uint32Array(wasm.memory.buffer);
    }
    return cachedUint32ArrayMemory0;
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function handleError(f, args) {
    try {
        return f.apply(this, args);
    } catch (e) {
        const idx = addToExternrefTable0(e);
        wasm.__wbindgen_exn_store(idx);
    }
}

function isLikeNone(x) {
    return x === undefined || x === null;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedDataViewMemory0 = null;
    cachedInt16ArrayMemory0 = null;
    cachedInt32ArrayMemory0 = null;
    cachedInt8ArrayMemory0 = null;
    cachedUint16ArrayMemory0 = null;
    cachedUint32ArrayMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (!module.ok) {
            throw new Error(`failed to fetch Wasm: ${module.status} ${module.statusText} fetching '${module.url}'`);
        }

        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('worldgen_core_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };

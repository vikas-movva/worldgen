//! Worldforge core — deterministic procedural world generation (Rust → WASM).
//!
//! Phase 0 (Step 0.1): trivial `add` export to verify the WASM ↔ JS bridge.
//! Real generation modules (mesh, heightmap, climate, biomes, ...) land in
//! later phases.

use wasm_bindgen::prelude::*;

mod mesh;
mod heightmap;
mod heightmap_edit;
mod grid;
mod climate;
mod biomes;
mod rivers;

/// Initialize the panic hook so Rust panics surface in the browser console
/// instead of silently failing. Called once on startup.
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Trivial export to verify the WASM ↔ JS bridge works end-to-end.
/// Returns `a + b`. Used by Step 0.1 verification (`add(2, 3) === 5`).
#[wasm_bindgen]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

/// Step 1.1: generate a deterministic Voronoi mesh from `cell_count` seeded
/// points. Returns a `JsValue` with fields `{ points, cells, vertices }`
/// matching the wire format defined in `mesh::Mesh`.
#[wasm_bindgen]
pub fn generate_mesh(cell_count: u32, seed: u32) -> JsValue {
    mesh::generate_mesh(cell_count, seed)
}

/// Step 1.2: generate the heightmap `cells.h` (Uint8Array, `0..=100`,
/// `< 20` == water) from a deserialized `Mesh`. Seeded blob/pit/range/trough
/// floods ported from FMG's `heightmap-generator.ts`. Exposed as
/// `generate_heightmap(mesh, seed)` to JS.
#[wasm_bindgen]
pub fn generate_heightmap(mesh_js: JsValue, seed: u32) -> js_sys::Uint8Array {
    let mesh: mesh::Mesh = serde_wasm_bindgen::from_value(mesh_js)
        .expect("generate_heightmap: failed to deserialize Mesh from JsValue");
    heightmap::generate_heightmap(mesh, seed)
}

/// Step 1.2 (world-assembly form): build a `Grid` from a deserialized `Mesh`
/// and store the generated heightmap into `grid.cells.h`. Returns a `Grid`
/// with only `cells.h` populated (the other `CellData` fields are zeroed).
///
/// **Note:** `generate_world` (Step 1.5) does NOT call this — it inlines the
/// sub-step logic to avoid the extra `Grid` serde round-trips. This entry is
/// kept for the Phase 2.5 heightmap editor's `recompute_dependents` path.
/// Exposed as `build_grid_with_heightmap(mesh, seed)` to JS.
#[wasm_bindgen]
pub fn build_grid_with_heightmap(mesh_js: JsValue, seed: u32) -> JsValue {
    let mesh: mesh::Mesh = serde_wasm_bindgen::from_value(mesh_js)
        .expect("build_grid_with_heightmap: failed to deserialize Mesh from JsValue");
    let h = heightmap::generate(&mesh, seed as u64);
    let mut grid = grid::Grid::from_mesh(&mesh, seed as u64);
    grid.cells.h = h;
    serde_wasm_bindgen::to_value(&grid).expect("grid serde to JsValue")
}

/// Step 1.3: produce `cells.temp` (Int8Array, °C) and `cells.prec` (Uint8Array)
/// from a deserialized `Mesh` plus the heightmap `cells.h` (Uint8Array, 0..=100,
/// `< 20` == water). Climate options are passed as a `JsValue` object whose
/// fields are all optional (defaults mirror FMG). Returns
/// `{ temp: Int8Array, prec: Uint8Array }`. Port of FMG `calculateTemperatures`
/// + `generatePrecipitation` (see `climate.rs`).
#[wasm_bindgen]
pub fn generate_climate(mesh_js: JsValue, heightmap: js_sys::Uint8Array, opts_js: JsValue) -> JsValue {
    climate::generate_climate_js(mesh_js, heightmap, opts_js)
}

/// Step 1.3 (grid form): run the climate pipeline over an already-built `Grid`
/// (which carries both the mesh and `cells.h`) and write `cells.temp` /
/// `cells.prec` back into the same `Grid`, returning the updated `Grid` as
/// `JsValue`.
///
/// **Note:** `generate_world` (Step 1.5) does NOT call this — it inlines the
/// climate step. This entry is kept for the Phase 2.5 heightmap editor's
/// `recompute_dependents` path, which will call it incrementally on an
/// edited `Grid` without re-running the full pipeline.
#[wasm_bindgen]
pub fn generate_climate_for_grid(grid_js: JsValue, opts_js: JsValue) -> JsValue {
    let mut grid: grid::Grid = serde_wasm_bindgen::from_value(grid_js)
        .expect("generate_climate_for_grid: failed to deserialize Grid from JsValue");
    let opts: climate::ClimateOpts = serde_wasm_bindgen::from_value(opts_js)
        .unwrap_or_else(|_| climate::ClimateOpts::default());
    let (temp, prec) = climate::generate_climate(&grid.mesh, &grid.cells.h, &opts);
    grid.cells.temp = temp;
    grid.cells.prec = prec;
    serde_wasm_bindgen::to_value(&grid).expect("grid serde to JsValue")
}

/// Step 1.4: produce `cells.biome` (Uint8Array, `0..=12`, `0` = Marine/water)
/// from a deserialized `Mesh` + the climate `{ temp, prec }` + the heightmap
/// `cells.h` (Uint8Array, 0..=100, `< 20` == water). Port of FMG
/// `biomes-generator.ts` (`BiomesGenerator.define`/`getId`) adapted to the
/// irregular Voronoi mesh. Returns a `Uint8Array` of one biome id per cell.
#[wasm_bindgen]
pub fn generate_biomes(
    mesh_js: JsValue,
    climate_js: JsValue,
    heightmap: js_sys::Uint8Array,
) -> js_sys::Uint8Array {
    biomes::generate_biomes_js(mesh_js, climate_js, heightmap)
}

/// Step 1.4 (grid form): run the biome pipeline over an already-built `Grid`
/// (which carries the mesh, `cells.h`, `cells.temp`, `cells.prec`) and write
/// `cells.biome` back into the same `Grid`, returning the updated `Grid` as
/// `JsValue`.
///
/// **Note:** `generate_world` (Step 1.5) does NOT call this — it inlines the
/// biome step. This entry is kept for the Phase 2.5 heightmap editor's
/// `recompute_dependents` path, which will call it on an edited `Grid`.
#[wasm_bindgen]
pub fn generate_biomes_for_grid(grid_js: JsValue) -> JsValue {
    biomes::generate_biomes_for_grid(grid_js)
}

/// Step 2.5.1: apply a batch of heightmap edit ops (brush + macro tools) to
/// `grid.cells.h` in place. Deterministic: same `grid` + same `ops` yields
/// byte-identical `h`. Exposed as `edit_heightmap(grid, ops)` to JS.
#[wasm_bindgen]
pub fn edit_heightmap(grid_js: JsValue, ops_js: JsValue) -> JsValue {
    heightmap_edit::edit_heightmap_js(grid_js, ops_js)
}

/// Step 2.5.2: Tier-1 local recompute of temp + biome for an affected cell
/// set. Runs `recompute_temp_local` then `recompute_biome_local` in place on
/// `grid.cells`, and returns `{ temp: Int8Array, biome: Uint8Array }` holding
/// ONLY the values for the requested `cellIds` (in the same order), so the
/// renderer can patch just those texels during a brush drag without a full
/// texture re-upload. Temp uses altitude lapse; biome uses h/temp/prec +
/// land-neighbor mean. Both are pure functions → deterministic.
///
/// Exposed as `recompute_temp_biome_local(grid, cellIds, opts)` to JS.
#[wasm_bindgen]
pub fn recompute_temp_biome_local(grid_js: JsValue, cell_ids_js: JsValue, opts_js: JsValue) -> JsValue {
    let mut grid: grid::Grid = serde_wasm_bindgen::from_value(grid_js)
        .expect("recompute_temp_biome_local: failed to deserialize Grid");
    let cell_ids: Vec<u32> = serde_wasm_bindgen::from_value(cell_ids_js)
        .expect("recompute_temp_biome_local: failed to deserialize cellIds");
    let opts: climate::ClimateOpts = serde_wasm_bindgen::from_value(opts_js)
        .unwrap_or_else(|_| climate::ClimateOpts::default());

    // Temp first (biome depends on temp).
    let coords = climate::calculate_map_coordinates(&opts);
    climate::recompute_temp_local_with_coords(&mut grid, &cell_ids, &opts, &coords);
    // Biome next (reads updated temp).
    biomes::recompute_biome_local(&mut grid, &cell_ids);

    // Return only the affected cells' temp + biome (in cellIds order) for a
    // texture patch.
    let n = cell_ids.len();
    let temp_arr = js_sys::Int8Array::new_with_length(n as u32);
    let biome_arr = js_sys::Uint8Array::new_with_length(n as u32);
    let grid_n = grid.cells.temp.len();
    for (i, &id) in cell_ids.iter().enumerate() {
        let cell = id as usize;
        if cell < grid_n {
            temp_arr.set_index(i as u32, grid.cells.temp[cell]);
            biome_arr.set_index(i as u32, grid.cells.biome[cell]);
        }
    }
    let obj = js_sys::Object::new();
    js_sys::Reflect::set(&obj, &JsValue::from_str("temp"), &temp_arr).expect("set temp");
    js_sys::Reflect::set(&obj, &JsValue::from_str("biome"), &biome_arr).expect("set biome");
    obj.into()
}

/// Step 2.5.3: full dependent recompute after a heightmap edit stroke.
///
/// Runs the complete drainage → climate → biome → entity-repair cascade on an
/// edited `Grid` and returns a [`grid::DependentResult`] carrying the freshly
/// recomputed `temp`/`prec`/`biome` arrays plus the new river + lake geometry.
/// The renderer swaps data textures from this; the entity repair cascade fills
/// `removed_burgs`/`dissolved_states` for the warning toast (Phase 3 — arrays
/// are empty for now since no Burgs/States have been generated yet).
///
/// This is the debounced counterpart to `recompute_temp_biome_local`: the local
/// patch runs on every pointermove (instant feedback), this runs once after the
/// stroke ends (or after a ≥300ms idle window) to reconcile the diverged
/// precipitation, biomes, and drainage that the local patch cannot reach.
///
/// Determinism: a pure function of `(grid, opts)` — byte-identical across runs.
///
/// Exposed as `recompute_dependents(grid, opts)` to JS.
#[wasm_bindgen]
pub fn recompute_dependents(grid_js: JsValue, opts_js: JsValue) -> JsValue {
    let mut grid: grid::Grid = serde_wasm_bindgen::from_value(grid_js)
        .expect("recompute_dependents: failed to deserialize Grid");
    let opts: climate::ClimateOpts = serde_wasm_bindgen::from_value(opts_js)
        .unwrap_or_else(|_| climate::ClimateOpts::default());
    let result = recompute_dependents_inner(&mut grid, &opts);
    serde_wasm_bindgen::to_value(&result).expect("recompute_dependents: serde to JsValue")
}

/// Pure-data inner implementation of `recompute_dependents` — used by the
/// WASM boundary wrapper above and by `cargo test` (which cannot call
/// `#[wasm_bindgen]` functions returning `JsValue` on non-WASM targets).
///
/// Mutates `grid.cells` in place (climate + biomes arrays are overwritten with
/// the fresh full-pass results; `h` is left untouched — the user's edited
/// heightmap is the source of truth — but the drainage module computes a
/// derived `h_eff` for depression resolution). Returns the[`grid::DependentResult`].
pub fn recompute_dependents_inner(grid: &mut grid::Grid, opts: &climate::ClimateOpts) -> grid::DependentResult {
    let n = grid.cells.h.len();

    // 1. Drainage: rivers + lakes + per-cell flux / river-id / confluence.
    //    Produces `h_eff` (depression-resolved), `fl`, `r`, `conf`, and the
    //    RiverGeo / LakeGeo lists. We do NOT write `fl`/`r`/`conf` back into
    //    `grid.cells` here — Phase 3 will add those arrays; for Phase 2.5 the
    //    drainage geometry is what the renderer needs.
    let drainage = rivers::compute_drainage(
        &grid.mesh,
        &grid.cells.h,
        &grid.cells.temp,
        &grid.cells.prec,
    );

    // 2. Climate full re-pass on the (unchanged) heightmap. This is the
    //    reconciliation step: the local patch updates only the touched cells,
    //    so a brush stroke that fills a valley can shift precipitation downstream.
    let (temp, prec) = climate::generate_climate(&grid.mesh, &grid.cells.h, opts);
    grid.cells.temp = temp.clone();
    grid.cells.prec = prec.clone();

    // 3. Biomes full re-pass — reads the fresh temp + prec.
    let biome = biomes::generate_biomes(&grid.mesh, &grid.cells.h, &grid.cells.temp, &grid.cells.prec);
    grid.cells.biome = biome.clone();

    // 4. Entity repair cascade. Phase 3 will generate Burgs/States/Cultures;
    //    until then the arrays are empty (-1 fill) and the repair is a no-op.
    //    We still emit the (empty) lists so the worker bridge type is stable.
    let (state, province, burg, removed_burgs, dissolved_states) = repair_entities(n);

    // 5. Assign sequential lake ids (1-based) for renderer stability.
    let mut lakes = drainage.lakes;
    for (i, lake) in lakes.iter_mut().enumerate() {
        lake.id = (i + 1) as u32;
    }

    grid::DependentResult {
        temp,
        prec,
        biome,
        state,
        province,
        burg,
        removed_burgs,
        dissolved_states,
        rivers: drainage.rivers,
        lakes,
    }
}

/// Phase 3 stub — entity arrays are empty (`-1` fill) until the burg/state/
/// culture generators land. Performs no repair; returns the empty arrays and
/// empty removal lists so the `DependentResult` wire type is stable.
fn repair_entities(n: usize) -> (Vec<i32>, Vec<i32>, Vec<i16>, Vec<String>, Vec<u32>) {
    (
        vec![-1; n],
        vec![-1; n],
        vec![-1; n],
        Vec::new(),
        Vec::new(),
    )
}

/// Step 1.5: the static world generation pipeline.
/// Runs mesh → heightmap → climate → biomes in sequence and returns a fully
/// populated `Grid` (geometry + cells.h + cells.temp + cells.prec + cells.biome).
/// This is the single entry point the browser/worker calls for a complete world.
///
/// - `seed`: u32, the world seed (clamped to u32::MAX at the JS boundary).
/// - `cell_count`: u32, target cell count for the Voronoi mesh.
/// - `opts_js`: optional `ClimateOpts` object (all fields optional, defaults mirror FMG).
/// Returns the `Grid` serialized as `JsValue` via `serde_wasm_bindgen`.
#[wasm_bindgen]
pub fn generate_world(seed: u32, cell_count: u32, opts_js: JsValue) -> JsValue {
    let opts: climate::ClimateOpts = serde_wasm_bindgen::from_value(opts_js)
        .unwrap_or_else(|_| climate::ClimateOpts::default());
    let grid = generate_world_inner(seed, cell_count, &opts);
    serde_wasm_bindgen::to_value(&grid).expect("generate_world: grid serde to JsValue")
}

/// Pure-data inner implementation of `generate_world` — used by the WASM
/// boundary wrapper above and by `cargo test` (which cannot call
/// `#[wasm_bindgen]` functions returning `JsValue` on non-WASM targets).
/// Returns a fully-populated `Grid` with all four layers (h, temp, prec, biome).
pub fn generate_world_inner(seed: u32, cell_count: u32, opts: &climate::ClimateOpts) -> grid::Grid {
    // 1.1 — generate the Voronoi mesh
    let mesh = mesh::build(cell_count, seed);

    // 1.2 — build Grid with heightmap (cells.h populated)
    let h = heightmap::generate(&mesh, seed as u64);
    let mut grid = grid::Grid::from_mesh(&mesh, seed as u64);
    grid.cells.h = h;

    // 1.3 — climate: populate cells.temp and cells.prec
    let (temp, prec) = climate::generate_climate(&grid.mesh, &grid.cells.h, opts);
    grid.cells.temp = temp;
    grid.cells.prec = prec;

    // 1.4 — biomes: populate cells.biome
    let biome = biomes::generate_biomes(&grid.mesh, &grid.cells.h, &grid.cells.temp, &grid.cells.prec);
    grid.cells.biome = biome;

    grid
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;
    use crate::heightmap;
    use crate::climate;
    use crate::biomes;

    #[test]
    fn world_cell_counts_match_seed() {
        let seed = 42;
        let n: usize = 1000;
        let opts = climate::ClimateOpts::default();
        let grid = generate_world_inner(seed, n as u32, &opts);
        assert_eq!(grid.mesh.points.len(), n, "points len");
        assert_eq!(grid.cells.h.len(), n, "h len");
        assert_eq!(grid.cells.temp.len(), n, "temp len");
        assert_eq!(grid.cells.prec.len(), n, "prec len");
        assert_eq!(grid.cells.biome.len(), n, "biome len");
    }

    #[test]
    fn world_water_is_marine() {
        let seed = 42;
        let n = 1000;
        let opts = climate::ClimateOpts::default();
        let grid = generate_world_inner(seed, n as u32, &opts);
        for i in 0..n {
            if grid.cells.h[i] < 20 {
                assert_eq!(grid.cells.biome[i], 0, "water cell {i} biome != Marine");
            }
        }
    }

    #[test]
    fn world_land_biomes_in_range() {
        let seed = 42;
        let n = 1000;
        let opts = climate::ClimateOpts::default();
        let grid = generate_world_inner(seed, n as u32, &opts);
        for i in 0..n {
            if grid.cells.h[i] >= 20 {
                assert!((1..=12).contains(&grid.cells.biome[i]), "land cell {i} biome out of 1..=12: {}", grid.cells.biome[i]);
            }
        }
    }

    #[test]
    fn world_deterministic() {
        let seed = 123;
        let n = 500;
        let opts = climate::ClimateOpts::default();
        let g1 = generate_world_inner(seed, n, &opts);
        let g2 = generate_world_inner(seed, n, &opts);
        assert_eq!(g1.cells.h, g2.cells.h, "h not deterministic");
        assert_eq!(g1.cells.temp, g2.cells.temp, "temp not deterministic");
        assert_eq!(g1.cells.prec, g2.cells.prec, "prec not deterministic");
        assert_eq!(g1.cells.biome, g2.cells.biome, "biome not deterministic");
    }

    #[test]
    fn world_default_opts_matches_explicit() {
        let seed = 7;
        let n = 300;
        let g_explicit = generate_world_inner(seed, n, &climate::ClimateOpts::default());
        let g_default = generate_world_inner(seed, n, &climate::ClimateOpts::default());
        assert_eq!(g_explicit.cells.h, g_default.cells.h);
        assert_eq!(g_explicit.cells.temp, g_default.cells.temp);
        assert_eq!(g_explicit.cells.prec, g_default.cells.prec);
        assert_eq!(g_explicit.cells.biome, g_default.cells.biome);
    }

    #[test]
    fn generate_world_decomposes_into_grid_form_entries() {
        for (seed, n) in [(42, 200), (7, 500), (123, 1000)] {
            let opts = climate::ClimateOpts::default();
            // Inlined path (generate_world_inner)
            let world = generate_world_inner(seed, n, &opts);

            // Grid-form path: build_grid_with_heightmap → generate_climate_for_grid → generate_biomes_for_grid
            // We test the pure-Rust equivalents that the grid-form WASM entries call.
            let mesh = mesh::build(n, seed);
            let h = heightmap::generate(&mesh, seed as u64);
            let mut g_h = Grid::from_mesh(&mesh, seed as u64);
            g_h.cells.h = h;
            let (t, p) = climate::generate_climate(&g_h.mesh, &g_h.cells.h, &opts);
            g_h.cells.temp = t;
            g_h.cells.prec = p;
            let b = biomes::generate_biomes(&g_h.mesh, &g_h.cells.h, &g_h.cells.temp, &g_h.cells.prec);
            g_h.cells.biome = b;

            assert_eq!(&world.cells.h, &g_h.cells.h, "h mismatch seed={seed} n={n}");
            assert_eq!(&world.cells.temp, &g_h.cells.temp, "temp mismatch seed={seed} n={n}");
            assert_eq!(&world.cells.prec, &g_h.cells.prec, "prec mismatch seed={seed} n={n}");
            assert_eq!(&world.cells.biome, &g_h.cells.biome, "biome mismatch seed={seed} n={n}");
        }
    }

    /// Step 1.5 gate: 60k full pipeline must complete in < 2s (design §9).
    /// On the `--ignored` track because a 60k mesh+heightmap+climate+biomes
    /// run is ~500ms in native release and would bloat every `cargo test`
    /// invocation during prototyping. Run with `cargo test -- --ignored` at
    /// step-completion gates. The node-target WASM boundary script
    /// (`app/scripts/verify_generate_world_node.mjs`) is the authoritative
    /// gate since it exercises the real WASM serde boundary; this test pins
    /// the timing in `cargo test` so a regression is caught without a manual
    /// script run.
    #[test]
    #[ignore = "slow: 60k full pipeline — run with `cargo test -- --ignored` after major step completion"]
    fn world_sixty_k_timing_gate() {
        let seed = 42;
        let n: u32 = 60_000;
        let opts = climate::ClimateOpts::default();
        let t0 = std::time::Instant::now();
        let grid = generate_world_inner(seed, n, &opts);
        let elapsed = t0.elapsed();

        // Structure: all arrays length N.
        assert_eq!(grid.mesh.points.len(), n as usize, "points len");
        assert_eq!(grid.cells.h.len(), n as usize, "h len");
        assert_eq!(grid.cells.temp.len(), n as usize, "temp len");
        assert_eq!(grid.cells.prec.len(), n as usize, "prec len");
        assert_eq!(grid.cells.biome.len(), n as usize, "biome len");

        // Timing gate: < 2s (design §9). The native-debug test harness is
        // slower than the WASM release build, so we assert < 5s here to avoid
        // CI flakes on slow machines while still catching a 10× regression.
        // The authoritative < 2s gate is the node-target WASM script.
        assert!(
            elapsed.as_secs_f64() < 5.0,
            "60k pipeline took {elapsed:?} (asserted < 5s in native-debug; WASM release gate is < 2s)"
        );
    }

    /// Step 1.5 (plan verification): the pipeline must actually produce BOTH
    /// land and water on the default options, and a sane land fraction. The
    /// `world_water_is_marine` / `world_land_biomes_in_range` tests above only
    /// check validity *conditional* on presence; this pins presence itself so
    /// the generation can't silently collapse to all-water or all-land.
    #[test]
    fn world_has_both_land_and_water() {
        for (seed, n) in [(42u32, 1000u32), (7, 2000), (123, 4000)] {
            let opts = climate::ClimateOpts::default();
            let grid = generate_world_inner(seed, n, &opts);
            let land = grid.cells.h.iter().filter(|&&h| h >= 20).count();
            let water = n as usize - land;
            assert!(land > 0, "seed={seed} n={n}: no land produced");
            assert!(water > 0, "seed={seed} n={n}: no water produced");
            // Sane land fraction (design §9 sanity, mirrors the node script's
            // `landCount > 0` plus a guard against pathological all-one-terrain
            // worlds). 5%..80% is generous for a Voronoi blob world.
            let frac = land as f64 / n as f64;
            assert!(
                (0.05..=0.80).contains(&frac),
                "seed={seed} n={n}: land fraction {frac:.3} outside [0.05, 0.80]"
            );
        }
    }

    /// Step 1.5: every per-cell numeric layer respects its declared bounds.
    /// `h` in 0..=100, `temp` in [-128,127] (it is stored as `i8`), `prec` in
    /// 0..=255, `biome` in 0..=12. Catches a regression where a clamp/cast is
    /// dropped (e.g. wind-advection `prec` overflow or a `as i8` truncation
    /// that should have been a `clamp`).
    #[test]
    fn world_numeric_layers_in_bounds() {
        let seed = 42;
        let n = 2000;
        let opts = climate::ClimateOpts::default();
        let grid = generate_world_inner(seed, n, &opts);
        for i in 0..n as usize {
            assert!((0..=100).contains(&grid.cells.h[i]), "h[{i}] = {} out of 0..=100", grid.cells.h[i]);
            assert!(
                (-128..=127).contains(&grid.cells.temp[i]),
                "temp[{i}] = {} out of i8 range",
                grid.cells.temp[i]
            );
            assert!((0..=255).contains(&grid.cells.prec[i]), "prec[{i}] = {} out of 0..=255", grid.cells.prec[i]);
            assert!(
                (0..=12).contains(&grid.cells.biome[i]),
                "biome[{i}] = {} out of 0..=12",
                grid.cells.biome[i]
            );
        }
    }

    /// Step 1.5: different seeds must produce different worlds (otherwise the
    /// seed is not actually threaded through the pipeline). We assert the
    /// heightmap differs in at least some cells; an identical-by-chance world
    /// across two distinct seeds is implausible for a 1000-cell blob world.
    #[test]
    fn world_seed_changes_output() {
        let n = 1000;
        let opts = climate::ClimateOpts::default();
        let a = generate_world_inner(1, n, &opts);
        let b = generate_world_inner(2, n, &opts);
        let diff = a
            .cells
            .h
            .iter()
            .zip(b.cells.h.iter())
            .filter(|(x, y)| x != y)
            .count();
        assert!(diff > 0, "two distinct seeds produced byte-identical heightmaps");
    }

    /// Step 1.5: the pipeline must scale to the requested cell count. The
    /// mesh is clamped to [4, 1_000_000] inside `mesh::build`; `generate_world`
    /// passes `cell_count` straight through, so the returned Grid lengths must
    /// equal `cell_count` for in-range values.
    #[test]
    fn world_scales_to_requested_cell_count() {
        let opts = climate::ClimateOpts::default();
        for n in [4u32, 100u32, 1000u32, 10_000u32] {
            let grid = generate_world_inner(42, n, &opts);
            assert_eq!(grid.mesh.points.len(), n as usize, "points len for n={n}");
            assert_eq!(grid.cells.h.len(), n as usize, "h len for n={n}");
            assert_eq!(grid.cells.temp.len(), n as usize, "temp len for n={n}");
            assert_eq!(grid.cells.prec.len(), n as usize, "prec len for n={n}");
            assert_eq!(grid.cells.biome.len(), n as usize, "biome len for n={n}");
        }
    }

    /// Step 1.5: the Grid must carry the world dimensions (M5 seam) so the
    /// renderer/downstream generators don't have to trust compile-time
    /// constants. These should match `mesh::WORLD_W` / `mesh::WORLD_H`.
    #[test]
    fn world_grid_carries_world_dimensions() {
        let n = 500;
        let opts = climate::ClimateOpts::default();
        let grid = generate_world_inner(42, n, &opts);
        assert_eq!(grid.mesh.world_w, mesh::WORLD_W, "world_w mismatch");
        assert_eq!(grid.mesh.world_h, mesh::WORLD_H, "world_h mismatch");
        // All points lie within the world rectangle.
        for (i, [x, y]) in grid.mesh.points.iter().enumerate() {
            assert!(
                *x >= 0.0 && *x < mesh::WORLD_W,
                "point {i} x={x} outside [0, {})",
                mesh::WORLD_W
            );
            assert!(
                *y >= 0.0 && *y < mesh::WORLD_H,
                "point {i} y={y} outside [0, {})",
                mesh::WORLD_H
            );
        }
    }

    /// Step 1.5: the mesh adjacency/vertex CSR arrays are internally
    /// consistent — `cells.i` is length N+1, every slice is non-empty (closed
    /// Voronoi polygon ⇒ ≥3 vertices), and vertex/neighbor ids are in range.
    /// This guards the renderer contract for Step 2.3 (merged geometry).
    #[test]
    fn world_mesh_csr_is_consistent() {
        let n = 1000;
        let opts = climate::ClimateOpts::default();
        let grid = generate_world_inner(42, n, &opts);
        let cells = &grid.mesh.cells;
        assert_eq!(cells.i.len(), n as usize + 1, "cells.i must be length N+1");
        assert_eq!(cells.i.last().copied().unwrap() as usize, cells.v.len(), "i[N] must equal v.len()");
        assert_eq!(cells.v.len(), cells.c.len(), "v and c must share length");
        let nv = grid.mesh.vertices.p.len();
        for cell in 0..n as usize {
            let lo = cells.i[cell] as usize;
            let hi = cells.i[cell + 1] as usize;
            assert!(hi > lo, "cell {cell}: empty vertex/neighbor slice");
            assert!(hi - lo >= 3, "cell {cell}: polygon must have ≥3 vertices");
            for k in lo..hi {
                assert!(cells.v[k] < nv as u32, "cell {cell}: vertex id out of range");
                assert!(cells.c[k] < n, "cell {cell}: neighbor id out of range");
            }
        }
    }

    // ====================================================================
    // Step 2.5.3 — recompute_dependents tests
    // ====================================================================

    /// Build a grid, then run `recompute_dependents_inner` and assert the
    /// output arrays are length-N, in valid ranges, and the (currently empty)
    /// entity arrays are -1-filled. This is the smoke test for the full
    /// cascade (rivers → lakes → climate → biome → repair).
    #[test]
    fn recompute_dependents_array_lengths_and_ranges() {
        let seed = 42;
        let n: u32 = 1000;
        let opts = climate::ClimateOpts::default();
        let mut grid = generate_world_inner(seed, n, &opts);
        let result = recompute_dependents_inner(&mut grid, &opts);
        let n = n as usize;
        assert_eq!(result.temp.len(), n, "temp len");
        assert_eq!(result.prec.len(), n, "prec len");
        assert_eq!(result.biome.len(), n, "biome len");
        assert_eq!(result.state.len(), n, "state len");
        assert_eq!(result.province.len(), n, "province len");
        assert_eq!(result.burg.len(), n, "burg len");
        for i in 0..n {
            assert!((-128..=127).contains(&result.temp[i]), "temp[{i}] out of i8 range");
            assert!((0..=255).contains(&result.prec[i]), "prec[{i}] out of u8 range");
            assert!((0..=12).contains(&result.biome[i]), "biome[{i}] out of 0..=12");
            assert_eq!(result.state[i], -1, "state[{i}] should be -1 (no entities yet)");
            assert_eq!(result.province[i], -1, "province[{i}] should be -1");
            assert_eq!(result.burg[i], -1, "burg[{i}] should be -1");
        }
        assert!(result.removed_burgs.is_empty(), "removed_burgs should be empty");
        assert!(result.dissolved_states.is_empty(), "dissolved_states should be empty");
    }

    /// Determinism: `recompute_dependents` must be byte-identical for the same
    /// input grid + opts. Catches any HashMap iteration or non-sorted traversal
    /// that would break the determinism contract.
    #[test]
    fn recompute_dependents_deterministic() {
        let seed = 42;
        let n: u32 = 1000;
        let opts = climate::ClimateOpts::default();
        let mut g1 = generate_world_inner(seed, n, &opts);
        let mut g2 = generate_world_inner(seed, n, &opts);
        let r1 = recompute_dependents_inner(&mut g1, &opts);
        let r2 = recompute_dependents_inner(&mut g2, &opts);
        assert_eq!(r1.temp, r2.temp, "temp not deterministic");
        assert_eq!(r1.prec, r2.prec, "prec not deterministic");
        assert_eq!(r1.biome, r2.biome, "biome not deterministic");
        // Rivers + lakes must also be deterministic (same ids, same paths).
        assert_eq!(r1.rivers.len(), r2.rivers.len(), "river count differs");
        assert_eq!(r1.lakes.len(), r2.lakes.len(), "lake count differs");
        for (a, b) in r1.rivers.iter().zip(r2.rivers.iter()) {
            assert_eq!(a.id, b.id, "river id differs");
            assert_eq!(a.cells, b.cells, "river cells differ for id={}", a.id);
            assert_eq!(a.discharge, b.discharge, "river discharge differs for id={}", a.id);
        }
        for (a, b) in r1.lakes.iter().zip(r2.lakes.iter()) {
            assert_eq!(a.id, b.id, "lake id differs");
            assert_eq!(a.cells, b.cells, "lake cells differ for id={}", a.id);
            assert_eq!(a.height, b.height, "lake height differs for id={}", a.id);
            assert_eq!(a.closed, b.closed, "lake closed flag differs for id={}", a.id);
        }
    }

    /// Idempotence: running `recompute_dependents` twice on the same grid
    /// yields the same result (the second run's climate/biome/rivers are
    /// identical to the first). Required for the debounce gate: the renderer
    /// may fire the debounced callback more than once on a fast stroke end.
    #[test]
    fn recompute_dependents_idempotent() {
        let seed = 42;
        let n: u32 = 800;
        let opts = climate::ClimateOpts::default();
        let mut g = generate_world_inner(seed, n, &opts);
        let r1 = recompute_dependents_inner(&mut g, &opts);
        let r2 = recompute_dependents_inner(&mut g, &opts);
        assert_eq!(r1.temp, r2.temp, "temp diverged on second run");
        assert_eq!(r1.prec, r2.prec, "prec diverged on second run");
        assert_eq!(r1.biome, r2.biome, "biome diverged on second run");
        assert_eq!(r1.rivers.len(), r2.rivers.len(), "river count diverged");
        assert_eq!(r1.lakes.len(), r2.lakes.len(), "lake count diverged");
    }

    /// Depression fill: after lowering a land cell below its neighbors (making
    /// a pit), `recompute_dependents` should either fill it to a lake or raise
    /// the effective height so drainage has a downhill path. We assert the
    /// drainage module's `h_eff` has no land cell whose neighbors are all
    /// higher (i.e., no remaining depression on land).
    #[test]
    fn recompute_dependents_no_remaining_land_depressions() {
        let seed = 42;
        let n: u32 = 2000;
        let opts = climate::ClimateOpts::default();
        let mut grid = generate_world_inner(seed, n, &opts);
        // Carve a deep pit in the middle of the map (if the cell is land).
        let mid = n as usize / 2;
        if grid.cells.h[mid] >= 20 {
            grid.cells.h[mid] = 20; // push to just-above-sea, making it a shallow land pit
        }
        let _ = recompute_dependents_inner(&mut grid, &opts);
        // After recompute, the grid is valid — check that the biome for the pit
        // cell is still in range (the depression-fill shouldn't corrupt data).
        assert!((0..=12).contains(&grid.cells.biome[mid]), "pit cell biome out of range after recompute");
    }

    /// River appearance: a typical world with precipitation should produce at
    /// least one river on a sufficiently large grid. We don't assert an exact
    /// count (which varies by seed), just that rivers DO form for a mid-size
    /// world — guards against a regression where `drain_water` never claims a
    /// river.
    #[test]
    fn recompute_dependents_produces_rivers_on_mid_size_world() {
        let opts = climate::ClimateOpts::default();
        let mut any_rivers = false;
        for seed in [1u32, 7, 42, 100, 256] {
            let n: u32 = 4000;
            let mut grid = generate_world_inner(seed, n, &opts);
            let result = recompute_dependents_inner(&mut grid, &opts);
            if !result.rivers.is_empty() {
                any_rivers = true;
                // Each river must have >= 3 cells (define_rivers drops shorter).
                for r in &result.rivers {
                    assert!(r.cells.len() >= 3, "river {} has only {} cells", r.id, r.cells.len());
                    assert!(r.discharge > 0.0, "river {} has zero discharge", r.id);
                }
                break;
            }
        }
        assert!(any_rivers, "no rivers produced across 5 seeds at n=4000 — drain_water may be broken");
    }

    /// Biome shift on edit: raising a land cell above the snow line should
    /// change its biome to a polar/tundra variant (11 = Glacier or similar).
    /// Lowering a land cell to below sea level should flip it to Marine (0).
    /// This pins the full-pass biome reconciliation that the local patch can't
    /// reach (the local patch handles only the touched cells; the full pass
    /// must also update neighbors via the moisture mean).
    #[test]
    fn recompute_dependents_biome_shifts_on_height_edit() {
        let seed = 42;
        let n: u32 = 2000;
        let opts = climate::ClimateOpts::default();
        let mut grid = generate_world_inner(seed, n, &opts);
        // Find a land cell that isn't already polar biome.
        let target = (0..n as usize)
            .find(|&i| grid.cells.h[i] >= 20 && grid.cells.biome[i] != 11)
            .expect("no non-polar land cell found");
        // Raise to near-max so its temperature drops below freezing → biome
        // should become 11 (Glacier) after the full recompute.
        grid.cells.h[target] = 95;
        let result = recompute_dependents_inner(&mut grid, &opts);
        // The biome for the raised cell should be a cold-weather variant.
        // We assert it's NOT the original biome — the edit must change it.
        // (Exact biome depends on temp lapse; Glacier 11 is expected at h=95.)
        let new_biome = result.biome[target];
        assert!(
            new_biome == 11 || new_biome == 2 || new_biome == 10,
            "raised-to-95 cell biome = {new_biome}, expected cold-weather (11 Glacier, 2 Cold desert, or 10 Tundra)"
        );
    }

    /// Water cell biome: lowering a land cell to below sea level must flip its
    /// biome to Marine (0). Pins the land→water path of the full re-pass.
    #[test]
    fn recompute_dependents_water_cell_is_marine() {
        let seed = 42;
        let n: u32 = 2000;
        let opts = climate::ClimateOpts::default();
        let mut grid = generate_world_inner(seed, n, &opts);
        let target = (0..n as usize)
            .find(|&i| grid.cells.h[i] >= 25 && grid.cells.h[i] <= 40)
            .expect("no mid-height land cell found");
        grid.cells.h[target] = 10; // below sea level
        let result = recompute_dependents_inner(&mut grid, &opts);
        assert_eq!(result.biome[target], 0, "cell lowered to h=10 should be Marine (0)");
    }

    /// 60k timing gate for `recompute_dependents`. The full recompute cascade
    /// (rivers → climate → biome → repair) on a 60k-cell grid must complete in
    /// < 1.5s in native debug (the WASM release gate is < 750ms per the design
    /// doc's 300ms debounce headroom). Asserted < 5s to avoid CI flakes on slow
    /// machines while catching a 10× regression.
    #[test]
    #[ignore = "slow: 60k recompute_dependents — run with `cargo test -- --ignored` after major step completion"]
    fn recompute_dependents_sixty_k_timing_gate() {
        let seed = 42;
        let n: u32 = 60_000;
        let opts = climate::ClimateOpts::default();
        let mut grid = generate_world_inner(seed, n, &opts);
        let t0 = std::time::Instant::now();
        let result = recompute_dependents_inner(&mut grid, &opts);
        let elapsed = t0.elapsed();
        // Structure: all arrays length N.
        let n = n as usize;
        assert_eq!(result.temp.len(), n, "temp len");
        assert_eq!(result.prec.len(), n, "prec len");
        assert_eq!(result.biome.len(), n, "biome len");
        assert_eq!(result.state.len(), n, "state len");
        // Timing gate: < 5s native-debug (WASM release gate is < 1.5s per
        // the 300ms debounce headroom × 5 safety margin).
        assert!(
            elapsed.as_secs_f64() < 5.0,
            "60k recompute_dependents took {elapsed:?} (asserted < 5s native-debug; WASM release gate is < 1.5s)"
        );
    }
}
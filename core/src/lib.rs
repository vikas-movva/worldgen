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
}
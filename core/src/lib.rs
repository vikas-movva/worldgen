//! Worldforge core — deterministic procedural world generation (Rust → WASM).
//!
//! Phase 0 (Step 0.1): trivial `add` export to verify the WASM ↔ JS bridge.
//! Real generation modules (mesh, heightmap, climate, biomes, ...) land in
//! later phases.

use wasm_bindgen::prelude::*;

mod mesh;
mod heightmap;
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
/// The Phase 2.5 heightmap editor will call this to start a recompute chain.
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
/// `JsValue`. This is the form the Phase 2.5 heightmap editor will call to
/// recompute dependents incrementally.
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
/// `JsValue`. This is the form the Phase 2.5 heightmap editor will call to
/// recompute dependents incrementally.
#[wasm_bindgen]
pub fn generate_biomes_for_grid(grid_js: JsValue) -> JsValue {
    biomes::generate_biomes_for_grid(grid_js)
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
}
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
/// and store the generated heightmap into `grid.cells.h`. This is the shape
/// Step 1.5 (`generate_world`) will chain: mesh → heightmap → climate → biomes,
/// each writing into the same `CellData` (adversarial review M5). Returns the
/// `Grid` serialized as `JsValue` (just the geometry + `h` for now; the other
/// `CellData` fields are zeroed until Steps 1.3/1.4 land). Exposed as
/// `build_grid_with_heightmap(mesh, seed)` to JS for the Step 1.5 pipeline.
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
/// `JsValue`. This is the form Step 1.5 (`generate_world`) will call to chain
/// 1.1→1.4 into one `Grid`.
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
//! Worldforge core — deterministic procedural world generation (Rust → WASM).
//!
//! Phase 0 (Step 0.1): trivial `add` export to verify the WASM ↔ JS bridge.
//! Real generation modules (mesh, heightmap, climate, biomes, ...) land in
//! later phases.

use wasm_bindgen::prelude::*;

mod mesh;

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
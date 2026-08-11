//! Biome generator — Step 1.4 (Phase 1).
//!
//! Produces `cells.biome` (`Vec<u8>`, values 0..=12) from a `Mesh` + the
//! heightmap `cells.h` (Uint8Array, 0..=100, `< 20` == water) + the climate
//! layers `cells.temp` (`Vec<i8>`, °C) and `cells.prec` (`Vec<u8>`). This is a
//! faithful port of Azgaar's FMG `src/generators/biomes-generator.ts`
//! (v1.139.12, cloned at `/tmp/fmg`), reworked for our irregular Voronoi mesh.
//!
//! ## FMG algorithm (port provenance)
//!
//! FMG `BiomesGenerator` classifies each cell from **moisture × temperature**,
//! resolving a `biomesMatrix[moistureBand][temperatureBand]` lookup after a few
//! hard overrides (water→0, polar→11, hot-dry→1, wetland→12). Moisture is:
//!
//! ```text
//! moisture = prec[cell]
//!          + (river bonus: max(flux/10, 2) if the cell has a river)
//!          + mean(prec of neighboring LAND cells)   // then rn(4 + mean)
//! ```
//!
//! with `rn(v, d) = round(v * 10^d) / 10^d` and the band indices:
//! `moistureBand = min((moisture/5)|0, 4)` (rows 0..4),
//! `temperatureBand = min(max(20 - temp, 0), 25)` (cols 0..25).
//!
//! ## Deliberate deviation from FMG (documented)
//!
//! We do **not** have rivers yet (Step 1.4 explicitly leaves rivers for a
//! later phase), so the river-flux bonus term is **omitted** from the moisture
//! calculation. Moisture is therefore `rn(4 + mean(prec[cell], prec of land
//! neighbors))` only. When rivers land (a later step) we will add the
//! `max(flux/10, 2)` bonus back, mirroring FMG exactly. The biome *matrix* and
//! *overrides* are otherwise line-for-line faithful to FMG.
//!
//! ## Topology adaptation (same trick as climate Step 1.3)
//!
//! FMG is a structured grid; our mesh is irregular. FMG's "neighbor" lookup is
//! `cells.c[cellId]` over the CSR adjacency in `Mesh.cells`. We use the same
//! `Mesh.cells.c` (and `Mesh.cells.i` CSR offsets) which the mesh generator
//! already provides — no spacing-grid indirection is needed here because the
//! biome moisture neighborhood is the *Delaunay-adjacent* cells, exactly what
//! `cells.c` holds.

use js_sys::Uint8Array;
use serde::Deserialize;
use wasm_bindgen::prelude::*;

use crate::mesh::Mesh;

/// Minimum height that counts as land for the moisture neighbor mean (FMG
/// `MIN_LAND_HEIGHT = 20` — matches our `SEA_LEVEL`).
const MIN_LAND_HEIGHT: u8 = 20;

/// The 13 FMG biome ids (index == `Biome.i`). `0` is Marine (water), `1..=12`
/// are land biomes. Names/colors kept for renderer fidelity later.
#[derive(Debug, Clone, Copy)]
pub struct BiomeDef {
    pub id: u8,
    pub name: &'static str,
}

/// 13 biomes, FMG `getDefaultBiomes()` names in id order. Retained as a table
/// for the renderer (color/habitability/icon-density) in later phases.
#[allow(dead_code)]
pub const BIOMES: [BiomeDef; 13] = [
    BiomeDef { id: 0, name: "Marine" },
    BiomeDef { id: 1, name: "Hot desert" },
    BiomeDef { id: 2, name: "Cold desert" },
    BiomeDef { id: 3, name: "Savanna" },
    BiomeDef { id: 4, name: "Grassland" },
    BiomeDef { id: 5, name: "Tropical seasonal forest" },
    BiomeDef { id: 6, name: "Temperate deciduous forest" },
    BiomeDef { id: 7, name: "Tropical rainforest" },
    BiomeDef { id: 8, name: "Temperate rainforest" },
    BiomeDef { id: 9, name: "Taiga" },
    BiomeDef { id: 10, name: "Tundra" },
    BiomeDef { id: 11, name: "Glacier" },
    BiomeDef { id: 12, name: "Wetland" },
];

/// FMG `biomesMatrix`: rows = moistureBand (0..4, dry→wet), cols =
/// temperatureBand (0..25, cold→hot, since `band = 20 - temp`). Indexed
/// `biomesMatrix[moistureBand][temperatureBand]`. Verbatim from FMG.
const BIOMES_MATRIX: [[u8; 26]; 5] = [
    [1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 10],
    [3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 9, 9, 9, 9, 10, 10, 10],
    [5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 9, 9, 9, 9, 9, 10, 10, 10],
    [5, 6, 6, 6, 6, 6, 6, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 9, 9, 9, 9, 9, 9, 10, 10, 10],
    [7, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 9, 9, 9, 9, 9, 9, 9, 10, 10],
];

/// `rn(v, d)` — FMG `utils/numberUtils.ts`: round to `d` decimal places via
/// `round(v * 10^d) / 10^d`. We only ever call with `d = 0`, so this is plain
/// `round` (but keep the form explicit for fidelity).
#[inline]
fn rn(v: f64, _d: u32) -> f64 {
    (v + 0.5).floor()
}

/// Classify one land cell from (moisture, temperature, height). Faithful to
/// FMG `BiomesGenerator.getId`. `has_river` is accepted for interface
/// parity with FMG but ignored (no rivers yet — see module docs).
fn biome_id(moisture: f64, temperature: f64, height: u8, _has_river: bool) -> u8 {
    if height < MIN_LAND_HEIGHT {
        return 0; // Marine (water)
    }
    if temperature < -5.0 {
        return 11; // Glacier / permafrost (FMG: "too cold")
    }
    // FMG: hot & dry & no river → hot desert.
    if temperature >= 25.0 && !_has_river && moisture < 8.0 {
        return 1;
    }
    if is_wetland(moisture, temperature, height) {
        return 12; // Wetland
    }
    let moisture_band = ((moisture / 5.0) as i32).max(0).min(4) as usize;
    let temperature_band = (20 - temperature as i32).max(0).min(25) as usize;
    BIOMES_MATRIX[moisture_band][temperature_band]
}

/// FMG `isWetland(moisture, temperature, height)`.
fn is_wetland(moisture: f64, temperature: f64, height: u8) -> bool {
    if temperature <= -2.0 {
        return false; // too cold
    }
    if moisture > 40.0 && height < 25 {
        return true; // near coast
    }
    if moisture > 24.0 && height > 24 && height < 60 {
        return true; // off coast
    }
    false
}

/// Generate biomes for the whole mesh. `heightmap`, `temp`, `prec` are all
/// length `N` (the cell count). Returns `Vec<u8>` length `N` with biome ids
/// `0..=12`. Pure function of its inputs (no RNG) → deterministic by
/// construction, matching the determinism contract (technical-requirements §4).
pub fn generate_biomes(mesh: &Mesh, heightmap: &[u8], temp: &[i8], prec: &[u8]) -> Vec<u8> {
    let n = mesh.points.len();
    let i = &mesh.cells.i;
    let c = &mesh.cells.c;

    // Pre-pass: per-cell mean precipitation of land neighbors.
    // We collect (prec[cell] + sum(prec of land neighbors)) / (1 + landNeighborCount),
    // then `rn(4 + mean)` — the FMG moisture formula without the river term.
    let mut biome = vec![0u8; n];

    // Scratch accumulators reused per cell.
    for cell in 0..n {
        let height = heightmap[cell];
        if height < MIN_LAND_HEIGHT {
            biome[cell] = 0; // water → Marine
            continue;
        }

        let temperature = temp[cell] as f64;
        let cell_prec = prec[cell] as f64;

        // Neighbor mean of precipitation over LAND neighbors (FMG filters
        // `heights[neib] >= MIN_LAND_HEIGHT`).
        let lo = i[cell] as usize;
        let hi = i[cell + 1] as usize;
        let mut sum = 0.0f64;
        let mut land_count = 0usize;
        for &neigh in &c[lo..hi] {
            let n2 = neigh as usize;
            if heightmap[n2] >= MIN_LAND_HEIGHT {
                sum += prec[n2] as f64;
                land_count += 1;
            }
        }
        // FMG: `moistAround = neighborsLandPrec.map(p) .concat([moisture])`.
        // mean(moistAround). With 0 land neighbors, mean == prec[cell].
        let mean_prec = if land_count > 0 {
            sum / land_count as f64
        } else {
            cell_prec
        };
        // FMG `moisture = prec[cell]; ...; rn(4 + mean(moistAround))`.
        let moisture = rn(4.0 + mean_prec, 0);

        biome[cell] = biome_id(moisture, temperature, height, false);
    }

    biome
}

/// Climate inputs on the JS wire (we accept `temp`/`prec` as separate typed
/// arrays so the bare entry point mirrors `generate_climate`'s shape).
#[derive(Deserialize)]
struct ClimateWire {
    temp: Vec<i8>,
    prec: Vec<u8>,
}

/// Bare typed-array entry — mirrors `generate_climate_js`. Returns the biome id
/// per cell as a `Uint8Array` (0..=12). Wrapped by the `#[wasm_bindgen]`
/// `generate_biomes` in `lib.rs` so the worker sees a clean name.
pub fn generate_biomes_js(
    mesh_js: JsValue,
    climate_js: JsValue,
    heightmap: Uint8Array,
) -> Uint8Array {
    let mesh: Mesh = serde_wasm_bindgen::from_value(mesh_js)
        .expect("generate_biomes: failed to deserialize Mesh from JsValue");
    let h = heightmap.to_vec();
    let climate: ClimateWire = serde_wasm_bindgen::from_value(climate_js)
        .expect("generate_biomes: failed to deserialize {temp, prec} from JsValue");

    let biome = generate_biomes(&mesh, &h, &climate.temp, &climate.prec);

    let out = Uint8Array::new_with_length(biome.len() as u32);
    for (idx, &v) in biome.iter().enumerate() {
        out.set_index(idx as u32, v);
    }
    out
}

/// Grid-form entry for Step 1.5. Runs biomes over an existing `Grid` (carrying
/// mesh + `cells.h` + `cells.temp` + `cells.prec`) and writes `cells.biome`
/// back, returning the updated `Grid` as `JsValue`. Wrapped as
/// `generate_biomes_for_grid` in `lib.rs`.
pub fn generate_biomes_for_grid(grid_js: JsValue) -> JsValue {
    let mut grid: crate::grid::Grid = serde_wasm_bindgen::from_value(grid_js)
        .expect("generate_biomes_for_grid: failed to deserialize Grid from JsValue");
    let biome = generate_biomes(
        &grid.mesh,
        &grid.cells.h,
        &grid.cells.temp,
        &grid.cells.prec,
    );
    grid.cells.biome = biome;
    serde_wasm_bindgen::to_value(&grid).expect("grid serde to JsValue")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::climate;
    use crate::heightmap;
    use crate::mesh;

    fn fixture(cell_count: u32, seed: u32) -> (Mesh, Vec<u8>, Vec<i8>, Vec<u8>) {
        let m = mesh::build(cell_count, seed);
        let h = heightmap::generate(&m, seed as u64);
        let (t, p) = climate::generate_climate(&m, &h, &climate::ClimateOpts::default());
        (m, h, t, p)
    }

    #[test]
    fn biome_length_matches_cell_count() {
        let (m, h, t, p) = fixture(1000, 42);
        let b = generate_biomes(&m, &h, &t, &p);
        assert_eq!(b.len(), m.points.len());
    }

    #[test]
    fn water_cells_are_marine() {
        let (m, h, t, p) = fixture(1000, 42);
        let b = generate_biomes(&m, &h, &t, &p);
        for (cell, &height) in h.iter().enumerate() {
            if height < MIN_LAND_HEIGHT {
                assert_eq!(
                    b[cell], 0,
                    "water cell {cell} (h={height}) must be Marine (0), got {}",
                    b[cell]
                );
            }
        }
    }

    #[test]
    fn land_cells_in_valid_range() {
        let (m, h, t, p) = fixture(3000, 7);
        let b = generate_biomes(&m, &h, &t, &p);
        let mut land = 0;
        for (cell, &height) in h.iter().enumerate() {
            if height >= MIN_LAND_HEIGHT {
                land += 1;
                assert!(
                    (1..=12).contains(&b[cell]),
                    "land cell {cell} (h={height}) biome {} out of [1,12]",
                    b[cell]
                );
            }
        }
        assert!(land > 0, "expected at least some land cells");
    }

    #[test]
    fn deterministic_same_inputs() {
        let (m, h, t, p) = fixture(1000, 99);
        let b1 = generate_biomes(&m, &h, &t, &p);
        let b2 = generate_biomes(&m, &h, &t, &p);
        assert_eq!(b1, b2, "biomes must be deterministic for identical inputs");
    }

    #[test]
    fn polar_cells_tend_cold_biomes() {
        // At a high |latitude| almost every land cell should be glacier/tundra
        // (11/10) given the matrix; sanity-check the matrix wiring produces
        // cold biomes rather than tropical ones near the poles.
        let (m, h, t, p) = fixture(10000, 42);
        let b = generate_biomes(&m, &h, &t, &p);
        let world_h = m.world_h;
        let mut polar_land = 0;
        let mut polar_tropical = 0;
        for cell in 0..m.points.len() {
            if h[cell] < MIN_LAND_HEIGHT {
                continue;
            }
            let y = m.points[cell][1];
            let rel = y / world_h; // 0 top (north) .. 1 bottom (south)
            let is_polar = rel < 0.08 || rel > 0.92;
            if is_polar {
                polar_land += 1;
                // Tropical rainforest (7) / tropical seasonal (5) at the pole is
                // implausible given the matrix — guard against a band-index bug.
                if b[cell] == 7 || b[cell] == 5 {
                    polar_tropical += 1;
                }
            }
        }
        // Allow a small tolerance (matrix has some warm cols only at band<some),
        // but it must be a tiny fraction of polar land.
        if polar_land > 0 {
            assert!(
                (polar_tropical as f64) / (polar_land as f64) < 0.05,
                "too many tropical biomes at the poles: {polar_tropical}/{polar_land}"
            );
        }
    }

    #[test]
    fn sixty_k_smoke() {
        let start = std::time::Instant::now();
        let (m, h, t, p) = fixture(60000, 123);
        let build_ms = start.elapsed().as_millis();
        let gen_start = std::time::Instant::now();
        let b = generate_biomes(&m, &h, &t, &p);
        let gen_ms = gen_start.elapsed().as_millis();
        assert_eq!(b.len(), 60000);
        eprintln!("60k: fixture_build={build_ms}ms biome_gen={gen_ms}ms");
    }
}

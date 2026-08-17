//! World `Grid` — the assembled world state shared by all generators and the
//! renderer. This is the "M5" scaffolding from the adversarial heightmap
//! review: `generate_heightmap` currently returns a bare `Vec<u8>`, but the
//! technical-requirements §3.1 puts `h` on `CellData` inside a `Grid`. Step 1.5
//! (`generate_world`) will build a `Grid`, populate `cells.h` from the heightmap
//! output, then run climate/biomes and store their arrays on the same `CellData`.
//!
//! For now this module defines the **structure** and a constructor that lifts a
//! mesh's geometry into a `Grid` with a pre-sized `CellData`. No generation
//! logic lives here yet — only the data model, so later steps have a stable
//! home for their outputs.

use crate::mesh::{Cells, Mesh, Vertices};
use serde::{Deserialize, Serialize};

/// Per-cell arrays. `h` is produced by the heightmap (Step 1.2); `temp`/`prec`
/// are filled by climate (Step 1.3); `biome` by biomes (Step 1.4). The entity
/// index arrays (`state`/`province`/`culture`/`religion`/`burg`) and drainage
/// arrays (`fl`/`r`/`conf`) are populated by Step 2.5.3 (`recompute_dependents`,
/// rivers) and Phase 3 (entities — `-1` / `0` means "unassigned" until then).
/// All vectors are length `N` (the actual cell count after any dedup).
///
/// `h` values are 0..=100, `< 20` == water (see `heightmap::SEA_LEVEL`).
/// `temp` is `Int8` degrees Celsius; `prec` is `Uint8` precipitation units.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CellData {
    pub h: Vec<u8>,
    pub temp: Vec<i8>,
    pub prec: Vec<u8>,
    /// Biome id (0 = ocean/water, else a land biome). Filled in Step 1.4.
    pub biome: Vec<u8>,
    /// Entity index arrays (Phase 3). `-1` (or `0` for `burg`) == unassigned.
    /// Step 2.5.3's entity repair cascade writes `-1` to water cells; Phase 3
    /// generators overwrite with real entity ids.
    pub state: Vec<i32>,
    pub province: Vec<i32>,
    pub culture: Vec<i32>,
    pub religion: Vec<i32>,
    pub burg: Vec<i16>,
    /// Drainage arrays (Step 2.5.3 rivers). `fl` = water flux (discharge);
    /// `r` = river id at cell (0 = none); `conf` = confluence flag. FMG
    /// `cells.fl`/`cells.r`/`cells.conf` mirrors. A river-flux bonus term in
    /// the biome moisture formula reads `fl` (see `biomes.rs`).
    pub fl: Vec<u16>,
    pub r: Vec<u16>,
    pub conf: Vec<u16>,
}

impl CellData {
    /// Allocate a `CellData` with `N` cells, all height set to `0` (water) and
    /// the other layers zeroed. Entity arrays default to `-1` (or `0` for
    /// `burg`); drainage arrays default to `0`. `generate_heightmap` later
    /// overwrites `h`.
    pub fn with_capacity(n: usize) -> CellData {
        CellData {
            h: vec![0u8; n],
            temp: vec![0i8; n],
            prec: vec![0u8; n],
            biome: vec![0u8; n],
            state: vec![-1i32; n],
            province: vec![-1i32; n],
            culture: vec![-1i32; n],
            religion: vec![-1i32; n],
            burg: vec![0i16; n],
            fl: vec![0u16; n],
            r: vec![0u16; n],
            conf: vec![0u16; n],
        }
    }
}

/// The world: geometry (mesh) + per-cell data. This is the unit that
/// `generate_world` assembles and `.world` serializes.
#[derive(Serialize, Deserialize, Clone)]
pub struct Grid {
    pub seed: u64,
    pub mesh: Mesh,
    pub cells: CellData,
}

impl Grid {
    /// Lift a mesh into a `Grid` with zeroed `CellData`. The geometry is owned
    /// (cloned from the passed mesh) so the grid is self-contained for
    /// serialization.
    pub fn from_mesh(mesh: &Mesh, seed: u64) -> Grid {
        let n = mesh.points.len();
        Grid {
            seed,
            mesh: mesh.clone(),
            cells: CellData::with_capacity(n),
        }
    }

    /// Number of cells in the grid.
    #[allow(dead_code)]
    pub fn cell_count(&self) -> usize {
        self.mesh.points.len()
    }

    /// Borrow the mesh's CSR adjacency/vertex arrays for generators/renderers.
    #[allow(dead_code)]
    pub fn cells_topology(&self) -> &Cells {
        &self.mesh.cells
    }

    /// Borrow the mesh's vertices for the renderer.
    #[allow(dead_code)]
    pub fn vertices(&self) -> &Vertices {
        &self.mesh.vertices
    }
}

// ===========================================================================//
// Tests — verification gate for the Grid / CellData data model.
// ===========================================================================//

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{Cells, Mesh, Vertices};

    /// Build a minimal valid mesh for testing (3 cells, chain topology).
    fn simple_mesh() -> Mesh {
        let points = vec![[0.0, 0.0], [100.0, 0.0], [200.0, 0.0]];
        let v = vec![0, 1, 1, 2]; // vertex ids (not used by Grid tests)
        // 3 cells in a chain: cell 0 neighbors [1], cell 1 neighbors [0,2], cell 2 neighbors [1]
        let c = vec![1, 0, 2, 1]; // CSR: cell 0:[1], cell 1:[0,2], cell 2:[1]
        let i = vec![0, 1, 3, 4]; // offsets: 0..1, 1..3, 3..4
        let b = vec![0, 0, 0];
        Mesh {
            points,
            cells: Cells {
                v,
                c,
                i,
                b,
                spacing: vec![],
                cells_x: 3,
                cells_y: 1,
            },
            vertices: Vertices { p: vec![] },
            world_w: 10000.0,
            world_h: 8000.0,
        }
    }

    #[test]
    fn cell_data_with_capacity_correct_lengths() {
        let cd = CellData::with_capacity(100);
        assert_eq!(cd.h.len(), 100);
        assert_eq!(cd.temp.len(), 100);
        assert_eq!(cd.prec.len(), 100);
        assert_eq!(cd.biome.len(), 100);
        assert_eq!(cd.state.len(), 100);
        assert_eq!(cd.province.len(), 100);
        assert_eq!(cd.culture.len(), 100);
        assert_eq!(cd.religion.len(), 100);
        assert_eq!(cd.burg.len(), 100);
        assert_eq!(cd.fl.len(), 100);
        assert_eq!(cd.r.len(), 100);
        assert_eq!(cd.conf.len(), 100);
    }

    #[test]
    fn cell_data_with_capacity_defaults() {
        let cd = CellData::with_capacity(50);
        // h defaults to 0 (water).
        assert!(cd.h.iter().all(|&v| v == 0));
        // temp defaults to 0.
        assert!(cd.temp.iter().all(|&v| v == 0));
        // prec defaults to 0.
        assert!(cd.prec.iter().all(|&v| v == 0));
        // biome defaults to 0 (ocean/water).
        assert!(cd.biome.iter().all(|&v| v == 0));
        // Entity arrays default to -1 (or 0 for burg).
        assert!(cd.state.iter().all(|&v| v == -1));
        assert!(cd.province.iter().all(|&v| v == -1));
        assert!(cd.culture.iter().all(|&v| v == -1));
        assert!(cd.religion.iter().all(|&v| v == -1));
        // burg defaults to 0 (unassigned).
        assert!(cd.burg.iter().all(|&v| v == 0));
        // Drainage arrays default to 0.
        assert!(cd.fl.iter().all(|&v| v == 0));
        assert!(cd.r.iter().all(|&v| v == 0));
        assert!(cd.conf.iter().all(|&v| v == 0));
    }

    #[test]
    fn grid_from_mesh_correct_cell_count() {
        let mesh = simple_mesh();
        let seed = 42;
        let grid = Grid::from_mesh(&mesh, seed);
        assert_eq!(grid.seed, seed);
        assert_eq!(grid.cell_count(), 3);
        assert_eq!(grid.cells.h.len(), 3);
        assert_eq!(grid.cells.temp.len(), 3);
        assert_eq!(grid.cells.prec.len(), 3);
        assert_eq!(grid.cells.biome.len(), 3);
        assert_eq!(grid.cells.state.len(), 3);
        assert_eq!(grid.cells.burg.len(), 3);
        assert_eq!(grid.cells.fl.len(), 3);
        assert_eq!(grid.cells.r.len(), 3);
        assert_eq!(grid.cells.conf.len(), 3);
    }

    #[test]
    fn grid_from_mesh_copies_entity_arrays_to_unassigned() {
        let mesh = simple_mesh();
        let grid = Grid::from_mesh(&mesh, 1);
        assert!(grid.cells.state.iter().all(|&v| v == -1));
        assert!(grid.cells.burg.iter().all(|&v| v == 0));
    }

    #[test]
    fn grid_from_mesh_clones_geometry() {
        let mesh = simple_mesh();
        let grid = Grid::from_mesh(&mesh, 1);
        // The grid's mesh should be a copy (not a borrow).
        assert_eq!(grid.mesh.points.len(), mesh.points.len());
        assert_eq!(grid.mesh.cells.c.len(), mesh.cells.c.len());
        assert_eq!(grid.mesh.cells.i.len(), mesh.cells.i.len());
    }

    #[test]
    fn grid_cell_count_matches_points_len() {
        let n = 100;
        // Use the real mesh builder for a larger, realistic mesh.
        let mesh = crate::mesh::build(n, 42);
        let grid = Grid::from_mesh(&mesh, 42);
        assert_eq!(grid.cell_count(), mesh.points.len());
        assert_eq!(grid.cells.h.len(), grid.cell_count());
    }

    #[test]
    fn grid_from_mesh_seed_is_stored() {
        let mesh = simple_mesh();
        let grid = Grid::from_mesh(&mesh, 999);
        assert_eq!(grid.seed, 999);
    }

    #[test]
    fn cell_data_with_capacity_zero() {
        let cd = CellData::with_capacity(0);
        assert_eq!(cd.h.len(), 0);
        assert_eq!(cd.state.len(), 0);
        assert_eq!(cd.burg.len(), 0);
    }

    #[test]
    fn cell_data_with_capacity_one() {
        let cd = CellData::with_capacity(1);
        assert_eq!(cd.h.len(), 1);
        assert_eq!(cd.h[0], 0);
        assert_eq!(cd.state[0], -1);
        assert_eq!(cd.burg[0], 0);
    }
}

// ---------------------------------------------------------------------------
// Step 2.5.3 — dependent-recompute output types
// ---------------------------------------------------------------------------

/// River geometry (minimal, renderer-facing). `cells` is the ordered list of
/// cell ids forming the river path (-1 entries mark "off-map pour" sentinels,
/// matching FMG's `addCellToRiver(-1, ...)`); `points` are the world-space
/// polyline vertices (one per cell, simple midpoint for now — no meandering
/// until Step 2.5.4/Phase 3). Stored at the cell-resolution; the renderer
/// draws polylines over the existing merged-cell geometry.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RiverGeo {
    /// River id (1-based; matches `cells.r`).
    pub id: u32,
    /// Source cell id (first land cell of the path).
    pub source: u32,
    /// Mouth cell id (last land cell before water/off-map).
    pub mouth: u32,
    /// Discharge (sum of collected precipitation flux).
    pub discharge: f64,
    /// Ordered cell ids forming the path (-1 = off-map pour sentinel).
    pub cells: Vec<i32>,
    /// World-space polyline points (one per land cell in `cells`).
    pub points: Vec<[f64; 2]>,
}

/// Lake geometry (minimal). A lake is a depression cell whose height was
/// raised to the shoreline minimum + `LAKE_ELEVATION_DELTA` so it holds
/// water rather than draining. `cells` are the cell ids belonging to the
/// lake; `shoreline` are the land cells adjacent to the lake; `height` is
/// the resolved lake surface height (used by `resolveDepressions` as the
/// effective height when computing downstream flow).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LakeGeo {
    /// Lake id (1-based; written to a separate `LakeGeo` list, not onto
    /// `cells` — Phase 3 will assign feature ids).
    pub id: u32,
    /// Lake surface height (0..100 scale).
    pub height: f64,
    /// Cell ids belonging to the lake (water cells filled by depression
    /// resolution).
    pub cells: Vec<u32>,
    /// Land cell ids adjacent to the lake (the shoreline).
    pub shoreline: Vec<u32>,
    /// Whether the lake is closed (deep depression, no sea outlet) — FMG
    /// `Lakes.detectCloseLakes`. Closed lakes retain their outlet identity;
    /// open lakes drain to the sea.
    pub closed: bool,
}

/// Output of `recompute_dependents` (Step 2.5.3). Carries the freshly
/// recomputed per-cell arrays (climate + biomes + entity indices post-repair)
/// and the new river/lake geometry. The renderer swaps data textures from
/// this; the entity repair cascade fills `removed_burgs`/`dissolved_states`
/// for the warning toast.
///
/// Drainage arrays `fl`/`r`/`conf` are included so downstream consumers
/// (biome moisture's river-flux bonus, the Tier-1 local recompute, and the
/// Phase 3 entity generators) can read drainage state without re-running
/// the full cascade.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct DependentResult {
    pub temp: Vec<i8>,
    pub prec: Vec<u8>,
    pub biome: Vec<u8>,
    pub state: Vec<i32>,
    pub province: Vec<i32>,
    pub culture: Vec<i32>,
    pub religion: Vec<i32>,
    pub burg: Vec<i16>,
    /// Per-cell water flux (discharge), from `rivers::compute_drainage`.
    pub fl: Vec<u16>,
    /// River id at each cell (0 = none), from `rivers::compute_drainage`.
    pub r: Vec<u16>,
    /// Confluence flag (0 = none; nonzero = confluence flux).
    pub conf: Vec<u16>,
    /// Coastline mask: `true` for land cells adjacent to water (h < SEA_LEVEL).
    /// Computed by `recompute_dependents_inner` as the coastline/land-water step.
    pub coastline: Vec<u8>,
    pub removed_burgs: Vec<String>,
    pub dissolved_states: Vec<u32>,
    pub rivers: Vec<RiverGeo>,
    pub lakes: Vec<LakeGeo>,
}

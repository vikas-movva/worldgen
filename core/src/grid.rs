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
/// are filled by climate (Step 1.3); `biome` by biomes (Step 1.4). All vectors
/// are length `N` (the actual cell count after any dedup).
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
}

impl CellData {
    /// Allocate a `CellData` with `N` cells, all height set to `0` (water) and
    /// the other layers zeroed. `generate_heightmap` later overwrites `h`.
    pub fn with_capacity(n: usize) -> CellData {
        CellData {
            h: vec![0u8; n],
            temp: vec![0i8; n],
            prec: vec![0u8; n],
            biome: vec![0u8; n],
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

//! Heightmap editor — Step 2.5.1 (Phase 2.5).
//!
//! Brush + macro editing of `cells.h` in the Rust core. All ops are pure
//! functions over `h` + `cells.c` adjacency (no RNG needed except the seeded
//! macro tools which use the world `seed`). Determinism: the same `EditOp[]`
//! applied to the same `grid` yields byte-identical `h`.
//!
//! See `worldgen-technical-requirements.md` §3.5 for the contract.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use crate::grid::Grid;
use crate::heightmap::SEA_LEVEL;
use crate::heightmap::{build_range, get_line_power, MeshView};
use crate::mesh::Mesh;
use rand::{rngs::StdRng, SeedableRng};

/// Clamp a float into `[0, 100]` then round to nearest `u8` (same as `heightmap::lim`).
fn lim(v: f64) -> u8 {
    v.clamp(0.0, 100.0).round() as u8
}

/// Brush / macro edit modes (matches `EditMode` in technical-requirements §3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditMode {
    Raise,
    Lower,
    Flatten,
    Smooth,
    Range,
    Trough,
    Strait,
    Mask,
    Invert,
    Add,
    Multiply,
}

/// One edit operation. `cells` is the set of affected cell ids (brush = radius
/// query; macro = path). For brush modes, `radius` and `strength` are used;
/// for macros, `strength` may be a multiplier/offset. `target_cell` is used by
/// Range/Trough as the ridge walk endpoint (FMG `addRange`/`addTrough`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditOp {
    pub mode: EditMode,
    pub center_cell: u32,
    pub target_cell: u32,
    pub radius: f32,
    pub strength: f32,
    pub cells: Vec<u32>,
}

// ---------------------------------------------------------------------------
// Brush helpers
// ---------------------------------------------------------------------------

/// Gather all cells within `radius` (in world units) of `center_cell` using
/// BFS over `cells.c` adjacency, stopping when the Euclidean distance from the
/// center cell's position exceeds `radius`. Returns the set including the
/// center cell. Deterministic (no RNG).
fn gather_radius_cells(mesh: &Mesh, center_cell: u32, radius: f32) -> Vec<u32> {
    let center = center_cell as usize;
    let r2 = (radius as f64) * (radius as f64);
    let [cx, cy] = mesh.points[center];
    let i = &mesh.cells.i;
    let c = &mesh.cells.c;
    let n = mesh.points.len();
    let mut visited = vec![false; n];
    let mut queue: Vec<usize> = Vec::with_capacity(64);
    let mut out: Vec<u32> = Vec::with_capacity(64);
    visited[center] = true;
    queue.push(center);
    out.push(center as u32);
    while let Some(q) = queue.pop() {
        let lo = i[q] as usize;
        let hi = i[q + 1] as usize;
        for &nbor in &c[lo..hi] {
            let nb = nbor as usize;
            if visited[nb] {
                continue;
            }
            let [px, py] = mesh.points[nb];
            let dx = px - cx;
            let dy = py - cy;
            if dx * dx + dy * dy <= r2 {
                visited[nb] = true;
                queue.push(nb);
                out.push(nb as u32);
            }
        }
    }
    out
}

/// Radial falloff: `(1 - d²/r²)` clamped to [0, 1], where d is distance from
/// center. At center (d=0) → 1.0; at edge (d=r) → 0.0.
fn falloff(distance_sq: f64, radius: f32) -> f64 {
    let r2 = (radius as f64) * (radius as f64);
    if r2 <= 0.0 || distance_sq >= r2 {
        return 0.0;
    }
    1.0 - distance_sq / r2
}

// ---------------------------------------------------------------------------
// Brush ops (raise / lower / flatten / smooth)
// ---------------------------------------------------------------------------

/// Apply a brush `op` to `h`. For Raise/Lower/Flatten/Smooth, `op.cells` should
/// be the radius-bounded cell set (caller may pre-compute or we gather from
/// `center_cell`+`radius` if `cells` is empty).
fn apply_brush(mesh: &Mesh, h: &mut [u8], op: &EditOp) {
    let cells: Vec<u32> = if op.cells.is_empty() {
        gather_radius_cells(mesh, op.center_cell, op.radius)
    } else {
        op.cells.clone()
    };
    let [cx, cy] = mesh.points[op.center_cell as usize];
    let strength = op.strength as f64;
    match op.mode {
        EditMode::Raise => {
            for &cid in &cells {
                let [px, py] = mesh.points[cid as usize];
                let dx = px - cx;
                let dy = py - cy;
                let f = falloff(dx * dx + dy * dy, op.radius);
                h[cid as usize] = lim(h[cid as usize] as f64 + strength * f * 100.0);
            }
        }
        EditMode::Lower => {
            for &cid in &cells {
                let [px, py] = mesh.points[cid as usize];
                let dx = px - cx;
                let dy = py - cy;
                let f = falloff(dx * dx + dy * dy, op.radius);
                h[cid as usize] = lim(h[cid as usize] as f64 - strength * f * 100.0);
            }
        }
        EditMode::Flatten => {
            // Flatten: blend each cell toward the center cell's height.
            let target = h[op.center_cell as usize] as f64;
            for &cid in &cells {
                let [px, py] = mesh.points[cid as usize];
                let dx = px - cx;
                let dy = py - cy;
                let f = falloff(dx * dx + dy * dy, op.radius);
                let blend = strength * f;
                h[cid as usize] = lim(h[cid as usize] as f64 * (1.0 - blend) + target * blend);
            }
        }
        EditMode::Smooth => {
            // Smooth: blend each cell toward the mean of its neighbors.
            let snapshot = h.to_vec();
            let n = h.len();
            for &cid in &cells {
                let ci = cid as usize;
                let lo = mesh.cells.i[ci] as usize;
                let hi = mesh.cells.i[ci + 1] as usize;
                let mut sum = snapshot[ci] as f64;
                let mut count = 1usize;
                for &nb in &mesh.cells.c[lo..hi] {
                    sum += snapshot[nb as usize] as f64;
                    count += 1;
                }
                let mean = sum / count as f64;
                let [px, py] = mesh.points[ci];
                let dx = px - cx;
                let dy = py - cy;
                let f = falloff(dx * dx + dy * dy, op.radius);
                let blend = strength * f;
                h[ci] = lim(h[ci] as f64 * (1.0 - blend) + mean * blend);
            }
            let _ = n;
        }
        // Macro modes are handled by `apply_macro`.
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Macro ops (Range / Trough / Strait / Mask / Invert / Add / Multiply)
// ---------------------------------------------------------------------------

/// Apply a macro `op` to `h`. These are pure functions over `h` + `cells.c`
/// adjacency. For `Range`/`Trough`, the path is built greedily from
/// `center_cell` → `target_cell` (FMG `getRange`) using `grid.seed`, then spread
/// outward with `linePower` decay (FMG `addRange`/`addTrough`). For other
/// macros, `op.cells` is the explicit cell set.
fn apply_macro(mesh: &Mesh, h: &mut [u8], op: &EditOp, grid_seed: u64) {
    // Area macros (Strait/Mask/Invert/Add/Multiply) operate over `op.cells`.
    // When the caller leaves it empty (the editor hot path), gather the
    // radius-bounded neighborhood around `center_cell` — identical to how the
    // brush tools gather, so a single click with a brush radius produces a
    // useful macro edit instead of a no-op. Range/Trough ignore this set and
    // build their own ridge path from center → target.
    let area_cells: Vec<u32> = if op.cells.is_empty() {
        gather_radius_cells(mesh, op.center_cell, op.radius)
    } else {
        op.cells.clone()
    };
    match op.mode {
        EditMode::Range | EditMode::Trough => {
            // FMG `addRange`/`addTrough` port: greedy walk from center to target,
            // then BFS spread outward with `linePower` decay. Uses the world seed
            // (per §3.5 determinism nuance: "macro tools that use RNG must use
            // the world seed, not a fresh RNG").
            let raise = op.mode == EditMode::Range;
            let view = MeshView::from_mesh(mesh);
            let line_power = get_line_power(h.len());
            let mut rng = StdRng::seed_from_u64(grid_seed);
            let max_cells = h.len().clamp(50, 2000);

            // Greedy ridge walk from center to target.
            let range = build_range(
                &view,
                &mut rng,
                op.center_cell as usize,
                op.target_cell as usize,
                0.0, // no randomness in editor ops (deterministic strokes)
            );

            // Initial ridge height scales with strength (FMG: 30-60 for raise,
            // 20-30 for trough). We use strength as a 0..1 multiplier.
            let base_height = if raise { 45.0 } else { 25.0 };
            let mut height = lim(base_height * op.strength as f64);

            let n = h.len();
            let mut used = vec![false; n];
            let mut queue: Vec<usize> = range.clone();
            for q in &queue {
                used[*q] = true;
            }
            let mut painted = range.len();
            while !queue.is_empty() && painted < max_cells {
                let frontier = queue.clone();
                queue = Vec::new();
                for &i in &frontier {
                    let add = height as f64 * 0.85; // FMG: gen_range(0.0..0.3) + 0.85; deterministic 0.85
                    h[i] = lim(h[i] as f64 + if raise { add } else { -add });
                    painted += 1;
                    if painted >= max_cells {
                        break;
                    }
                }
                if painted >= max_cells {
                    break;
                }
                // FMG: h = h * linePower - 1; break if h < 2.
                height = lim(height as f64 * line_power - 1.0);
                if height < 2 {
                    break;
                }
                for &f in &frontier {
                    let lo = view.cells.i[f] as usize;
                    let hi = view.cells.i[f + 1] as usize;
                    for &i in &view.cells.c[lo..hi] {
                        let i = i as usize;
                        if !used[i] {
                            queue.push(i);
                            used[i] = true;
                        }
                    }
                }
            }

            // Prominence carving: every 6th ridge cell, descend to lowest
            // neighbor and blend (FMG `addRange` prominence pass).
            let prominence_depth = range.len();
            for (d, &cur0) in range.iter().enumerate() {
                if d % 6 != 0 {
                    continue;
                }
                let mut cur = cur0;
                for _ in 0..prominence_depth {
                    let lo = view.cells.i[cur] as usize;
                    let hi = view.cells.i[cur + 1] as usize;
                    if hi <= lo {
                        break;
                    }
                    let mut min_idx = lo;
                    let mut min_h = h[view.cells.c[lo] as usize];
                    for k in lo + 1..hi {
                        let cand = view.cells.c[k] as usize;
                        if h[cand] < min_h {
                            min_h = h[cand];
                            min_idx = k;
                        }
                    }
                    let min = view.cells.c[min_idx] as usize;
                    h[min] = lim((h[cur] as f64 * 2.0 + h[min] as f64) / 3.0);
                    cur = min;
                }
            }
        }
        EditMode::Strait => {
            // Carve a band: lower all cells in the path toward sea level.
            let strength = op.strength as f64;
            for &cid in &area_cells {
                let ci = cid as usize;
                let target = SEA_LEVEL as f64;
                h[ci] = lim(h[ci] as f64 * (1.0 - strength) + target * strength);
            }
        }
        EditMode::Mask => {
            // Radial falloff mask: `(1 - nx²)(1 - ny²)` over the affected cells.
            let strength = op.strength as f64;
            let snapshot = h.to_vec();
            let [cx, cy] = mesh.points[op.center_cell as usize];
            // Guard against a zero radius (division by zero below); a single
            // cell collapses to a distance 0 mask, which is a no-op.
            let radius = if op.radius <= 0.0 { 1.0 } else { op.radius as f64 };
            for &cid in &area_cells {
                let ci = cid as usize;
                let [px, py] = mesh.points[ci];
                let nx = (px - cx) / radius;
                let ny = (py - cy) / radius;
                let dist = (1.0 - nx * nx) * (1.0 - ny * ny);
                let masked = snapshot[ci] as f64 * dist.max(0.0);
                h[ci] = lim(snapshot[ci] as f64 * (1.0 - strength) + masked * strength);
            }
        }
        EditMode::Invert => {
            // Mirror h across sea level: water <-> land.
            for &cid in &area_cells {
                let ci = cid as usize;
                h[ci] = lim(100.0 - h[ci] as f64);
            }
        }
        EditMode::Add => {
            // Add a constant offset to all affected cells.
            let offset = op.strength as f64 * 100.0;
            for &cid in &area_cells {
                let ci = cid as usize;
                h[ci] = lim(h[ci] as f64 + offset);
            }
        }
        EditMode::Multiply => {
            // Multiply all affected cells by a factor (around sea level for land).
            let mult = op.strength as f64;
            for &cid in &area_cells {
                let ci = cid as usize;
                let v = h[ci] as f64;
                if v >= SEA_LEVEL as f64 {
                    h[ci] = lim((v - SEA_LEVEL as f64) * mult + SEA_LEVEL as f64);
                } else {
                    h[ci] = lim(v * mult);
                }
            }
        }
        // Brush modes handled elsewhere.
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Apply a batch of edit ops to `grid.cells.h` in place. Deterministic: same
/// `grid` + same `ops` → byte-identical `h`. Exposed as
/// `edit_heightmap(grid, ops)` to JS via `lib.rs`.
pub fn edit_heightmap(grid: &mut Grid, ops: &[EditOp]) {
    for op in ops {
        match op.mode {
            EditMode::Raise | EditMode::Lower | EditMode::Flatten | EditMode::Smooth => {
                apply_brush(&grid.mesh, &mut grid.cells.h, op);
            }
            _ => {
                apply_macro(&grid.mesh, &mut grid.cells.h, op, grid.seed);
            }
        }
    }
}

/// Inner implementation: takes a `Grid` (deserialized from JS) + a JSON
/// array of `EditOp` and returns the updated `Grid` with mutated `cells.h`.
/// Called by `lib.rs::edit_heightmap` (the `#[wasm_bindgen]` entry point).
pub fn edit_heightmap_js(grid_js: JsValue, ops_js: JsValue) -> JsValue {
    let mut grid: Grid = serde_wasm_bindgen::from_value(grid_js)
        .expect("edit_heightmap: failed to deserialize Grid");
    let ops: Vec<EditOp> = serde_wasm_bindgen::from_value(ops_js)
        .expect("edit_heightmap: failed to deserialize EditOp[]");
    edit_heightmap(&mut grid, &ops);
    serde_wasm_bindgen::to_value(&grid).expect("edit_heightmap: grid serde to JsValue")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh;

    fn test_grid(n: u32, seed: u32) -> Grid {
        let mesh = mesh::build(n, seed);
        let mut grid = Grid::from_mesh(&mesh, seed as u64);
        // Start with a flat heightmap at h=50 (land).
        grid.cells.h = vec![50u8; grid.mesh.points.len()];
        grid
    }

    /// `Raise` increases `h` at the center, falls off toward the radius edge,
    /// and clamps to [0, 100].
    #[test]
    fn raise_increases_center_with_falloff() {
        let mut grid = test_grid(2000, 42);
        let center = 500u32;
        let radius = 500.0f32; // world units — generous for 2000 cells
        let cells = gather_radius_cells(&grid.mesh, center, radius);
        let op = EditOp {
            mode: EditMode::Raise,
            center_cell: center,
            target_cell: 0,
            radius,
            strength: 0.5,
            cells: cells.clone(),
        };
        let before = grid.cells.h[center as usize];
        edit_heightmap(&mut grid, &[op]);
        let after = grid.cells.h[center as usize];
        assert!(
            after > before,
            "raise should increase center: {before} -> {after}"
        );
        assert!(after <= 100, "raise clamps to 100: {after}");
        // Edge cells (farthest from center) should be raised less than center.
        let [cx, cy] = grid.mesh.points[center as usize];
        let mut max_dist_sq = 0.0f64;
        let mut farthest = center;
        for &cid in &cells {
            let [px, py] = grid.mesh.points[cid as usize];
            let d = (px - cx).powi(2) + (py - cy).powi(2);
            if d > max_dist_sq {
                max_dist_sq = d;
                farthest = cid;
            }
        }
        if farthest != center {
            // The farthest cell should have been raised less than the center.
            // (It started at 50, center is now >50; farthest should be <= center.)
            assert!(
                grid.cells.h[farthest as usize] <= grid.cells.h[center as usize],
                "falloff: farthest {} should be <= center {}",
                grid.cells.h[farthest as usize],
                grid.cells.h[center as usize]
            );
        }
    }

    /// `Lower` decreases `h` at the center, clamps to 0.
    #[test]
    fn lower_decreases_center() {
        let mut grid = test_grid(2000, 42);
        let center = 100u32;
        let radius = 500.0;
        let cells = gather_radius_cells(&grid.mesh, center, radius);
        let before = grid.cells.h[center as usize];
        let op = EditOp {
            mode: EditMode::Lower,
            center_cell: center,
            target_cell: 0,
            radius,
            strength: 0.5,
            cells,
        };
        edit_heightmap(&mut grid, &[op]);
        let after = grid.cells.h[center as usize];
        assert!(
            after < before,
            "lower should decrease center: {before} -> {after}"
        );
        // Extreme lower should clamp to 0.
        let mut grid2 = test_grid(2000, 42);
        let op2 = EditOp {
            mode: EditMode::Lower,
            center_cell: center,
            target_cell: 0,
            radius,
            strength: 1.0,
            cells: vec![center],
        };
        edit_heightmap(&mut grid2, &[op2]);
        assert_eq!(grid2.cells.h[center as usize], 0, "lower clamps to 0");
    }

    /// `Smooth` reduces local variance.
    #[test]
    fn smooth_reduces_variance() {
        let mut grid = test_grid(2000, 42);
        // Create a spike: center at 100, all others at 50.
        let center = 300u32;
        grid.cells.h[center as usize] = 100;
        let radius = 800.0;
        let cells = gather_radius_cells(&grid.mesh, center, radius);
        let before_var = variance(&grid.cells.h);
        let op = EditOp {
            mode: EditMode::Smooth,
            center_cell: center,
            target_cell: 0,
            radius,
            strength: 1.0,
            cells,
        };
        edit_heightmap(&mut grid, &[op]);
        let after_var = variance(&grid.cells.h);
        assert!(
            after_var <= before_var,
            "smooth should not increase variance: {before_var} -> {after_var}"
        );
    }

    /// `Flatten` blends cells toward the center cell's height.
    #[test]
    fn flatten_blends_toward_center() {
        let mut grid = test_grid(2000, 42);
        let center = 200u32;
        grid.cells.h[center as usize] = 80; // center is high
        let radius = 500.0;
        let cells = gather_radius_cells(&grid.mesh, center, radius);
        let op = EditOp {
            mode: EditMode::Flatten,
            center_cell: center,
            target_cell: 0,
            radius,
            strength: 1.0,
            cells: cells.clone(),
        };
        edit_heightmap(&mut grid, &[op]);
        // After full-strength flatten, center stays 80; nearby cells move toward 80.
        assert_eq!(
            grid.cells.h[center as usize], 80,
            "flatten center unchanged"
        );
        // At least one neighbor should have moved toward 80 (from 50).
        let lo = grid.mesh.cells.i[center as usize] as usize;
        let hi = grid.mesh.cells.i[center as usize + 1] as usize;
        let any_moved = grid.mesh.cells.c[lo..hi]
            .iter()
            .any(|&nb| grid.cells.h[nb as usize] > 50);
        assert!(any_moved, "flatten should pull neighbors toward center");
    }

    /// Detop: same grid + same ops → byte-identical `h`.
    #[test]
    fn deterministic_same_ops() {
        let make = || test_grid(3000, 99);
        let ops = vec![
            EditOp {
                mode: EditMode::Raise,
                center_cell: 100,
                target_cell: 0,
                radius: 500.0,
                strength: 0.3,
                cells: vec![],
            },
            EditOp {
                mode: EditMode::Smooth,
                center_cell: 200,
                target_cell: 0,
                radius: 600.0,
                strength: 0.7,
                cells: vec![],
            },
            EditOp {
                mode: EditMode::Lower,
                center_cell: 50,
                target_cell: 0,
                radius: 400.0,
                strength: 0.5,
                cells: vec![],
            },
        ];
        let mut a = make();
        let mut b = make();
        edit_heightmap(&mut a, &ops);
        edit_heightmap(&mut b, &ops);
        assert_eq!(a.cells.h, b.cells.h, "edit_heightmap must be deterministic");
    }

    /// `Add` adds a constant offset, clamped to [0, 100].
    #[test]
    fn add_offset_clamps() {
        let mut grid = test_grid(1000, 42);
        let cells: Vec<u32> = (0..grid.mesh.points.len() as u32).collect();
        let op = EditOp {
            mode: EditMode::Add,
            center_cell: 0,
            target_cell: 0,
            radius: 0.0,
            strength: 1.0, // offset = 1.0 * 100 = 100 → 50 + 100 = clamp 100
            cells: cells.clone(),
        };
        edit_heightmap(&mut grid, &[op]);
        for &cid in &cells {
            assert_eq!(grid.cells.h[cid as usize], 100, "add clamps to 100");
        }
    }

    /// `Multiply` scales land cells around sea level.
    #[test]
    fn multiply_scales_around_sea_level() {
        let mut grid = test_grid(1000, 42);
        // Set some cells to known land heights.
        for i in 0..grid.cells.h.len() {
            grid.cells.h[i] = 40; // land (≥20), 20 above sea level
        }
        let cells: Vec<u32> = (0..grid.cells.h.len() as u32).collect();
        let op = EditOp {
            mode: EditMode::Multiply,
            center_cell: 0,
            target_cell: 0,
            radius: 0.0,
            strength: 2.0, // (40 - 20) * 2 + 20 = 60
            cells,
        };
        edit_heightmap(&mut grid, &[op]);
        for &h in &grid.cells.h {
            assert_eq!(h, 60, "multiply: (40-20)*2 + 20 = 60, got {h}");
        }
    }

    /// `Invert` mirrors h across 50 (not exactly sea level, but 100 - h).
    #[test]
    fn invert_mirrors_height() {
        let mut grid = test_grid(1000, 42);
        for i in 0..grid.cells.h.len() {
            grid.cells.h[i] = 30;
        }
        let cells: Vec<u32> = (0..grid.cells.h.len() as u32).collect();
        let op = EditOp {
            mode: EditMode::Invert,
            center_cell: 0,
            target_cell: 0,
            radius: 0.0,
            strength: 0.0,
            cells,
        };
        edit_heightmap(&mut grid, &[op]);
        for &h in &grid.cells.h {
            assert_eq!(h, 70, "invert: 100 - 30 = 70, got {h}");
        }
    }

    /// `gather_radius_cells` returns the center cell and is deterministic.
    #[test]
    fn gather_radius_includes_center_and_is_deterministic() {
        let mesh = mesh::build(2000, 42);
        let a = gather_radius_cells(&mesh, 500, 500.0);
        let b = gather_radius_cells(&mesh, 500, 500.0);
        assert_eq!(a, b, "gather must be deterministic");
        assert!(a.contains(&500), "must include center");
        assert!(!a.is_empty(), "must return at least the center");
    }

    /// `Range` raises cells along the ridge path from center to target,
    /// and the start cell is raised. Deterministic across re-runs.
    #[test]
    fn range_raises_along_path() {
        let mut grid = test_grid(2000, 42);
        let center = 100u32;
        let target = 1900u32; // opposite corner
        let before = grid.cells.h[center as usize];
        let op = EditOp {
            mode: EditMode::Range,
            center_cell: center,
            target_cell: target,
            radius: 0.0,
            strength: 1.0,
            cells: vec![],
        };
        edit_heightmap(&mut grid, std::slice::from_ref(&op));
        let after = grid.cells.h[center as usize];
        assert!(
            after > before,
            "range should raise start: {before} -> {after}"
        );
        // Determinism: same grid + same op → byte-identical h.
        let mut grid2 = test_grid(2000, 42);
        edit_heightmap(&mut grid2, &[op]);
        assert_eq!(grid.cells.h, grid2.cells.h, "range must be deterministic");
        // All h in [0, 100]
        for &h in &grid.cells.h {
            assert!(h <= 100, "range h out of bounds: {h}");
        }
    }

    /// `Trough` lowers cells along the ridge path, and the start cell is
    /// lowered. Deterministic across re-runs.
    #[test]
    fn trough_lowers_along_path() {
        let mut grid = test_grid(2000, 42);
        let center = 100u32;
        let target = 1900u32;
        let before = grid.cells.h[center as usize];
        let op = EditOp {
            mode: EditMode::Trough,
            center_cell: center,
            target_cell: target,
            radius: 0.0,
            strength: 1.0,
            cells: vec![],
        };
        edit_heightmap(&mut grid, std::slice::from_ref(&op));
        let after = grid.cells.h[center as usize];
        assert!(
            after < before,
            "trough should lower start: {before} -> {after}"
        );
        // Determinism.
        let mut grid2 = test_grid(2000, 42);
        edit_heightmap(&mut grid2, &[op]);
        assert_eq!(grid.cells.h, grid2.cells.h, "trough must be deterministic");
        for &h in &grid.cells.h {
            assert!(h <= 100, "trough h out of bounds: {h}");
        }
    }

    /// `Strait` blends cells toward sea level, so land cells near coast drop.
    #[test]
    fn strait_blends_toward_sea_level() {
        let mut grid = test_grid(2000, 42);
        let cells: Vec<u32> = (0..grid.mesh.points.len() as u32).collect();
        let op = EditOp {
            mode: EditMode::Strait,
            center_cell: 0,
            target_cell: 0,
            radius: 0.0,
            strength: 1.0, // full blend → all cells become sea level
            cells: cells.clone(),
        };
        edit_heightmap(&mut grid, &[op]);
        for &cid in &cells {
            assert_eq!(
                grid.cells.h[cid as usize], SEA_LEVEL,
                "strait full strength should set h to sea level"
            );
        }
    }

    /// `Mask` applies `(1 - nx²)(1 - ny²)` falloff: center cell gets full
    /// strength, edge cells get less. All results in [0, 100].
    #[test]
    fn mask_falloff_center_higher_than_edge() {
        let mut grid = test_grid(2000, 42);
        let center = 500u32;
        let radius = 500.0f32;
        let cells = gather_radius_cells(&grid.mesh, center, radius);
        // Set all cells to 100 so mask scales down (dist < 1).
        for &cid in &cells {
            grid.cells.h[cid as usize] = 100;
        }
        let op = EditOp {
            mode: EditMode::Mask,
            center_cell: center,
            target_cell: 0,
            radius,
            strength: 1.0,
            cells: cells.clone(),
        };
        edit_heightmap(&mut grid, &[op]);
        let center_h = grid.cells.h[center as usize];
        // Center (nx=0, ny=0) → dist=1.0 → masked = 100 * 1.0 = 100 → no change.
        assert_eq!(center_h, 100, "mask center should be unchanged (dist=1.0)");
        // At least one edge cell should be lower than center.
        let any_lower = cells
            .iter()
            .any(|&cid| cid != center && grid.cells.h[cid as usize] < center_h);
        assert!(any_lower, "mask should lower at least one edge cell");
        for &h in &grid.cells.h {
            assert!(h <= 100, "mask h out of bounds: {h}");
        }
    }

    /// `Invert` is deterministic: same grid + same op → byte-identical.
    #[test]
    fn invert_is_deterministic() {
        let mut a = test_grid(1000, 42);
        let mut b = test_grid(1000, 42);
        let cells: Vec<u32> = (0..a.mesh.points.len() as u32).collect();
        let op = EditOp {
            mode: EditMode::Invert,
            center_cell: 0,
            target_cell: 0,
            radius: 0.0,
            strength: 0.0,
            cells,
        };
        edit_heightmap(&mut a, std::slice::from_ref(&op));
        edit_heightmap(&mut b, &[op]);
        assert_eq!(a.cells.h, b.cells.h, "invert must be deterministic");
    }

    /// `Add` is deterministic: same grid + same op → byte-identical.
    #[test]
    fn add_is_deterministic() {
        let mut a = test_grid(1000, 42);
        let mut b = test_grid(1000, 42);
        let cells: Vec<u32> = (0..a.mesh.points.len() as u32).collect();
        let op = EditOp {
            mode: EditMode::Add,
            center_cell: 0,
            target_cell: 0,
            radius: 0.0,
            strength: 0.3,
            cells,
        };
        edit_heightmap(&mut a, std::slice::from_ref(&op));
        edit_heightmap(&mut b, &[op]);
        assert_eq!(a.cells.h, b.cells.h, "add must be deterministic");
    }

    /// `Multiply` is deterministic: same grid + same op → byte-identical.
    #[test]
    fn multiply_is_deterministic() {
        let mut a = test_grid(1000, 42);
        let mut b = test_grid(1000, 42);
        let cells: Vec<u32> = (0..a.mesh.points.len() as u32).collect();
        let op = EditOp {
            mode: EditMode::Multiply,
            center_cell: 0,
            target_cell: 0,
            radius: 0.0,
            strength: 1.5,
            cells,
        };
        edit_heightmap(&mut a, std::slice::from_ref(&op));
        edit_heightmap(&mut b, &[op]);
        assert_eq!(a.cells.h, b.cells.h, "multiply must be deterministic");
    }

    /// `Strait` is deterministic: same grid + same op → byte-identical.
    #[test]
    fn strait_is_deterministic() {
        let mut a = test_grid(2000, 42);
        let mut b = test_grid(2000, 42);
        let cells: Vec<u32> = (0..a.mesh.points.len() as u32).collect();
        let op = EditOp {
            mode: EditMode::Strait,
            center_cell: 0,
            target_cell: 0,
            radius: 0.0,
            strength: 0.5,
            cells,
        };
        edit_heightmap(&mut a, std::slice::from_ref(&op));
        edit_heightmap(&mut b, &[op]);
        assert_eq!(a.cells.h, b.cells.h, "strait must be deterministic");
    }

    /// `Mask` is deterministic: same grid + same op → byte-identical.
    #[test]
    fn mask_is_deterministic() {
        let mut a = test_grid(2000, 42);
        let mut b = test_grid(2000, 42);
        let center = 500u32;
        let radius = 500.0f32;
        let cells = gather_radius_cells(&a.mesh, center, radius);
        let op = EditOp {
            mode: EditMode::Mask,
            center_cell: center,
            target_cell: 0,
            radius,
            strength: 0.7,
            cells,
        };
        edit_heightmap(&mut a, std::slice::from_ref(&op));
        edit_heightmap(&mut b, &[op]);
        assert_eq!(a.cells.h, b.cells.h, "mask must be deterministic");
    }

    fn variance(h: &[u8]) -> f64 {
        if h.is_empty() {
            return 0.0;
        }
        let mean = h.iter().map(|&x| x as f64).sum::<f64>() / h.len() as f64;
        h.iter().map(|&x| (x as f64 - mean).powi(2)).sum::<f64>() / h.len() as f64
    }
}

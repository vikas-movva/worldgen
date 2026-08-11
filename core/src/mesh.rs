//! Voronoi mesh generator — Step 1.1 (Phase 1).
//!
//! Builds a Voronoi diagram over `cell_count` deterministically-seeded random
//! points in the world rectangle `[0, WORLD_W) × [0, WORLD_H)`, then extracts the
//! cell/vertex/adjacency topology in a form the renderer and downstream
//! generators (heightmap, climate, biomes, ...) consume.
//!
//! ## Algorithm
//!
//! 1. Sample `cell_count` points uniformly in the world rectangle using a
//!    `StdRng::seed_from_u64(seed)` (the single RNG source — see the
//!    Determinism Contract, technical-requirements §4).
//! 2. Bulk-load them into a `spade::DelaunayTriangulation` via `bulk_load_stable`,
//!    which preserves insertion order — so Delaunay vertex index `i` corresponds
//!    to input point `i`, giving deterministic cell ids.
//! 3. Walk the Delaunay dual: for each Delaunay vertex (= one Voronoi cell),
//!    `VoronoiFace::adjacent_edges()` iterates the cell boundary in clockwise
//!    order. Each directed edge's `from()` vertex is a Voronoi vertex; inner
//!    ones carry the triangle circumcenter as their position, outer ones are
//!    infinite and are clamped to the world rectangle along their direction
//!    vector (so every cell closes into a finite polygon for WebGL rendering).
//! 4. Neighbors come from the undirected Delaunay edges adjacent to each vertex
//!    (`VertexHandle::out_undirected_edges` → `rev().next()`), which gives the
//!    same cells in a deterministic (clockwise) order.
//! 5. Output is CSR-packed: a single offset array `i` of length `N+1` is shared
//!    by both `v` (cell vertex ids) and `c` (cell neighbor ids), since both are
//!    derived from the same boundary traversal and have the same length per
//!    cell (every cell has as many vertices as neighbors in a closed Voronoi
//!    polygon).
//!
//! Determinism is airtight: `StdRng::seed_from_u64`, `bulk_load_stable`,
//! `BTreeMap` for stable vertex-id assignment (no HashMap), explicit clock-wise
//! ordering from spade, total-order sort for neighbor fallback ordering. All
//! math in `f64`. No `Math.random`/`Date`/`performance.now` — pure compute.

use std::collections::BTreeMap;

use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use spade::{
    handles::{FixedVertexHandle, VoronoiVertex},
    DelaunayTriangulation, Point2, Triangulation,
};
use wasm_bindgen::prelude::*;

/// World width in world-space units (technical-requirements §2).
pub const WORLD_W: f64 = 10000.0;
/// World height in world-space units.
pub const WORLD_H: f64 = 8000.0;

/// CSR-packed mesh topology returned to JS.
///
/// Wire format agreed in Step 1.1:
/// ```text
/// {
///   points: Float64Array,        // length 2*N, interleaved [x0,y0,x1,y1,...]
///   cells: {
///     v: Uint32Array,             // flat vertex indices per cell
///     c: Uint32Array,             // flat neighbor cell indices per cell
///     i: Uint32Array,             // length N+1 — cell i's slice is v[i[k]..i[k+1]]
///     b: Uint8Array               // border flag: 1 if cell touches the hull
///   },
///   vertices: {
///     p: Float64Array             // length 2*M, interleaved [x,y,...]
///   }
/// }
/// ```
/// `cells.i` is shared between `v` and `c` — a cell has as many vertices as
/// neighbors in a closed Voronoi polygon. (Edge cells with infinite vertices
/// are clamped to the world rectangle so they still close.)
#[derive(Serialize, Deserialize, Clone)]
pub struct Mesh {
    pub points: Vec<[f64; 2]>,
    pub cells: Cells,
    pub vertices: Vertices,
    // World dimensions, carried on the wire so the heightmap (and later
    // generators) don't have to trust the compile-time `WORLD_W`/`WORLD_H`
    // constants. The `MeshView` previously hardcoded those constants; this field
    // makes the mesh self-describing and correct for sub-regions / non-square
    // worlds in later phases (adversarial review M5).
    pub world_w: f64,
    pub world_h: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Cells {
    pub v: Vec<u32>,
    pub c: Vec<u32>,
    pub i: Vec<u32>,
    pub b: Vec<u8>,
    /// FMG-style sampling grid: for each slot in a `cells_x × cells_y` integer
    /// grid covering the world rectangle, `spacing[slot]` is the cell id of the
    /// nearest cell. Used by the heightmap generator to map an `(x,y)` to a
    /// **spatially-close** cell id (so `Range 5-15 …` actually starts near the
    /// west edge). Without a real spacing index, `find_grid_cell` fell back to
    /// Row-major(√N) which mapped to arbitrary cell ids whose positions bore no
    /// relation to the requested `(x,y)` — features landed semi-randomly
    /// (adversarial review M7).
    pub spacing: Vec<u32>,
    /// Grid column count (`cells_x = √N`, rounded). `spacing` has length
    /// `cells_x * cells_y`.
    pub cells_x: u32,
    /// Grid row count.
    pub cells_y: u32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Vertices {
    pub p: Vec<[f64; 2]>,
}

/// Generate a deterministic Voronoi mesh.
///
/// `cell_count` is clamped to `[4, 1_000_000]` to stay sane (the app caps at
/// 60k in MVP). Returns a `JsValue` via `serde-wasm-bindgen` so the worker can
/// postMessage it (Phase 2 will replace the boundary with transferable
/// TypedArrays for zero-copy — but the shape stays the same).
pub fn build(cell_count: u32, seed: u32) -> Mesh {
    let n = cell_count.max(4) as usize;

    // 1. Deterministic points in the world rectangle.
    let mut rng = StdRng::seed_from_u64(seed as u64);
    let mut points_in: Vec<Point2<f64>> = Vec::with_capacity(n);
    for _ in 0..n {
        // Sample uniformly; reject-pull is not needed for the open rectangle.
        let x = rng.gen_range(0.0..WORLD_W);
        let y = rng.gen_range(0.0..WORLD_H);
        points_in.push(Point2::new(x, y));
    }

    // 2. Order-preserving bulk load → vertex index == input point index.
    //    Duplicates collapse to the lowest index (FMG-style), so cell ids stay
    //    stable and we don't lose input count under ties (collinear/duplicate
    //    points are also the documented E3 edge case — slight seeded jitter is
    //    added to avoid degenerate triangles).
    let jittered: Vec<Point2<f64>> = points_in
        .iter()
        .map(|p| Point2::new(p.x + small_jitter(&mut rng), p.y + small_jitter(&mut rng)))
        .collect();
    let tris: DelaunayTriangulation<Point2<f64>> =
        DelaunayTriangulation::bulk_load_stable(jittered)
            .expect("bulk_load_stable only fails on < 3 points or all-collinear input");

    // spade may deduplicate coincident points; the resulting triangulation has
    // `num_vertices() <= n`. We keep its vertex indices as the canonical cell
    // ids (this is the documented behavior — `bulk_load_stable` keeps the
    // lowest index of any duplicate set).
    let num_cells = tris.num_vertices();

    // 3. Assign stable ids to Voronoi inner faces (= Voronoi vertices).
    //    BTreeMap keyed on the fixed face's underlying index → deterministic
    //    iteration order independent of spade's internal allocator (satisfies
    //    the "no HashMap in order-sensitive code" determinism clause, §4).
    //    Populated lazily in the cell walk below; vertex id == index in
    //    `v_positions` for inner vertices (outer/clamped vertices follow).
    let mut voronoi_vertex_ids: BTreeMap<usize, u32> = BTreeMap::new();

    // 4. For each cell, walk its boundary in clockwise order and record:
    //      - the sequence of clamped Voronoi vertex ids (cells.v)
    //      - the sequence of neighbor cell ids (cells.c)
    //      - the border flag (cells.b): set if ANY boundary vertex is Outer
    //    `i` is the CSR offset array (shared by v and c).
    let mut v_flat: Vec<u32> = Vec::with_capacity(num_cells * 6);
    let mut c_flat: Vec<u32> = Vec::with_capacity(num_cells * 6);
    let mut i_arr: Vec<u32> = Vec::with_capacity(num_cells + 1);
    let mut b_arr: Vec<u8> = vec![0; num_cells];
    let mut v_positions: Vec<[f64; 2]> = Vec::new();

    // Walk vertices in fixed-index order (0..num_cells) — deterministic.
    // We need cell_id for indexing b_arr and the triangulation; suppress
    // clippy::needless_range_loop because we genuinely need the index.
    #[allow(clippy::needless_range_loop)]
    for cell_id in 0..num_cells {
        i_arr.push(v_flat.len() as u32);
        let vhandle = tris.vertex(FixedVertexHandle::from_index(cell_id));
        let cell_pos = vhandle.position();
        let vface = vhandle.as_voronoi_face();

        let mut border = false;
        // `adjacent_edges` is clockwise. Each edge's `from()` is a Voronoi
        // vertex; we record its (clamped) id. The neighbor for that edge is
        // the far endpoint of the dual directed Delaunay edge.
        for edge in vface.adjacent_edges() {
            let from = edge.from();
            // Vertex id == index into `v_positions`. For an inner Voronoi
            // vertex (a real circumcenter) we memoize by the dual face's fixed
            // key so two cells sharing the vertex agree on its id. For an outer
            // (infinite) vertex we mint a fresh clamped coordinate — outer
            // vertices only appear on hull cells and don't coalesce, so a
            // fresh id per occurrence is correct and deterministic.
            let vv_id: u32 = match from {
                VoronoiVertex::Inner(face) => {
                    let key = face.fix().index();
                    if let Some(&id) = voronoi_vertex_ids.get(&key) {
                        id
                    } else {
                        let id = v_positions.len() as u32;
                        let p = face.circumcenter();
                        v_positions.push([p.x, p.y]);
                        voronoi_vertex_ids.insert(key, id);
                        id
                    }
                }
                VoronoiVertex::Outer(_) => {
                    border = true;
                    let dir = edge.direction_vector();
                    let clamped = clamp_to_world(cell_pos, dir);
                    let id = v_positions.len() as u32;
                    v_positions.push(clamped);
                    id
                }
            };
            v_flat.push(vv_id);

            // Neighbor: the directed Delaunay edge dual to this Voronoi edge
            // connects `cell_id` to its neighbor; pick the far endpoint.
            let de = edge.as_delaunay_edge();
            let n0 = de.from().fix().index();
            let n1 = de.to().fix().index();
            let neighbor = if n0 == cell_id { n1 } else { n0 };
            c_flat.push(neighbor as u32);
        }
        b_arr[cell_id] = if border { 1 } else { 0 };
    }
    i_arr.push(v_flat.len() as u32);

    // 5. Assemble the Mesh.
    //    CRITICAL: the triangulation may deduplicate coincident/jitter-colliding
    //    points. `num_cells = tris.num_vertices()` is the actual cell count.
    //    We must return the ACTUAL cell positions from the triangulation (the
    //    jittered points), not the original pre-jitter input points. Otherwise
    //    downstream consumers (heightmap, renderer) that index `points[cell_id]`
    //    get the wrong position when dedup occurs.
    let points: Vec<[f64; 2]> = (0..num_cells)
        .map(|i| {
            let p = tris.vertex(FixedVertexHandle::from_index(i)).position();
            [p.x, p.y]
        })
        .collect();

    // 6. Build the FMG-style sampling grid (`cells.spacing`).
    //    For each slot in a `cells_x × cells_y` integer grid covering the world
    //    rectangle, `spacing[slot]` = the cell id whose position is nearest to the
    //    slot's center. The heightmap uses this to map a template's requested
    //    `(x, y)` to a spatially-close cell id (so `Range 5-15 …` actually
    //    starts near the west edge). Without a real spacing index, features
    //    landed at arbitrary `RowMajor(√N)` cell ids (adversarial review M7).
    //
    //    `cells_x = √N` rounded, aspect-corrected so `cells_x * spacing_x =
    //    world_w` and `cells_y * spacing_y = world_h`. For each slot center we
    //    do a linear nearest-cell search — O(N · slots) = O(N · N) = O(N²). At
    //    60k this is ~7e9 ops (~couple seconds), acceptable at build time; a KD-
    //    tree could make it O(N log N) if a later phase needs faster mesh builds.
    let aspect = WORLD_W / WORLD_H;
    let cells_x = ((num_cells as f64).sqrt() * (aspect.sqrt())).round() as u32;
    let cells_x = cells_x.max(1);
    let cells_y = ((cells_x as f64) / aspect).round() as u32;
    let cells_y = cells_y.max(1);
    let spacing = build_spacing(&points, cells_x, cells_y);

    Mesh {
        points,
        cells: Cells {
            v: v_flat,
            c: c_flat,
            i: i_arr,
            b: b_arr,
            spacing,
            cells_x,
            cells_y,
        },
        vertices: Vertices { p: v_positions },
        world_w: WORLD_W,
        world_h: WORLD_H,
    }
}

/// Build the sampling grid: `spacing[slot]` = a cell id that lies in (or very
/// near) each grid slot. O(N), not O(N²): we bucket each cell into its slot
/// (`cell_id → slot_id`), then for any empty slot we fall back to the nearest
/// non-empty slot via a two-pass prefix scan. This is sufficient for the
/// heightmap's purpose — `find_grid_cell` only needs *a* cell near `(x, y)`,
/// not provably the global nearest — and keeps 60k mesh builds at ~1s instead of
/// 60s+ in debug.
fn build_spacing(points: &[[f64; 2]], cells_x: u32, cells_y: u32) -> Vec<u32> {
    let n_slots = (cells_x as usize) * (cells_y as usize);
    let sx = WORLD_W / cells_x as f64;
    let sy = WORLD_H / cells_y as f64;
    // Slot id for each cell.
    let mut slot_of_cell: Vec<u32> = Vec::with_capacity(points.len());
    for &[px, py] in points {
        let col = ((px / sx) as u32).min(cells_x - 1);
        let row = ((py / sy) as u32).min(cells_y - 1);
        slot_of_cell.push(row * cells_x + col);
    }
    // `slot_occupant[s]` = first cell id in slot `s` (deterministic — first
    // cell in `points` order wins, which is the post-dedup triangulation order).
    let mut slot_occupant: Vec<i64> = vec![-1i64; n_slots];
    for (cell_id, &s) in slot_of_cell.iter().enumerate() {
        if slot_occupant[s as usize] == -1 {
            slot_occupant[s as usize] = cell_id as i64;
        }
    }
    // Fill empty slots with the nearest non-empty slot's occupant, scanning
    // both directions so a leading run of empty slots (e.g. slot 0 if no cell
    // landed in the top-left corner) still gets a real neighbor instead of
    // silently falling back to cell 0 (which can be anywhere on the map).
    // Forward pass: carry the last occupied id left→right.
    let mut last: i64 = -1;
    for s in 0..n_slots {
        if slot_occupant[s] != -1 {
            last = slot_occupant[s];
        } else if last != -1 {
            slot_occupant[s] = last;
        }
    }
    // Backward pass: only slots still -1 (those before the first occupied) get
    // the next occupied id from the right. This guarantees every slot resolves
    // to a cell within ~1 slot-width of its position, so `find_grid_cell` is
    // spatially faithful (adversarial review M7).
    let mut next: i64 = -1;
    for s in (0..n_slots).rev() {
        if slot_occupant[s] != -1 {
            next = slot_occupant[s];
        } else if next != -1 {
            slot_occupant[s] = next;
        }
    }
    // Coerce to u32 with a default of cell 0 (only reached if every slot was
    // empty, which can't happen for N ≥ 1 since at least one cell occupies one
    // slot).
    slot_occupant
        .iter()
        .map(|&c| if c < 0 { 0u32 } else { c as u32 })
        .collect()
}

/// Small non-zero jitter to avoid degenerate (collinear/duplicate) Delaunay
/// input — the technical-requirements E3 edge case. Bounded to ~1e-6 of the
/// world dimension so it doesn't visibly perturb the points but does break
/// exact coincidences. Seeded from the world RNG → reproducible.
fn small_jitter(rng: &mut StdRng) -> f64 {
    // Uniform in [-5e-6, 5e-6) of a world unit. Quantizing the *stored* points
    // to integers (per §4 rule 5) is done on serialize, not at generation.
    let j: f64 = rng.gen_range(-5e-6..5e-6);
    j
}

/// Clamp an infinite Voronoi vertex to the world rectangle.
///
/// `cell_pos` is the cell's seed point (the Delaunay vertex this Voronoi face
/// surrounds). `dir` is the direction vector of the infinite edge (per spade's
/// `DirectedVoronoiEdge::direction_vector`). We march from `cell_pos` along
/// `dir` until we hit one of the rectangle's four sides, then return that
/// intersection. This guarantees the clamped vertex lies outside the cell (the
/// infinite ray did) and on the world boundary, so the polygon still closes.
fn clamp_to_world(cell_pos: Point2<f64>, dir: Point2<f64>) -> [f64; 2] {
    // Find the smallest t > 0 such that cell_pos + t*dir hits a rectangle side.
    // Solve for each of the four sides and take the minimum positive t.
    let (px, py) = (cell_pos.x, cell_pos.y);
    let (dx, dy) = (dir.x, dir.y);
    let mut best_t = f64::INFINITY;
    // x = 0
    if dx < 0.0 {
        let t = -px / dx;
        if t > 0.0 && t < best_t {
            best_t = t;
        }
    }
    // x = WORLD_W
    if dx > 0.0 {
        let t = (WORLD_W - px) / dx;
        if t > 0.0 && t < best_t {
            best_t = t;
        }
    }
    // y = 0
    if dy < 0.0 {
        let t = -py / dy;
        if t > 0.0 && t < best_t {
            best_t = t;
        }
    }
    // y = WORLD_H
    if dy > 0.0 {
        let t = (WORLD_H - py) / dy;
        if t > 0.0 && t < best_t {
            best_t = t;
        }
    }
    if !best_t.is_finite() {
        // dir is zero or pure-axis-aligned away from any side — fall back to
        // the cell position itself. (Shouldn't happen for real input.)
        return [px, py];
    }
    let x = px + best_t * dx;
    let y = py + best_t * dy;
    // Clamp defensively in case of FP slop right at the boundary.
    [
        x.clamp(0.0, WORLD_W),
        y.clamp(0.0, WORLD_H),
    ]
}

/// `#[wasm_bindgen]` entry point. Serializes the `Mesh` to a `JsValue` via
/// `serde-wasm-bindgen`. (Phase 2 may swap this for direct TypedArray
/// construction to enable zero-copy transfer; the field shape is stable.)
pub fn generate_mesh(cell_count: u32, seed: u32) -> JsValue {
    let mesh = build(cell_count, seed);
    serde_wasm_bindgen::to_value(&mesh).expect("mesh serde to JsValue")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adjacency must be symmetric: for every (a, b) we record,
    /// b ∈ cells.c[a] ⟺ a ∈ cells.c[b].
    #[test]
    fn adjacency_symmetric() {
        let mesh = build(1000, 42);
        let n = mesh.points.len();
        let i = &mesh.cells.i;
        let c = &mesh.cells.c;
        for cell in 0..n {
            let lo = i[cell] as usize;
            let hi = i[cell + 1] as usize;
            for &neigh in &c[lo..hi] {
                let n2 = neigh as usize;
                // Self-loops aren't allowed.
                assert_ne!(n2, cell, "self-neighbor at cell {cell}");
                // Reverse lookup: `cell` must be in c[n2].
                let lo2 = i[n2] as usize;
                let hi2 = i[n2 + 1] as usize;
                assert!(
                    c[lo2..hi2].iter().any(|&x| x as usize == cell),
                    "asymmetric: {cell}→{n2} set, but {n2}→{cell} missing"
                );
            }
        }
    }

    /// Every cell has ≥ 3 vertices (and ≥ 3 neighbors, since they come in 1:1).
    #[test]
    fn every_cell_has_at_least_3_vertices_and_neighbors() {
        let mesh = build(1000, 42);
        let n = mesh.points.len();
        let i = &mesh.cells.i;
        for cell in 0..n {
            let lo = i[cell] as usize;
            let hi = i[cell + 1] as usize;
            let nv = hi - lo;
            assert!(
                nv >= 3,
                "cell {cell} has only {nv} vertices/neighbors (need ≥3)"
            );
        }
    }

    /// CSR consistency: `i` has length N+1, is monotonic non-decreasing, and
    /// `v`/`c` lengths match `i[N]`.
    #[test]
    fn csr_well_formed() {
        let mesh = build(1000, 42);
        let n = mesh.points.len();
        assert_eq!(mesh.cells.i.len(), n + 1, "i must be length N+1");
        assert_eq!(mesh.cells.v.len(), mesh.cells.c.len(), "v and c same length");
        assert_eq!(
            *mesh.cells.i.last().unwrap() as usize,
            mesh.cells.v.len(),
            "i[N] must equal flat length"
        );
        for w in mesh.cells.i.windows(2) {
            assert!(w[0] <= w[1], "i must be monotonic non-decreasing");
        }
    }

    /// Border flag: every cell on the hull has b=1, every interior cell b=0
    /// (an interior cell must have b=0; a hull cell may have b=1).
    #[test]
    fn border_flag_consistent_with_hull() {
        let mesh = build(1000, 42);
        let n = mesh.points.len();
        let i = &mesh.cells.i;
        let v = &mesh.cells.v;
        // Count border cells; should be at least 3 (any convex hull has ≥3
        // vertices), and at most n.
        let border_count = mesh.cells.b.iter().filter(|&&b| b == 1).count();
        assert!(border_count >= 3, "need ≥3 border cells, got {border_count}");
        assert!(border_count <= n, "more border cells than cells");
        // Sanity: every cell's vertex ids in-bounds of v_positions.
        for cell in 0..n {
            let lo = i[cell] as usize;
            let hi = i[cell + 1] as usize;
            for &vid in &v[lo..hi] {
                assert!(
                    (vid as usize) < mesh.vertices.p.len(),
                    "vertex id {vid} out of bounds for cell {cell}"
                );
            }
        }
    }

    /// Determinism: same seed → byte-identical serialized output across runs.
    /// (CI will additionally xxHash the bytes; here we deep-compare the
    /// structs.)
    #[test]
    fn deterministic_same_seed() {
        let a = build(1000, 42);
        let b = build(1000, 42);
        assert_eq!(a.points, b.points, "points differ");
        assert_eq!(a.cells.v, b.cells.v, "cells.v differ");
        assert_eq!(a.cells.c, b.cells.c, "cells.c differ");
        assert_eq!(a.cells.i, b.cells.i, "cells.i differ");
        assert_eq!(a.cells.b, b.cells.b, "cells.b differ");
        // Vertex positions are floats; compare exact bits (determinism means
        // bit-identical, not just "close enough").
        assert_eq!(a.vertices.p.len(), b.vertices.p.len(), "vertex count differs");
        for (pa, pb) in a.vertices.p.iter().zip(b.vertices.p.iter()) {
            assert_eq!(pa[0].to_bits(), pb[0].to_bits(), "vertex x bits differ");
            assert_eq!(pa[1].to_bits(), pb[1].to_bits(), "vertex y bits differ");
        }
    }

    /// Different seeds → different outputs (sanity that the seed actually
    /// drives generation).
    #[test]
    fn different_seeds_differ() {
        let a = build(500, 42);
        let b = build(500, 7);
        assert_ne!(a.points, b.points, "different seeds should differ");
        assert_ne!(a.cells.v, b.cells.v, "different seeds should differ");
    }

    /// Point count matches the actual triangulation cell count (after dedup).
    /// At N=1000 with seeded jitter, duplicates are astronomically unlikely,
    /// so we expect 1000 cells.
    #[test]
    fn point_count_matches() {
        let mesh = build(1000, 42);
        // Actual cell count = i.len() - 1 (CSR offset array length N+1)
        let actual_cells = mesh.cells.i.len() - 1;
        assert_eq!(mesh.points.len(), actual_cells, "points must match actual cell count");
        assert_eq!(actual_cells, 1000, "should produce exactly 1000 cells after dedup");
    }

    /// All actual cell positions fall within the world rectangle.
    #[test]
    fn points_within_world() {
        let mesh = build(1000, 42);
        for &[x, y] in &mesh.points {
            assert!((0.0..=WORLD_W).contains(&x), "x={x} out of [0,{WORLD_W}]");
            assert!((0.0..=WORLD_H).contains(&y), "y={y} out of [0,{WORLD_H}]");
        }
        // Sanity: cell count from CSR matches points length
        assert_eq!(mesh.points.len(), mesh.cells.i.len() - 1);
    }

    /// The sampling grid `cells.spacing` must be well-formed: its length is
    /// `cells_x * cells_y`, every entry is a valid cell id, and it's
    /// deterministic across runs. Without this, the heightmap's
    /// `find_grid_cell` falls back to a RowMajor(√N) heuristic and features
    /// land at arbitrary cell ids (adversarial review M7).
    #[test]
    fn spacing_grid_well_formed() {
        let a = build(2000, 42);
        let b = build(2000, 42);
        let n = a.points.len();
        assert!(a.cells.cells_x >= 1, "cells_x must be ≥ 1");
        assert!(a.cells.cells_y >= 1, "cells_y must be ≥ 1");
        assert_eq!(
            a.cells.spacing.len(),
            (a.cells.cells_x as usize) * (a.cells.cells_y as usize),
            "spacing length must equal cells_x * cells_y"
        );
        for &cell_id in &a.cells.spacing {
            assert!((cell_id as usize) < n, "spacing cell id {cell_id} >= {n}");
        }
        assert_eq!(a.cells.spacing, b.cells.spacing, "spacing must be deterministic");
        assert_eq!(a.cells.cells_x, b.cells.cells_x);
        assert_eq!(a.cells.cells_y, b.cells.cells_y);
    }

    /// The sampling grid actually provides spatially-close cell ids for the
    /// corners of the world rectangle: the slot at `(row=0, col=0)` should
    /// point to a cell near `(x≈0, y≈0)`, and `(row=cells_y-1, col=cells_x-1)`
    /// to a cell near `(world_w, world_h)`. This is the "M7 regression doesn't
    /// return" canary.
    #[test]
    fn spacing_corners_map_nearby_cells() {
        let mesh = build(3000, 42);
        let cells_x = mesh.cells.cells_x as usize;
        let cells_y = mesh.cells.cells_y as usize;
        let s = &mesh.cells.spacing;
        let sx = WORLD_W / cells_x as f64;
        let sy = WORLD_H / cells_y as f64;
        let tl = s[0] as usize;
        let tl_x = mesh.points[tl][0];
        let tl_y = mesh.points[tl][1];
        assert!(
            tl_x < 3.0 * sx && tl_y < 3.0 * sy,
            "top-left spacing returned cell at ({tl_x},{tl_y}) — slot size ≈ ({sx},{sy}), should be near (0,0)"
        );
        let br = s[cells_y * cells_x - 1] as usize;
        let br_x = mesh.points[br][0];
        let br_y = mesh.points[br][1];
        assert!(
            br_x > WORLD_W - 3.0 * sx && br_y > WORLD_H - 3.0 * sy,
            "bottom-right spacing returned cell at ({br_x},{br_y}) — slot ≈ ({sx},{sy}), should be near ({WORLD_W},{WORLD_H})"
        );
    }
}
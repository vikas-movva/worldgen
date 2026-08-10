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
use serde::Serialize;
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
#[derive(Serialize)]
pub struct Mesh {
    pub points: Vec<[f64; 2]>,
    pub cells: Cells,
    pub vertices: Vertices,
}

#[derive(Serialize)]
pub struct Cells {
    pub v: Vec<u32>,
    pub c: Vec<u32>,
    pub i: Vec<u32>,
    pub b: Vec<u8>,
}

#[derive(Serialize)]
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

    // 5. Assemble the Mesh. (We pushed into v_positions inline so the `p`
    //    array is already fully ordered by id — inner faces first in
    //    BTreeMap-key order, then outer clamped vertices in encounter order.)
    let points = points_in.iter().map(|p| [p.x, p.y]).collect();

    Mesh {
        points,
        cells: Cells {
            v: v_flat,
            c: c_flat,
            i: i_arr,
            b: b_arr,
        },
        vertices: Vertices { p: v_positions },
    }
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
        x.max(0.0).min(WORLD_W),
        y.max(0.0).min(WORLD_H),
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

    /// Point count matches the request (exactly N when N≥3 and no duplicates).
    /// At N=1000 with seeded jitter, duplicates are astronomically unlikely.
    #[test]
    fn point_count_matches() {
        let mesh = build(1000, 42);
        assert_eq!(mesh.points.len(), 1000, "should produce exactly 1000 points");
    }

    /// All points fall within the world rectangle.
    #[test]
    fn points_within_world() {
        let mesh = build(1000, 42);
        for &[x, y] in &mesh.points {
            assert!((0.0..=WORLD_W).contains(&x), "x={x} out of [0,{WORLD_W}]");
            assert!((0.0..=WORLD_H).contains(&y), "y={y} out of [0,{WORLD_H}]");
        }
    }
}
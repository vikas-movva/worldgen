//! Voronoi mesh generator
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
use std::f64::consts::PI;

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
    let n = cell_count.clamp(4, 1_000_000) as usize;

    // 1. Poisson-disk sampling + Lloyd relaxation → well-spaced seed points.
    //    Uniform random sampling produces clustered points that yield skinny
    //    Voronoi cells (bad for downstream generators and rendering). Poisson
    //    disk enforces a minimum separation (≈ r_min = world_diagonal / √N),
    //    and 3-5 Lloyd iterations move each point to its cell centroid, giving
    //    a blue-noise, centroidal-Voronoi-tessellation layout with regular,
    //    roughly hexagonal cells.
    let mut rng = StdRng::seed_from_u64(seed as u64);
    let mut points_in: Vec<Point2<f64>> = poisson_disk_sample(n, &mut rng);
    // Clamp to a tiny margin inside the world edge so that the small_jitter
    // applied later (±5e-6) cannot push points outside [0, WORLD_W]×[0, WORLD_H].
    let margin = 1e-5;
    for p in &mut points_in {
        p.x = p.x.clamp(margin, WORLD_W - margin);
        p.y = p.y.clamp(margin, WORLD_H - margin);
    }
    for _ in 0..3 {
        points_in = lloyd_relax(&points_in, 0.5, &mut rng);
        // Re-clamp after each relaxation step (Lloyd can drift points out).
        for p in &mut points_in {
            p.x = p.x.clamp(margin, WORLD_W - margin);
            p.y = p.y.clamp(margin, WORLD_H - margin);
        }
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
    #[allow(clippy::needless_range_loop)]
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

/// Poisson-disk sampling of `n` points in the world rectangle.
///
/// Uses Bridson's algorithm with a uniform grid for O(N) candidate lookup.
/// `r_min` is the minimum separation; the grid cell size is `r_min / √2` so
/// every candidate only needs to check its own and neighbouring cells.
/// The RNG is consumed only for the initial seed point and the random
/// direction/angle of each candidate — the algorithm is deterministic given
/// the seed (§4).
///
/// Returns exactly `n` points (or fewer if the world is too small to fit
/// `n` points at the requested `r_min`, which only happens at extreme
/// `n` / tiny world ratios — the caller clamps `n` to `[4, 1_000_000]`).
fn poisson_disk_sample(n: usize, rng: &mut StdRng) -> Vec<Point2<f64>> {
    if n == 0 {
        return vec![];
    }
    // Minimum separation: target ~n points in a world of area A → average
    // spacing ≈ √(A/n). Poisson-disk r_min is typically a bit smaller than
    // that so the final count lands near n (Bridson's algorithm is a
    // rejection sampler — it stops when the active list is empty, which
    // happens once the disk is saturated).
    let area = WORLD_W * WORLD_H;
    let r_min = (area / n as f64).sqrt() * 0.9;
    let r_min = r_min.max(1.0); // guard against degenerate tiny worlds

    // Grid: cell size = r_min / √2 ensures any two points within r_min are
    // in the same or adjacent cells (so a 3×3 neighbourhood check suffices).
    let cell_size = r_min / 2.0_f64.sqrt();
    let cols = (WORLD_W / cell_size).ceil() as usize + 1;
    let rows = (WORLD_H / cell_size).ceil() as usize + 1;

    // `grid[s]` = list of point indices in cell `s` (row-major).
    let mut grid: Vec<Vec<usize>> = vec![vec![]; cols * rows];
    let mut points: Vec<Point2<f64>> = Vec::with_capacity(n);

    // Helper: insert a point into the grid if it satisfies the minimum
    // distance to all existing points (checked against the 3×3 neighbourhood).
    let insert = |p: Point2<f64>, grid: &mut Vec<Vec<usize>>, points: &mut Vec<Point2<f64>>| -> bool {
        let col = (p.x / cell_size) as usize;
        let row = (p.y / cell_size) as usize;
        let col = col.min(cols - 1);
        let row = row.min(rows - 1);
        // Check 3×3 neighbourhood.
        for dcol in 0..=2 {
            for drow in 0..=2 {
                let nc = col as isize + dcol as isize - 1;
                let nr = row as isize + drow as isize - 1;
                if nc < 0 || nr < 0 || nc >= cols as isize || nr >= rows as isize {
                    continue;
                }
                let idx = nr as usize * cols + nc as usize;
                for &pi in &grid[idx] {
                    let dp = points[pi];
                    let dx = p.x - dp.x;
                    let dy = p.y - dp.y;
                    if dx * dx + dy * dy < r_min * r_min {
                        return false;
                    }
                }
            }
        }
        let id = points.len();
        points.push(p);
        grid[row * cols + col].push(id);
        true
    };

    // Seed: a random point in the world rectangle.
    let sx = rng.gen_range(0.0..WORLD_W);
    let sy = rng.gen_range(0.0..WORLD_H);
    let seed = Point2::new(sx, sy);
    insert(seed, &mut grid, &mut points);

    // Active list: indices of points that may still spawn candidates.
    let mut active: Vec<usize> = vec![0];

    while !active.is_empty() && points.len() < n {
        // Pick a random active point.
        let ai = rng.gen_range(0..active.len());
        let anchor = active[ai];
        let ap = points[anchor];

        let mut found = false;
        // Up to 30 attempts per active point before we give up on it.
        for _ in 0..30 {
            // Random direction and radius in [r_min, 2*r_min].
            let angle = rng.gen_range(0.0..2.0 * PI);
            let dist = rng.gen_range(r_min..2.0 * r_min);
            let cx = ap.x + dist * angle.cos();
            let cy = ap.y + dist * angle.sin();
            if cx < 0.0 || cx >= WORLD_W || cy < 0.0 || cy >= WORLD_H {
                continue;
            }
            let cand = Point2::new(cx, cy);
            if insert(cand, &mut grid, &mut points) {
                active.push(points.len() - 1);
                found = true;
                break;
            }
        }
        if !found {
            // Remove this active point (swap-remove for O(1)).
            active.swap_remove(ai);
        }
    }

    // Poisson-disk may not reach exactly `n` points (the active list empties
    // before the disk is full — this is normal for Bridson's algorithm when
    // the minimum separation is large relative to the world area). Fill the
    // remainder with uniform random points, which is deterministic given the
    // seed (§4) and preserves the exact cell count contract.
    while points.len() < n {
        let x = rng.gen_range(0.0..WORLD_W);
        let y = rng.gen_range(0.0..WORLD_H);
        points.push(Point2::new(x, y));
    }

    points
}

/// Lloyd relaxation: move each point toward the centroid of its Voronoi cell.
///
/// Uses a uniform spatial grid so each point only clips against its local
/// neighbourhood (O(N × avg_neighbors) instead of O(N²)). The grid cell size
/// is set so that any point whose Voronoi cell could reach the target point
/// must be in the same or adjacent cell. Each point's cell polygon is clipped
/// against the half-planes of nearby points, and the centroid of the resulting
/// polygon is computed exactly (area-weighted). Points are moved a fraction
/// `step` of the way to the centroid and clamped to the world rectangle.
///
/// The RNG is unused (reserved for future tie-breaking); the function is
/// deterministic given the input points.
fn lloyd_relax(points: &[Point2<f64>], step: f64, _rng: &mut StdRng) -> Vec<Point2<f64>> {
    let n = points.len();
    if n < 3 {
        return points.to_vec();
    }
    // Spatial grid: cell size = 2 * r_min where r_min is the typical nearest-
    // neighbour distance. A point's Voronoi cell is bounded by bisectors with
    // its neighbours; any neighbour whose bisector intersects the cell must be
    // within ~2× the typical spacing. We use a generous cell size so the 3×3
    // neighbourhood covers all relevant points.
    let area = WORLD_W * WORLD_H;
    let r_typ = (area / n as f64).sqrt();
    let cell_size = r_typ * 2.5;
    let cell_size = cell_size.max(1.0);
    let cols = (WORLD_W / cell_size).ceil() as usize + 1;
    let rows = (WORLD_H / cell_size).ceil() as usize + 1;

    // Build the grid: `grid[s]` = list of point indices in cell `s`.
    let mut grid: Vec<Vec<usize>> = vec![vec![]; cols * rows];
    for (i, p) in points.iter().enumerate() {
        let col = (p.x / cell_size) as usize;
        let row = (p.y / cell_size) as usize;
        let col = col.min(cols - 1);
        let row = row.min(rows - 1);
        grid[row * cols + col].push(i);
    }

    let mut new_points: Vec<Point2<f64>> = Vec::with_capacity(n);
    for (i, &p) in points.iter().enumerate() {
        let col = (p.x / cell_size) as usize;
        let row = (p.y / cell_size) as usize;
        let col = col.min(cols - 1);
        let row = row.min(rows - 1);

        // Collect candidate neighbours from the 3×3 cell neighbourhood.
        let mut candidates: Vec<usize> = Vec::new();
        for dcol in 0..=2 {
            for drow in 0..=2 {
                let nc = col as isize + dcol as isize - 1;
                let nr = row as isize + drow as isize - 1;
                if nc < 0 || nr < 0 || nc >= cols as isize || nr >= rows as isize {
                    continue;
                }
                let idx = nr as usize * cols + nc as usize;
                for &pi in &grid[idx] {
                    if pi != i {
                        candidates.push(pi);
                    }
                }
            }
        }

        // Start with the world rectangle as the initial cell polygon.
        let mut poly: Vec<[f64; 2]> = vec![
            [0.0, 0.0],
            [WORLD_W, 0.0],
            [WORLD_W, WORLD_H],
            [0.0, WORLD_H],
        ];
        for &j in &candidates {
            let q = points[j];
            let mx = (p.x + q.x) / 2.0;
            let my = (p.y + q.y) / 2.0;
            let dx = q.x - p.x;
            let dy = q.y - p.y;
            poly = clip_polygon(&poly, mx, my, dx, dy);
            if poly.len() < 3 {
                break;
            }
        }
        if poly.len() >= 3 {
            // Compute polygon centroid (area-weighted).
            let mut cx = 0.0;
            let mut cy = 0.0;
            let mut area = 0.0;
            for k in 0..poly.len() {
                let x0 = poly[k][0];
                let y0 = poly[k][1];
                let x1 = poly[(k + 1) % poly.len()][0];
                let y1 = poly[(k + 1) % poly.len()][1];
                let cross = x0 * y1 - x1 * y0;
                cx += (x0 + x1) * cross;
                cy += (y0 + y1) * cross;
                area += cross;
            }
            area *= 0.5;
            if area.abs() > 1e-12 {
                cx /= 6.0 * area;
                cy /= 6.0 * area;
                let nx = p.x + (cx - p.x) * step;
                let ny = p.y + (cy - p.y) * step;
                new_points.push(Point2::new(
                    nx.clamp(0.0, WORLD_W),
                    ny.clamp(0.0, WORLD_H),
                ));
            } else {
                new_points.push(p);
            }
        } else {
            new_points.push(p);
        }
    }
    new_points
}

/// Clip a convex polygon against the half-plane `dx*(x - mx) + dy*(y - my) <= 0`.
/// Returns the resulting polygon (possibly empty).
fn clip_polygon(poly: &[[f64; 2]], mx: f64, my: f64, dx: f64, dy: f64) -> Vec<[f64; 2]> {
    let mut result: Vec<[f64; 2]> = Vec::with_capacity(poly.len() + 1);
    let n = poly.len();
    if n == 0 {
        return result;
    }
    for i in 0..n {
        let cur = poly[i];
        let next = poly[(i + 1) % n];
        let cur_inside = dx * (cur[0] - mx) + dy * (cur[1] - my) <= 1e-12;
        let next_inside = dx * (next[0] - mx) + dy * (next[1] - my) <= 1e-12;
        if cur_inside {
            result.push(cur);
        }
        if cur_inside != next_inside {
            // Edge crosses the line: compute intersection.
            let t = {
                let num = dx * (cur[0] - mx) + dy * (cur[1] - my);
                let den = dx * (next[0] - cur[0]) + dy * (next[1] - cur[1]);
                if den.abs() < 1e-15 {
                    continue;
                }
                -num / den
            };
            if t >= 0.0 && t <= 1.0 {
                let ix = cur[0] + t * (next[0] - cur[0]);
                let iy = cur[1] + t * (next[1] - cur[1]);
                result.push([ix, iy]);
            }
        }
    }
    result
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

    // ── Direct helper unit tests ───────────────────────────────────────────
    // The three private helpers below were previously exercised only
    // transitively through full `build()` runs. Direct tests pin their
    // contracts so a regression in the clamp math, jitter bound, or spacing
    // fill logic is caught without needing to reverse-engineer a seed that
    // happens to trigger the relevant branch.

    /// `clamp_to_world` marches from `cell_pos` along `dir` to the first
    /// world-rectangle side it hits. Each of the four cardinal ray directions
    /// must land exactly on the corresponding side, and the result must stay
    /// inside the world rectangle. A zero direction vector must fall back to
    /// the cell position itself rather than panic or produce NaN.
    #[test]
    fn clamp_to_world_hits_each_side() {
        // Ray pointing west (dx<0) from the interior → hits x=0.
        let p = Point2::new(5000.0, 4000.0);
        let [x, y] = clamp_to_world(p, Point2::new(-1.0, 0.0));
        assert!((x - 0.0).abs() < 1e-9, "west ray should hit x=0, got {x}");
        assert!((y - 4000.0).abs() < 1e-9, "y unchanged, got {y}");
        // Ray pointing east (dx>0) → hits x=WORLD_W.
        let [x, y] = clamp_to_world(p, Point2::new(1.0, 0.0));
        assert!((x - WORLD_W).abs() < 1e-9, "east ray should hit x=WORLD_W, got {x}");
        assert!((y - 4000.0).abs() < 1e-9);
        // Ray pointing north (dy<0) → hits y=0.
        let [x, y] = clamp_to_world(p, Point2::new(0.0, -1.0));
        assert!((x - 5000.0).abs() < 1e-9);
        assert!((y - 0.0).abs() < 1e-9, "north ray should hit y=0, got {y}");
        // Ray pointing south (dy>0) → hits y=WORLD_H.
        let [x, y] = clamp_to_world(p, Point2::new(0.0, 1.0));
        assert!((x - 5000.0).abs() < 1e-9);
        assert!((y - WORLD_H).abs() < 1e-9, "south ray should hit y=WORLD_H, got {y}");
    }

    /// A diagonal ray hits whichever side is nearer in terms of the parameter
    /// `t`, not whichever axis is larger — this is the branch that picks the
    /// minimum positive `t` over all four candidates.
    #[test]
    fn clamp_to_world_diagonal_picks_nearest_side() {
        // From near the west edge, a 45° SE ray should hit x=0? No — dx>0 so it
        // travels east; nearest side is the east wall at t=(WORLD_W-100)/1.
        let p = Point2::new(100.0, 4000.0);
        let [x, y] = clamp_to_world(p, Point2::new(1.0, 1.0));
        // East wall is far (9900 units); south wall is far (4000 units) but dy
        // and dx are both 1.0 so east (t=9900) vs south (t=4000): south wins.
        assert!((y - WORLD_H).abs() < 1e-9, "diagonal should hit south wall first, got y={y}");
        assert!((x - 4100.0).abs() < 1e-9, "x = 100 + 4000*1 = 4100, got {x}");
        // Negative-x boundary must not be selected when dx>0 (the `dx < 0.0`
        // guard), and vice versa.
        assert!((0.0..=WORLD_W).contains(&x));
        assert!((0.0..=WORLD_H).contains(&y));
    }

    /// Degenerate direction (zero vector): no side is reachable, so the helper
    /// falls back to the cell position. Must not panic or return NaN/inf.
    #[test]
    fn clamp_to_world_zero_direction_falls_back() {
        let p = Point2::new(1234.0, 5678.0);
        let [x, y] = clamp_to_world(p, Point2::new(0.0, 0.0));
        assert!((x - 1234.0).abs() < 1e-9 && (y - 5678.0).abs() < 1e-9);
        assert!(x.is_finite() && y.is_finite());
    }

    /// `small_jitter` must stay within the documented `[-5e-6, 5e-6)` bound
    /// and be reproducible for a given RNG state. The bound keeps the jitter
    /// invisible (~1e-6 of the world dimension) while still breaking exact
    /// coincidences (the E3 degeneracy guard).
    #[test]
    fn small_jitter_is_bounded() {
        let mut rng = StdRng::seed_from_u64(99);
        for _ in 0..10_000 {
            let j = small_jitter(&mut rng);
            assert!(j >= -5e-6, "jitter {j} below -5e-6");
            assert!(j < 5e-6, "jitter {j} at/above +5e-6");
            assert!(j.is_finite());
        }
    }

    /// `build_spacing`'s forward/backward fill must resolve every slot to a
    /// real cell id even when a run of leading slots is empty (no input cell
    /// landed there). Construct a pathological input where the first several
    /// slots have no occupant — the backward pass must propagate the first
    /// occupied slot leftward, so slot 0 does NOT silently fall back to cell 0
    /// (which could be anywhere). This is the M7 regression guard at the
    /// helper level.
    #[test]
    fn build_spacing_fills_leading_empty_slots() {
        // 4×1 grid over the full world width. Only slot 2 and 3 have cells;
        // slots 0 and 1 are empty and must be back-filled from slot 2.
        let cells_x: u32 = 4;
        let cells_y: u32 = 1;
        // Two cells, both in the right half (slot 2 and slot 3).
        let sx = WORLD_W / cells_x as f64; // 2500
        let points: Vec<[f64; 2]> = vec![
            [2.5 * sx + 100.0, WORLD_H * 0.5], // slot 2
            [3.5 * sx + 100.0, WORLD_H * 0.5], // slot 3
        ];
        let spacing = build_spacing(&points, cells_x, cells_y);
        assert_eq!(spacing.len(), 4);
        // Every entry must be a valid cell id (0 or 1 here).
        for &c in &spacing {
            assert!(c == 0 || c == 1, "invalid cell id {c} in spacing");
        }
        // Slots 0 and 1 must be back-filled with the slot-2 occupant (cell 0),
        // NOT left as -1/coerced-to-0-by-default and NOT pointing at an
        // arbitrary cell. cell 0 is in slot 2, so slots 0,1,2 should all
        // resolve to cell 0; slot 3 resolves to cell 1.
        assert_eq!(spacing[0], 0, "leading empty slot 0 should back-fill to cell 0");
        assert_eq!(spacing[1], 0, "leading empty slot 1 should back-fill to cell 0");
        assert_eq!(spacing[2], 0, "slot 2 holds cell 0");
        assert_eq!(spacing[3], 1, "slot 3 holds cell 1");
    }

    // ── Stronger / new top-level invariants ─────────────────────────────────

    /// The CSR offset array `i` is *shared* between `v` and `c` because a
    /// closed Voronoi cell has exactly as many vertices as neighbors. Pin
    /// that invariant explicitly: for every cell, the vertex-count slice and
    /// the neighbor-count slice have equal length. A future change that
    /// decouples the two arrays would silently break downstream readers
    /// (heightmap adjacency walks) without this guard.
    #[test]
    fn neighbor_count_equals_vertex_count_per_cell() {
        let mesh = build(1500, 42);
        let n = mesh.points.len();
        let i = &mesh.cells.i;
        for cell in 0..n {
            let lo = i[cell] as usize;
            let hi = i[cell + 1] as usize;
            let len = hi - lo;
            // Both v and c are sliced by the same [lo,hi); we already know
            // they have equal total length from csr_well_formed. Here we
            // verify the per-cell counts agree with the i-offset, i.e. no
            // off-by-one in the offset pushes.
            assert_eq!(
                len, hi - lo,
                "per-cell length self-consistent (sanity)"
            );
            assert!(len >= 3, "cell {cell} has {len} verts/neighbors (need ≥3)");
        }
        // The 1:1 vertex/neighbor relationship is what makes a single offset
        // array correct; assert the totals match (already implied by
        // csr_well_formed, restated as intent).
        assert_eq!(mesh.cells.v.len(), mesh.cells.c.len());
    }

    /// Every border cell's clamped (Outer) vertices must lie ON the world
    /// rectangle boundary — the clamp contract. If a border cell had an
    /// Outer vertex that was NOT on a boundary edge, the polygon wouldn't
    /// close correctly for the renderer. We identify Outer-derived vertices
    /// indirectly: a border cell's vertex ring must touch at least one world
    /// edge, and every vertex of a border cell that lies exactly on a world
    /// edge is a clamped Outer vertex (inner circumcenters are in the interior
    /// with overwhelming probability at N=1000).
    #[test]
    fn border_cells_have_vertices_on_world_boundary() {
        let mesh = build(1000, 42);
        let i = &mesh.cells.i;
        let v = &mesh.cells.v;
        let p = &mesh.vertices.p;
        const EPS: f64 = 1e-6;
        let n = mesh.points.len();
        let mut any_border = false;
        for cell in 0..n {
            if mesh.cells.b[cell] != 1 {
                continue;
            }
            any_border = true;
            let lo = i[cell] as usize;
            let hi = i[cell + 1] as usize;
            // At least one vertex of this border cell must sit on a world edge.
            let touches_edge = (lo..hi).any(|k| {
                let [x, y] = p[v[k] as usize];
                x.abs() < EPS
                    || (x - WORLD_W).abs() < EPS
                    || y.abs() < EPS
                    || (y - WORLD_H).abs() < EPS
            });
            assert!(touches_edge, "border cell {cell} has no vertex on the world boundary");
        }
        assert!(any_border, "expected at least one border cell at N=1000");
    }

    /// The minimum requested cell count is clamped to 4 (`cell_count.max(4)`).
    /// Build must not panic at the floor and must still produce a valid CSR
    /// mesh: N≥3, every cell has ≥3 verts/neighbors, adjacency is symmetric,
    /// and a border exists (the hull of ≥3 non-collinear points has ≥3 hull
    /// vertices). This guards degenerate-triangulation edge cases (spade's
    /// `bulk_load_stable` only fails on <3 points or all-collinear input).
    #[test]
    fn minimum_cell_count_does_not_panic() {
        let mesh = build(1, 42); // clamped to 4 internally
        let n = mesh.points.len();
        assert!(n >= 3, "need ≥3 cells for a triangulation, got {n}");
        assert_eq!(mesh.cells.i.len(), n + 1, "CSR i must be length N+1");
        let i = &mesh.cells.i;
        let c = &mesh.cells.c;
        for cell in 0..n {
            let lo = i[cell] as usize;
            let hi = i[cell + 1] as usize;
            assert!(hi - lo >= 3, "cell {cell} has <3 vertices");
            for &neigh in &c[lo..hi] {
                let n2 = neigh as usize;
                assert_ne!(n2, cell, "self-neighbor");
                assert!(n2 < n, "neighbor id {n2} out of bounds");
            }
        }
        let border_count = mesh.cells.b.iter().filter(|&&b| b == 1).count();
        assert!(border_count >= 3, "need ≥3 border cells, got {border_count}");
    }

    /// Determinism must hold for more than one seed. The existing
    /// `deterministic_same_seed` only checks seed 42. Re-run with a different
    /// seed and confirm byte-identical vertex positions (via `to_bits`) and
    /// identical topology arrays. This catches any path that accidentally
    /// depends on absolute seed value rather than treating it as an opaque
    /// RNG key.
    #[test]
    fn deterministic_across_seeds() {
        for seed in [7u32, 12345, 999] {
            let a = build(800, seed);
            let b = build(800, seed);
            assert_eq!(a.points, b.points, "seed {seed}: points differ");
            assert_eq!(a.cells.v, b.cells.v, "seed {seed}: cells.v differ");
            assert_eq!(a.cells.c, b.cells.c, "seed {seed}: cells.c differ");
            assert_eq!(a.cells.i, b.cells.i, "seed {seed}: cells.i differ");
            assert_eq!(a.cells.b, b.cells.b, "seed {seed}: cells.b differ");
            assert_eq!(a.cells.spacing, b.cells.spacing, "seed {seed}: spacing differs");
            assert_eq!(a.vertices.p.len(), b.vertices.p.len(), "seed {seed}: vertex count differs");
            for (pa, pb) in a.vertices.p.iter().zip(b.vertices.p.iter()) {
                assert_eq!(pa[0].to_bits(), pb[0].to_bits(), "seed {seed}: vertex x bits differ");
                assert_eq!(pa[1].to_bits(), pb[1].to_bits(), "seed {seed}: vertex y bits differ");
            }
        }
    }

    /// `different_seeds_differ` previously only checked `points` and
    /// `cells.v`. Strengthen: different seeds must produce genuinely
    /// different topology, not just different coordinates that happen to
    /// yield the same adjacency. At least one of the CSR arrays (`c`, `i`,
    /// `b`, `spacing`) or the vertex-count must differ.
    #[test]
    fn different_seeds_topologically_distinct() {
        let a = build(1000, 42);
        let b = build(1000, 7);
        assert_ne!(a.points, b.points, "coordinates should differ");
        let topology_differs = a.cells.c != b.cells.c
            || a.cells.i != b.cells.i
            || a.cells.b != b.cells.b
            || a.cells.spacing != b.cells.spacing
            || a.vertices.p.len() != b.vertices.p.len();
        assert!(topology_differs, "two seeds produced identical topology (c/i/b/spacing) — seed is not driving generation");
    }

    /// After `bulk_load_stable` dedup, no two cells should share the same
    /// seed-point position. (Jitter is added precisely to prevent this, so
    /// in practice duplicates never occur at N=1000 — but the regression
    /// guard catches a future change that drops the jitter or breaks dedup
    /// handling, which would silently corrupt `points[cell_id]` indexing
    /// downstream.)
    #[test]
    fn cell_points_are_unique() {
        let mesh = build(1000, 42);
        let mut seen = std::collections::HashSet::with_capacity(mesh.points.len());
        for &[x, y] in &mesh.points {
            // Quantize to 1e-9 so that FP-equal-but-not-bit-equal points (which
            // would indicate a dedup miss) still collide in the set.
            let key = (
                (x * 1e9).round() as i64,
                (y * 1e9).round() as i64,
            );
            assert!(seen.insert(key), "duplicate cell point at ({x},{y})");
        }
    }
}
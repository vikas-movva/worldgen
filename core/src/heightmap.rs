//! Heightmap generator — Step 1.2 (Phase 1).
//!
//! Produces `cells.h` (a `Uint8Array` of length N, values `0..=100`, `< 20` ==
//! water) from a Voronoi `Mesh`. This is a faithful port of Azgaar's FMG
//! `heightmap-generator.ts` flood primitives, reworked to be deterministic and
//! allocation-light for up to 60k cells.
//!
//! ## Determinism contract (technical-requirements §4)
//!
//! The only randomness source is the single `StdRng::seed_from_u64(seed)`.
//! Crucially, **`rng` is a parameter threaded through every helper** so the
//! draw order is identical run-to-run. We never call `Math.random`, `Date`,
//! `performance.now`, `thread_rng`, or `HashMap` iteration in order-sensitive
//! code. All math is `f64`; results are quantized to `u8` on write.
//!
//! ## Algorithm
//!
//! FMG picks a *template* (a script of tool steps) and runs it as a sequence.
//! We implement a small, self-contained default sequence of seeded `Hill` /
//! `Pit` / `Range` / `Trough` / `Smooth` / `Mask` floods over `cells.c`. The
//! `blobPower` / `linePower` decay tables are keyed by cell count exactly as in
//! FMG, so output visually matches. `generate_heightmap` runs the default
//! sequence and returns the height array. `Mesh` is taken by **value** from the
//! JS boundary (deserialized via serde-wasm-bindgen → `mesh::Mesh`); the
//! topology is read-only here.

use js_sys::Uint8Array;
use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::Deserialize;

use crate::mesh::{Cells, Mesh};

/// Sea level in the height scale. `< 20` is water (technical-requirements §3.1).
pub const SEA_LEVEL: u8 = 20;

/// Owned mesh needed by the heightmap (we only read `cells`/`points`/`world_w`).
/// `mesh::Mesh` is the serde target; here we wrap the bits we need so this
/// module doesn't depend on the WASM `Serialize` derive surface.
#[derive(Deserialize)]
pub struct MeshView {
    pub points: Vec<[f64; 2]>,
    pub cells: Cells,
    #[serde(rename = "world_w")]
    pub world_w: f64,
    #[serde(rename = "world_h")]
    pub world_h: f64,
}

impl MeshView {
    /// Build a `MeshView` from a serde-deserialized `mesh::Mesh`. We compute
    /// `world_w`/`world_h` from the point bounds so the JS side doesn't have to
    /// pass them (the wire `Mesh` has no width/height fields).
    pub fn from_mesh(mesh: &Mesh) -> MeshView {
        MeshView {
            points: mesh.points.clone(),
            cells: mesh.cells.clone(),
            world_w: mesh.world_w,
            world_h: mesh.world_h,
        }
    }
}

// ---------------------------------------------------------------------------
// FMG helpers, ported
// ---------------------------------------------------------------------------

/// Clamp a float into `[0, 100]` then round to nearest `u8` — FMG `lim`.
fn lim(v: f64) -> u8 {
    let c = v.clamp(0.0, 100.0);
    c.round() as u8
}

/// `P(probability)` — true with the given probability (FMG `P`).
fn p(rng: &mut StdRng, probability: f64) -> bool {
    if probability >= 1.0 {
        true
    } else if probability <= 0.0 {
        false
    } else {
        rng.gen_bool(probability)
    }
}

/// Parse an FMG range string (`"2"`, `"0.5"`, `"80-100"`, `"-2"`) into a number.
/// Mirrors `getNumberInRange`: a bare number with a fractional part returns the
/// integer part plus 1 with probability equal to the fraction; a `a-b` range
/// returns an integer uniformly in `[a, b]` (inclusive).
fn get_number_in_range(rng: &mut StdRng, s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    // Pure integer/float literal (possibly negative, e.g. "-2").
    if let Ok(n) = s.trim().parse::<f64>() {
        if n.fract() == 0.0 {
            return n;
        }
        // fractional literal: integer part + 1 w.p. frac (FMG: ~~r + +P(+r-~~r))
        let int_part = n.floor();
        let frac = n - int_part;
        return if p(rng, frac) { int_part + 1.0 } else { int_part };
    }
    let (sign, body): (f64, &str) = if let Some(stripped) = s.strip_prefix('-') {
        (-1.0, stripped)
    } else {
        (1.0, s)
    };
    let parts: Vec<&str> = body.split('-').collect();
    if parts.len() < 2 {
        return 0.0;
    }
    // Only the first element carries the (already-extracted) sign; the rest are
    // positive magnitudes. A range like `"-2-5"` is parsed as sign=-1, lo_raw=2,
    // hi_raw=5 → lo=-2, hi=-5 → empty range, so we treat `hi < lo` as lo..=lo.
    let Ok(lo_raw) = parts[0].trim().parse::<f64>() else {
        return 0.0;
    };
    let Ok(hi_raw) = parts.last().unwrap().trim().parse::<f64>() else {
        return 0.0;
    };
    // The sign applies only to the first bound; the upper bound is positive.
    // (FMG never emits negative ranges, but we must not silently produce
    // garbage — previously both bounds were signed, inverting the range.)
    let lo = if sign < 0.0 { -lo_raw } else { lo_raw };
    let hi = hi_raw;
    if hi < lo {
        // Sign made the range empty (e.g. "-2-5" → lo=-2, hi=+5 is fine, but
        // "-5--2" would collapse to lo=-5, hi=+2 which is valid). If still
        // empty after sign handling, clamp to lo..=lo so we never return 0.
        if hi_raw < lo_raw {
            return lo.round();
        }
    }
    // FMG `rand(lo, hi)` returns an integer in [lo, hi] inclusive.
    rng.gen_range((lo as i64)..=(hi as i64)) as f64
}

/// Map an `(x, y)` in world space to a cell id via the real `cells.spacing`
/// sampling grid built in `mesh.rs` (adversarial review M7). Previously this
/// was a `RowMajor(√N)` derivation that mapped to arbitrary cell ids whose
/// positions had no relation to the requested `(x, y)` — features landed
/// semi-randomly even when the template said `Range 5-15 …` (start near the
/// west edge). Now we resolve the slot id `(row, col)` in the real grid and
/// return `spacing[row * cells_x + col]`, the cell id of the nearest actual
/// cell, preserving the template's intent.
fn find_grid_cell(view: &MeshView, x: f64, y: f64) -> usize {
    let cells_x = view.cells.cells_x as usize;
    let cells_y = view.cells.cells_y as usize;
    if cells_x == 0 || cells_y == 0 || view.cells.spacing.is_empty() {
        // Fallback (should not happen post-M7): Row-major(√N) so we don't panic.
        let n = view.points.len();
        let side = (n as f64).sqrt();
        let sx = view.world_w / side;
        let sy = view.world_h / side;
        let col = ((x / sx).min(side - 1.0)).floor().max(0.0) as usize;
        let row = ((y / sy).min(side - 1.0)).floor().max(0.0) as usize;
        return row * (side as usize) + col;
    }
    let sx = view.world_w / cells_x as f64;
    let sy = view.world_h / cells_y as f64;
    let col = ((x / sx) as usize).min(cells_x - 1);
    let row = ((y / sy) as usize).min(cells_y - 1);
    let slot = row * cells_x + col;
    view.cells.spacing.get(slot).copied().unwrap_or(0) as usize
}

/// Step 2.5.4: pick the nearest cell to world-space `(x, y)`. Uses the
/// `cells.spacing` spatial grid (`find_grid_cell`) to get a bucket cell,
/// then refines by checking the bucket cell + its neighbors for the truly
/// nearest one (Euclidean distance to cell center). O(1)-ish, deterministic.
///
/// Returns `None` only if the mesh has no cells (edge case).
pub fn pick_cell(mesh: &crate::mesh::Mesh, x: f64, y: f64) -> Option<u32> {
    let n = mesh.points.len();
    if n == 0 {
        return None;
    }
    let view = MeshView::from_mesh(mesh);
    let bucket = find_grid_cell(&view, x, y);

    // Check the bucket cell + its neighbors for the nearest one.
    let mut best = bucket;
    let [bx, by] = mesh.points[bucket];
    let mut best_d2 = (bx - x).powi(2) + (by - y).powi(2);

    let csr_i = &mesh.cells.i;
    let csr_c = &mesh.cells.c;
    if bucket < csr_i.len() - 1 {
        let lo = csr_i[bucket] as usize;
        let hi = csr_i[bucket + 1] as usize;
        for &nb in &csr_c[lo..hi] {
            let nb = nb as usize;
            let [px, py] = mesh.points[nb];
            let d2 = (px - x).powi(2) + (py - y).powi(2);
            if d2 < best_d2 {
                best_d2 = d2;
                best = nb;
            }
        }
    }

    Some(best as u32)
}

/// Pick a point at fractional range `[minFrac, maxFrac]` of the world axis.
/// FMG `getPointInRange(range, length)` parses `range` as two ints /100 and
/// returns `rand(minFrac*length, maxFrac*length)`.
fn point_in_range(rng: &mut StdRng, range: &str, length: f64) -> Option<f64> {
    if !range.is_ascii() {
        return None;
    }
    let (a, b) = range.split_once('-')?;
    let Ok(min_pct) = a.trim().parse::<f64>() else {
        return None;
    };
    let max_pct = b.trim().parse::<f64>().unwrap_or(min_pct);
    let min = (min_pct / 100.0) * length;
    let max = (max_pct / 100.0) * length;
    if max <= min {
        return Some(min);
    }
    Some(rng.gen_range(min..max))
}

/// FMG `blobPower` table, keyed by cell count.
fn get_blob_power(cells: usize) -> f64 {
    let table: &[(usize, f64)] = &[
        (1000, 0.93),
        (2000, 0.95),
        (5000, 0.97),
        (10000, 0.98),
        (20000, 0.99),
        (30000, 0.991),
        (40000, 0.993),
        (50000, 0.994),
        (60000, 0.995),
        (70000, 0.9955),
        (80000, 0.996),
        (90000, 0.9964),
        (100000, 0.9973),
    ];
    for (n, v) in table {
        if cells <= *n {
            return *v;
        }
    }
    0.98
}

/// FMG `linePower` table, keyed by cell count.
pub fn get_line_power(cells: usize) -> f64 {
    let table: &[(usize, f64)] = &[
        (1000, 0.75),
        (2000, 0.77),
        (5000, 0.79),
        (10000, 0.81),
        (20000, 0.82),
        (30000, 0.83),
        (40000, 0.84),
        (50000, 0.86),
        (60000, 0.87),
        (70000, 0.88),
        (80000, 0.91),
        (90000, 0.92),
        (100000, 0.93),
    ];
    for (n, v) in table {
        if cells <= *n {
            return *v;
        }
    }
    0.81
}

// ---------------------------------------------------------------------------
// Flood primitives (ported from FMG heightmap-generator.ts)
// ---------------------------------------------------------------------------

/// `addHill`: seed a cell at height `h`, flood-fill neighbors with
/// `change[c] = change[q] ** blobPower * (0.9..1.1)`, spreading while change > 1.
/// `max_cells` bounds the flood extent so coverage is N-independent (a continent
/// stays continent-sized whether the map has 1k or 60k cells).
///
/// NOTE: `change` is tracked as `f64` internally (FMG keeps it as a float) and
/// only the *final* height is quantized to `u8`. Using an integer accumulator
/// would round `change` below 1.0 to 0 and stall the cascade after one ring,
/// which is why the spread must be float-based. Determinism is preserved: all
/// arithmetic is fixed `f64` with seeded jitter.
fn add_hill(view: &MeshView, h: &mut [u8], rng: &mut StdRng, start: usize, blob_power: f64, max_cells: usize) {
    let n = h.len();
    let mut change = vec![0f64; n];
    let height = lim(get_number_in_range(rng, "85-100"));
    // Avoid starting on an already-too-high cell: walk to a neighbor that is
    // low enough. Pick the lowest-height neighbor (not just the first CSR slot,
    // which is an arbitrary, fixed order) so the escape actually descends toward
    // a seedable cell instead of looping on one high chain (adversarial review
    // L10). The neighbor scan is bounded and deterministic — no RNG draw, so it
    // doesn't perturb the seed draw order.
    let mut limit = 0;
    let mut s = start;
    while (h[s] as f64 + height as f64) > 90.0 && limit < 50 {
        let lo = view.cells.i[s] as usize;
        let hi = view.cells.i[s + 1] as usize;
        // Find the lowest-height neighbor; if none, stay put.
        let mut best_neighbor = s;
        let mut best_h = h[s] as f64;
        for &c in &view.cells.c[lo..hi] {
            let c = c as usize;
            if (h[c] as f64) < best_h {
                best_h = h[c] as f64;
                best_neighbor = c;
            }
        }
        if best_neighbor == s {
            break; // local min already, can't descend further
        }
        s = best_neighbor;
        limit += 1;
    }
    change[s] = height as f64;
    let mut painted = 0usize;
    let mut queue = vec![s];
    while let Some(q) = queue.pop() {
        let lo = view.cells.i[q] as usize;
        let hi = view.cells.i[q + 1] as usize;
        for &c in &view.cells.c[lo..hi] {
            let c = c as usize;
            if change[c] != 0.0 {
                continue;
            }
            let decay = change[q];
            let jitter = rng.gen_range(0.0..0.2) + 0.9;
            let nv = decay.powf(blob_power) * jitter;
            change[c] = nv;
            if nv > 1.0 {
                queue.push(c);
                painted += 1;
                if painted >= max_cells {
                    break;
                }
            }
        }
        if painted >= max_cells {
            break;
        }
    }
    for i in 0..n {
        if change[i] != 0.0 {
            h[i] = lim(h[i] as f64 + change[i]);
        }
    }
}

/// `addPit`: carve a depression into the heightmap. Mirror image of `add_hill`:
/// seed a cell with a *negative* `change` of magnitude `height`, then BFS over
/// `cells.c` propagating `change[c] = change[q] ** blobPower * jitter` while
/// `|change| > 1.0`. Only at the end is `change` quantized to `u8` and applied
/// as `h[i] = lim(h[i] + change[i])` (note `change[i] < 0`).
///
/// The previous implementation dropped the per-cell `change` map and quantized
/// the running decay to `u8` every ring — reintroducing the integer-accumulator
/// trap warned about in `heightmap-generation.md §2` on the decay variable, and
/// losing the propagated-decay spatial gradient (every cell in a ring got the
/// same magnitude). See adversarial review C2.
fn add_pit(view: &MeshView, h: &mut [u8], rng: &mut StdRng, start: usize, blob_power: f64, max_cells: usize) {
    let n = h.len();
    // Don't start a pit underwater — climb to a land neighbor. Mirrors add_hill's
    // "avoid starting on an already-too-high cell" but inverted.
    let mut limit = 0;
    let mut s = start;
    while h[s] < SEA_LEVEL && limit < 50 {
        let neighbors = &view.cells.c
            [view.cells.i[s] as usize..view.cells.i[s + 1] as usize];
        if let Some(&next) = neighbors.first() {
            s = next as usize;
        } else {
            break;
        }
        limit += 1;
    }
    let height = lim(get_number_in_range(rng, "10-20")) as f64;
    // Per-cell change map (negative). f64 throughout, quantize to u8 ONCE at the
    // end — same contract as add_hill's `change: Vec<f64>`.
    let mut change = vec![0f64; n];
    change[s] = -height;
    let mut painted = 0usize;
    let mut queue = vec![s];
    while let Some(q) = queue.pop() {
        let lo = view.cells.i[q] as usize;
        let hi = view.cells.i[q + 1] as usize;
        for &c in &view.cells.c[lo..hi] {
            let c = c as usize;
            if change[c] != 0.0 {
                continue;
            }
            let decay = change[q];
            let jitter = rng.gen_range(0.0..0.2) + 0.9;
            // Decay the *magnitude* and preserve the sign: `change[q] < 0` for a
            // pit, and `powf` of a negative base with a non-integer exponent is
            // `NaN` in IEEE-754. The pit should keep deepening toward the start
            // cell (magnitude grows inward), so decay `decay.abs().powf(...)` and
            // re-apply the sign.
            let sign = decay.signum();
            let nv = sign * decay.abs().powf(blob_power) * jitter;
            change[c] = nv;
            if nv.abs() > 1.0 {
                queue.push(c);
                painted += 1;
                if painted >= max_cells {
                    break;
                }
            }
        }
        if painted >= max_cells {
            break;
        }
    }
    // Apply once, quantize once. `change[i] <= 0.0` everywhere it's nonzero.
    for i in 0..n {
        if change[i] != 0.0 {
            h[i] = lim(h[i] as f64 + change[i]);
        }
    }
}

/// Build the main ridge path from `start` to `end` walking the neighbor that
/// minimizes squared distance to `end`, with `randomness` chance to halve the
/// distance (FMG `getRange`). `used` prevents revisits.
pub fn build_range(view: &MeshView, rng: &mut StdRng, start: usize, end: usize, randomness: f64) -> Vec<usize> {
    let mut used = vec![false; view.points.len()];
    let mut range = vec![start];
    used[start] = true;
    let mut cur = start;
    let p = &view.points;
    while cur != end {
        let lo = view.cells.i[cur] as usize;
        let hi = view.cells.i[cur + 1] as usize;
        let mut min = f64::INFINITY;
        let mut next = cur;
        for &e in &view.cells.c[lo..hi] {
            let e = e as usize;
            if used[e] {
                continue;
            }
            let ex = p[e][0] - p[end][0];
            let ey = p[e][1] - p[end][1];
            let mut diff = ex * ex + ey * ey;
            if rng.gen_bool(randomness) {
                diff /= 2.0;
            }
            if diff < min {
                min = diff;
                next = e;
            }
        }
        if min == f64::INFINITY {
            break;
        }
        cur = next;
        range.push(cur);
        used[cur] = true;
    }
    range
}

/// `addRange` / `addTrough`: grow a ridge/trench along a path, spreading outward
/// with `linePower` decay, then carve prominences downhill every 6th cell.
/// `max_cells` caps the outward spread extent for N-independent sizing.
#[allow(clippy::too_many_arguments)]
fn add_ridge(
    view: &MeshView,
    h: &mut [u8],
    rng: &mut StdRng,
    start: usize,
    end: usize,
    randomness: f64,
    line_power: f64,
    raise: bool,
    max_cells: usize,
) {
    let n = h.len();
    let mut used = vec![false; n];
    let mut height = lim(get_number_in_range(rng, if raise { "30-60" } else { "20-30" }));
    let range = build_range(view, rng, start, end, randomness);
    let mut queue: Vec<usize> = range.clone();
    for q in &queue {
        used[*q] = true;
    }
    let mut painted = range.len();
    while !queue.is_empty() {
        let frontier = queue.clone();
        queue = Vec::new();
        for &i in &frontier {
            let add = height as f64 * (rng.gen_range(0.0..0.3) + 0.85);
            h[i] = lim(h[i] as f64 + if raise { add } else { -add });
            painted += 1;
            if painted >= max_cells {
                break;
            }
        }
        if painted >= max_cells {
            break;
        }
        // FMG: h = h ** linePower - 1; break if h < 2.
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
    // Prominences: every 6th ridge cell, descend to its lowest neighbor and
    // blend. FMG drives the descent depth off the ridge *path length*
    // (`range.length`), not the count of outward-spread rings we happened to
    // paint. The old code used `ring` (incremented before the decay break check,
    // so off-by-one) — using `range.len()` matches FMG and is independent of how
    // far the BFS spread (adversarial review L11).
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
            // leastIndex = neighbor with minimum height.
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

/// `smooth`: replace each cell with a weighted mean of itself + neighbors
/// (`fr` is the smoothing radius; `add` is a constant offset).
fn smooth(view: &MeshView, h: &mut [u8], fr: f64, add: f64) {
    let n = h.len();
    let snapshot = h.to_vec();
    for i in 0..n {
        let lo = view.cells.i[i] as usize;
        let hi = view.cells.i[i + 1] as usize;
        let mut sum = snapshot[i] as f64;
        let mut count = 1usize;
        for &c in &view.cells.c[lo..hi] {
            sum += snapshot[c as usize] as f64;
            count += 1;
        }
        let mean = sum / count as f64;
        let new_v = if (fr - 1.0).abs() < f64::EPSILON {
            mean + add
        } else {
            (snapshot[i] as f64 * (fr - 1.0) + mean + add) / fr
        };
        h[i] = lim(new_v);
    }
}

/// FMG `mask`: radial falloff `(1-nx²)(1-ny²)`; `power < 0` inverts (center→edge).
fn mask(view: &MeshView, h: &mut [u8], power: f64) {
    // `power == 0` is a no-op mask. The blend formula below collapses to a hard
    // radial falloff (`snapshot * distance`) at `fr == 1.0`, which silently
    // flattens edge cells to 0 — not the identity. Special-case it. The default
    // template always passes `Mask 2`, so this branch was unreachable until we
    // tested it (adversarial review C3).
    if power == 0.0 {
        return;
    }
    let n = h.len();
    // The blend weight `(fr - 1) / fr` must stay in [0, 1) so the masked value
    // is always a convex combination of the original `snapshot` and its radial
    // fall-off. `|power| < 1` made `fr - 1` negative → the blended value could
    // exceed `snapshot` (a "boost" instead of a mask). Clamp the floor at 1.0.
    let fr = power.abs().max(1.0);
    let snapshot = h.to_vec();
    for i in 0..n {
        let [x, y] = view.points[i];
        let nx = (2.0 * x) / view.world_w - 1.0; // [-1,1], 0 = center
        let ny = (2.0 * y) / view.world_h - 1.0;
        let mut distance = (1.0 - nx * nx) * (1.0 - ny * ny); // 1 center, 0 edge
        if power < 0.0 {
            distance = 1.0 - distance;
        }
        let masked = snapshot[i] as f64 * distance;
        // Blend: `original * (fr-1)/fr + masked * 1/fr` — a convex combination
        // because both weights are non-negative and sum to 1 once fr >= 1.
        h[i] = lim((snapshot[i] as f64 * (fr - 1.0) + masked) / fr);
    }
}

/// FMG `modify(range="land", mult)`: scale land cells around sea level.
/// `mult` is the multiplier (a2); cells below `SEA_LEVEL` are unaffected.
fn multiply_land(h: &mut [u8], mult: f64) {
    for v in h.iter_mut() {
        if *v >= SEA_LEVEL {
            let scaled = ((*v as f64 - SEA_LEVEL as f64) * mult + SEA_LEVEL as f64).round();
            *v = lim(scaled);
        }
    }
}

// ---------------------------------------------------------------------------
// Template runner
// ---------------------------------------------------------------------------

/// A single parsed template step. (We only implement the tools the default
/// sequence uses — Hill/Pit/Range/Trough/Smooth/Mask — kept minimal and
/// deterministic.)
#[derive(Debug)]
enum Tool {
    Hill,
    Pit,
    Range,
    Trough,
    Smooth,
    Mask,
    Multiply,
}

struct Step {
    tool: Tool,
    a2: String, // count (or power for Mask/Smooth, multiplier for Multiply)
    a4: String, // rangeX
    a5: String, // rangeY
}

/// Parse the default template as a set of steps. The default procedural
/// sequence (mirrors a blend of FMG's `continents`/`oldWorld` templates, tuned
/// to land ~25-55% land). Each line: `Tool count height rangeX rangeY`.
fn default_template() -> &'static str {
    // Balanced default template. We start from an all-water map and raise
    // continents via hills/ranges, then carve water back with pits/troughs,
    // smooth, and apply a gentle radial mask. Feature counts are size-scaled
    // in `run_step` so the land fraction stays in a sane band across 1k–60k
    // cells. Each line: `Tool count height rangeX rangeY`.
    "Hill 6-10 60-85 10-90 10-90\n\
     Range 2-4 30-60 5-15 25-75\n\
     Range 2-4 30-60 80-95 25-75\n\
     Range 1-3 30-55 80-90 20-80\n\
     Trough 2-4 15-25 15-85 20-80\n\
     Pit 2-4 10-20 15-85 20-80\n\
     Smooth 3 0 0 0\n\
     Mask 2 0 0 0"
}

fn parse_template(text: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let tool = match parts[0] {
            "Hill" => Tool::Hill,
            "Pit" => Tool::Pit,
            "Range" => Tool::Range,
            "Trough" => Tool::Trough,
            "Smooth" => Tool::Smooth,
            "Mask" => Tool::Mask,
            "Multiply" => Tool::Multiply,
            _ => continue,
        };
        steps.push(Step {
            tool,
            a2: parts.get(1).unwrap_or(&"").to_string(),
            a4: parts.get(3).unwrap_or(&"").to_string(),
            a5: parts.get(4).unwrap_or(&"").to_string(),
        });
    }
    steps
}

/// Run one parsed step against `h`, drawing randomness from `rng`.
fn run_step(view: &MeshView, h: &mut [u8], rng: &mut StdRng, step: &Step, blob_power: f64, line_power: f64) {
    // FMG templates use absolute feature counts tuned for a large map. To keep
    // the land fraction in a sane band across cell counts (1k–60k) we scale the
    // number of features with map size and cap each flood's *extent* to a
    // continent-sized budget that scales with N. This keeps a continent
    // continent-sized whether the map has 1k or 60k cells (FMG's unbounded
    // floods instead flood nearly the entire map at high N). Deliberate,
    // documented deviation that keeps the determinism + land-fraction gates
    // passing at every supported resolution.
    //
    // The per-feature floors are N-RELATIVE (N/50, not an absolute constant):
    // an absolute `max(300)` floor over-floods small maps — at N=1000 a 300-cell
    // hill budget is 30% of the whole map and ~14% of seeds exceeded 70% land
    // (adversarial review, C1). `N/50` keeps the floor proportional: 20 cells
    // at N=1k (2%, still enough to break coincidences) and 1200 at N=60k.
    //
    // `count_scale` is also tuned for an even land-fraction band across N: a flat
    // floor of 1.0 below N=20k left N=10k with the same feature count as N=1k,
    // but with 10× the cells the floods didn't coalesce — land fraction sagged to
    // 0.118–0.196. Scaling `count_scale = 1 + log₁₀(N/1000)` raises feature
    // counts smoothly and mildly with N (×1 at 1k, ×2 at 10k, ×2.78 at 60k,
    // capped at 3) so the union of floods covers a comparable fraction at every
    // resolution. A √N scaling was too aggressive and pushed N≥30k to 0.75+.
    let n = view.points.len();
    let count_scale = (1.0 + (n as f64 / 1000.0).log10()).clamp(1.0, 3.0);
    let scaled_count = |base: f64| ((base * count_scale).round() as i64).max(1) as usize;
    // One feature covers ~`extent_frac` of the map; expressed as a cell budget.
    let min_feature = (n / 50).max(1);
    let hill_budget = ((n as f64 * 0.10).round() as usize).max(min_feature);
    // The pit budget was 2.5% of N pre-fix and that was fine *only because the
    // old `add_pit` stalled after ~one ring of carving. The corrected pit
    // (per-cell `change` cascade, quantize-once) actually carves the full budget,
    // so 2.5% × ~4 pits = ~10% of land was being eaten at mid-N, dropping N=10000
    // to 0.118 for some seeds (adversarial review C1 follow-up). 1% × 4 pits =
    // ~4% carved — a local basin, not a regional one.
    let pit_budget = ((n as f64 * 0.01).round() as usize).max(min_feature);
    let ridge_budget = ((n as f64 * 0.07).round() as usize).max(min_feature);
    match step.tool {
        Tool::Hill => {
            let count = scaled_count(get_number_in_range(rng, &step.a2).max(1.0));
            for _ in 0..count {
                let Some(x) = point_in_range(rng, &step.a4, view.world_w) else {
                    continue;
                };
                let Some(y) = point_in_range(rng, &step.a5, view.world_h) else {
                    continue;
                };
                let start = find_grid_cell(view, x, y);
                add_hill(view, h, rng, start, blob_power, hill_budget);
            }
        }
        Tool::Pit => {
            let count = scaled_count(get_number_in_range(rng, &step.a2).max(1.0));
            for _ in 0..count {
                let Some(x) = point_in_range(rng, &step.a4, view.world_w) else {
                    continue;
                };
                let Some(y) = point_in_range(rng, &step.a5, view.world_h) else {
                    continue;
                };
                let start = find_grid_cell(view, x, y);
                add_pit(view, h, rng, start, blob_power, pit_budget);
            }
        }
        Tool::Range => {
            let count = scaled_count(get_number_in_range(rng, &step.a2).max(1.0));
            for _ in 0..count {
                let (start, end) = pick_range_endpoints(view, rng, &step.a4, &step.a5);
                if start == end {
                    continue;
                }
                add_ridge(view, h, rng, start, end, 0.15, line_power, true, ridge_budget);
            }
        }
        Tool::Trough => {
            let count = scaled_count(get_number_in_range(rng, &step.a2).max(1.0));
            for _ in 0..count {
                let (start, end) = pick_range_endpoints(view, rng, &step.a4, &step.a5);
                if start == end {
                    continue;
                }
                add_ridge(view, h, rng, start, end, 0.2, line_power, false, ridge_budget);
            }
        }
        Tool::Smooth => {
            let fr = get_number_in_range(rng, &step.a2).max(1.0);
            smooth(view, h, fr, 0.0);
        }
        Tool::Mask => {
            let power = get_number_in_range(rng, &step.a2);
            mask(view, h, power);
        }
        Tool::Multiply => {
            let mult = get_number_in_range(rng, &step.a2);
            multiply_land(h, mult);
        }
    }
}

/// FMG `addRange`/`addTrough`: pick a random start point in rangeX/rangeY, then
/// a random end point ~`[W/8, W/3]` (range) / `[W/8, W/2]` (trough) away.
fn pick_range_endpoints(view: &MeshView, rng: &mut StdRng, range_x: &str, range_y: &str) -> (usize, usize) {
    let w = view.world_w;
    let h = view.world_h;
    let Some(start_x) = point_in_range(rng, range_x, w) else {
        return (0, 0);
    };
    let Some(start_y) = point_in_range(rng, range_y, h) else {
        return (0, 0);
    };
    let start = find_grid_cell(view, start_x, start_y);
    // End: 10-90% width, 15-85% height (FMG: endX in [0.1W,0.9W], endY in [0.15H,0.85H]).
    let end_x = rng.gen_range(0.1..0.9) * w;
    let end_y = rng.gen_range(0.15..0.85) * h;
    let end = find_grid_cell(view, end_x, end_y);
    (start, end)
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Generate the heightmap for a deserialized `Mesh`. Returns a `Vec<u8>` of
/// length N, values `0..=100`, `< 20` == water.
pub fn generate(mesh: &Mesh, seed: u64) -> Vec<u8> {
    let view = MeshView::from_mesh(mesh);
    let n = view.points.len();
    let mut rng = StdRng::seed_from_u64(seed);
    let mut h = vec![0u8; n];

    let blob_power = get_blob_power(n);
    let line_power = get_line_power(n);

    // Start everything below sea level, then raise land.
    let steps = parse_template(default_template());
    for step in &steps {
        run_step(&view, &mut h, &mut rng, step, blob_power, line_power);
    }

    h
}

/// `#[wasm_bindgen]` entry point: takes a `Mesh` (deserialized from the JS
/// boundary via `serde-wasm-bindgen`) and returns a `Uint8Array` `cells.h`.
/// Exposed as `generate_heightmap(mesh, seed)`.
pub fn generate_heightmap(mesh: Mesh, seed: u32) -> Uint8Array {
    let h = generate(&mesh, seed as u64);
    let arr = Uint8Array::new_with_length(h.len() as u32);
    // Copy into the JS typed array. (Vec<u8> → Uint8Array via set.)
    // We write element-wise to avoid allocation mismatch on wasm32.
    for (i, &v) in h.iter().enumerate() {
        arr.set_index(i as u32, v);
    }
    arr
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh;

    /// Build a small mesh deterministically for tests.
    fn test_mesh(cell_count: u32, seed: u32) -> Mesh {
        mesh::build(cell_count, seed)
    }

    /// All heights must be within `[0, 100]`.
    #[test]
    fn heights_in_range() {
        let mesh = test_mesh(2000, 42);
        let h = generate(&mesh, 42);
        for &v in &h {
            assert!((0..=100).contains(&v), "height out of range: {v}");
        }
    }

    /// Land fraction for the default template must stay in the published sanity
    /// band [0.20, 0.70] across the **full** supported cell-count range and
    /// many seeds. The prior 4-seed / N=3000-only sweep missed the small-N
    /// flooding bug (adversarial review C1): at N=1000 ~14% of seeds exceeded
    /// 0.70 because the per-feature budget floors were absolute (`max(300)`),
    /// so a single hill could cover 30% of the map. The floors are now
    /// N-relative (`max(N/50)`) and this sweep enforces the band at every
    /// supported resolution. Both bounds are inclusive — a `0.200` boundary hit
    /// is acceptable land coverage, not a regression; an exclusive upper bound
    /// would reject a 0.700 candidate that is otherwise exactly on-spec.
    #[test]
    #[ignore = "slow: 50 seeds x 4 sizes — run with `cargo test -- --ignored` after major step completion"]
    fn land_fraction_sane_across_sizes_and_seeds() {
        for n in [1000u32, 3000, 10000, 30000] {
            for seed in 1u32..=50 {
                let mesh = test_mesh(n, seed);
                let h = generate(&mesh, seed as u64);
                let land = h.iter().filter(|&&v| v >= SEA_LEVEL).count();
                let frac = land as f64 / h.len() as f64;
                assert!(
                    (0.20..=0.70).contains(&frac),
                    "N={n} seed={seed}: land fraction {:.3} out of [0.20, 0.70]",
                    frac
                );
            }
        }
    }

    /// Determinism: identical seed + mesh → byte-identical heights.
    #[test]
    fn deterministic_same_seed() {
        let mesh = test_mesh(2000, 42);
        let a = generate(&mesh, 42);
        let b = generate(&mesh, 42);
        assert_eq!(a, b, "heightmap not deterministic for same seed");
    }

    /// Different seeds → different heights (sanity that seed drives output).
    #[test]
    fn different_seeds_differ() {
        let mesh = test_mesh(2000, 42);
        let a = generate(&mesh, 1);
        let b = generate(&mesh, 2);
        assert_ne!(a, b, "different seeds produced identical heightmaps");
    }

    /// 60k smoke test: must complete fast and stay in range with sane land frac.
    #[test]
    fn sixty_k_smoke() {
        let t0 = std::time::Instant::now();
        let mesh = test_mesh(60000, 7);
        let build_ms = t0.elapsed().as_millis();
        let g0 = std::time::Instant::now();
        let h = generate(&mesh, 7);
        let gen_ms = g0.elapsed().as_millis();
        for &v in &h {
            assert!((0..=100).contains(&v));
        }
        let land = h.iter().filter(|&&v| v >= SEA_LEVEL).count();
        let frac = land as f64 / h.len() as f64;
        assert!(
            (0.20..0.70).contains(&frac),
            "60k land fraction {:.2} out of [0.20,0.70]",
            frac
        );
        // Mesh build for 60k is excluded from the gate; heightmap gen itself is
        // cheap (floods). Print for visibility.
        eprintln!("60k: mesh_build={build_ms}ms heightmap_gen={gen_ms}ms");
    }

    /// `smooth` reduces local variance (max height after smoothing ≤ max before).
    #[test]
    fn smooth_reduces_variance() {
        let mesh = test_mesh(1000, 42);
        let view = MeshView::from_mesh(&mesh);
        let mut rng = StdRng::seed_from_u64(42);
        let mut h = vec![0u8; view.points.len()];
        let blob_power = get_blob_power(h.len());
        // Seed a few spikes.
        for _ in 0..5 {
            let start = (rng.gen_range(0..h.len())) as usize;
            add_hill(&view, &mut h, &mut rng, start, blob_power, 200);
        }
        let before_max = *h.iter().max().unwrap();
        let before_var = variance(&h);
        smooth(&view, &mut h, 3.0, 0.0);
        let after_max = *h.iter().max().unwrap();
        let after_var = variance(&h);
        assert!(after_max <= before_max);
        assert!(after_var <= before_var, "smoothing should not increase variance");
    }

    fn variance(h: &[u8]) -> f64 {
        let mean = h.iter().map(|&x| x as f64).sum::<f64>() / h.len() as f64;
        h.iter().map(|&x| (x as f64 - mean).powi(2)).sum::<f64>() / h.len() as f64
    }

    /// `mask` pulls edge cells toward 0 relative to center (center stays high).
    #[test]
    fn mask_falloff_edges_lower() {
        let mesh = test_mesh(1000, 42);
        let view = MeshView::from_mesh(&mesh);
        let mut rng = StdRng::seed_from_u64(42);
        let _ = &mut rng;
        let mut h = vec![50u8; view.points.len()];
        // Raise a center cell high so the mask has a clear gradient.
        let center = find_grid_cell(&view, view.world_w / 2.0, view.world_h / 2.0);
        h[center] = 100;
        mask(&view, &mut h, 4.0);
        // An edge cell should be lower than the center after masking.
        let edge = find_grid_cell(&view, 2.0, 2.0);
        assert!(h[edge] <= h[center], "edge {} should be <= center {}", h[edge], h[center]);
    }

    /// `mask` with `power == 0` is the identity (no-op). Previously the formula
    /// collapsed to `snapshot*distance` — a hard radial falloff with no blend,
    /// silently flattening edges to 0 (adversarial review C3). The default
    /// template passes `Mask 2` so this branch was never exercised.
    #[test]
    fn mask_power_zero_is_identity() {
        let mesh = test_mesh(1000, 42);
        let view = MeshView::from_mesh(&mesh);
        let mut h = vec![50u8; view.points.len()];
        let before = h.clone();
        mask(&view, &mut h, 0.0);
        assert_eq!(h, before, "mask(0) must be a no-op, not a hard falloff");
    }

    /// `add_pit` actually lowers the local mean in its footprint. The previous
    /// implementation had no per-cell `change` cascade — every ring used the
    /// same scalar magnitude and the running decay was quantized to `u8` every
    /// ring, so the propagated-decay gradient was lost (adversarial review C2).
    /// After a pit, the seed cell and its neighbors must be strictly lower than
    /// the surrounding, untouched cells.
    #[test]
    fn pit_lowers_local_mean() {
        let mesh = test_mesh(2000, 42);
        let view = MeshView::from_mesh(&mesh);
        let blob_power = get_blob_power(view.points.len());
        let mut rng = StdRng::seed_from_u64(42);
        // Flat high plain so the pit has a clear before state.
        let mut h = vec![80u8; view.points.len()];
        let start = 500;
        let budget = (view.points.len() / 10).max(20);
        add_pit(&view, &mut h, &mut rng, start, blob_power, budget);
        // The start cell should drop noticeably from the 80 baseline.
        assert!(
            h[start] < 80,
            "pit should lower the start cell ({} < 80)",
            h[start]
        );
        // Mean of the pit's neighbors should be lower than the untouched mean.
        let lo = view.cells.i[start] as usize;
        let hi = view.cells.i[start + 1] as usize;
        let neighbors: Vec<u8> = view.cells.c[lo..hi].iter().map(|&c| h[c as usize]).collect();
        let neighbor_mean = neighbors.iter().map(|&v| v as f64).sum::<f64>() / neighbors.len() as f64;
        assert!(
            neighbor_mean < 80.0,
            "pit should lower neighbor mean ({} < 80), spread cascade was lost in the old impl",
            neighbor_mean
        );
        // The pit's center must be at least as low as its rim (monotone decay).
        for &nh in &neighbors {
            assert!(
                h[start] <= nh + 5,
                "pit center {} should be <= rim+5 ({}), decay propagated",
                h[start],
                nh
            );
        }
    }

    // ── Direct helper unit tests ───────────────────────────────────────────
    // The private helpers below were previously exercised only transitively
    // through full `generate()` runs. Direct tests pin their contracts so a
    // regression in the clamp/round math, range parser, power tables, or
    // spatial index is caught without needing to reverse-engineer a seed that
    // happens to trigger the relevant branch.

    /// `lim(v)` clamps to `[0,100]` and rounds to nearest integer (ties to
    /// even per IEEE-754). FMG `lim` is exactly this. Edge cases: negative
    /// inputs floor to 0; >100 ceil to 100; 0.5 rounds to 1; 100.5 rounds
    /// to 100 (clamp wins before round). These are the quantization
    /// boundaries every flood primitive relies on.
    #[test]
    fn lim_clamp_and_round() {
        assert_eq!(lim(-10.0), 0);
        assert_eq!(lim(-0.1), 0);
        assert_eq!(lim(0.0), 0);
        assert_eq!(lim(0.4), 0);
        assert_eq!(lim(0.5), 1); // rounds to nearest
        assert_eq!(lim(0.6), 1);
        assert_eq!(lim(49.5), 50);
        assert_eq!(lim(100.0), 100);
        assert_eq!(lim(100.4), 100);
        assert_eq!(lim(100.5), 100); // clamp before round: 100.5 → clamp → 100
        assert_eq!(lim(150.0), 100);
    }

    /// `get_number_in_range` parses FMG range strings. Key contracts:
    /// - Empty string → 0.
    /// - Pure integer literal (e.g. `"42"`) → exactly that integer.
    /// - Pure float literal (e.g. `"3.7"`) → integer part + 1 w.p. frac (3.7 → 3 or 4, 0.7 prob of 4).
    /// - Negative literal (e.g. `"-2"`) → negative integer.
    /// - Range `"a-b"` (e.g. `"90-100"`) → uniform integer in `[a, b]` inclusive.
    /// - Negative range `"-2-5"` → sign applies ONLY to first bound (lo=-2, hi=5), not both (adversarial review L8).
    /// - Invalid/unparsable → 0 (defensive).
    /// - Determinism: same RNG state + same input → same output (tested via paired RNGs).
    #[test]
    fn get_number_in_range_parses_all_forms() {
        let mut rng1 = StdRng::seed_from_u64(1);
        let mut rng2 = StdRng::seed_from_u64(1);

        // Empty string
        assert_eq!(get_number_in_range(&mut rng1, ""), 0.0);

        // Pure integer
        assert_eq!(get_number_in_range(&mut rng1, "42"), 42.0);

        // Pure float with fraction → probabilistic (test determinism by pairing RNGs)
        // We can't assert exact value, but paired RNGs must agree.
        let a = get_number_in_range(&mut rng1, "3.7");
        let b = get_number_in_range(&mut rng2, "3.7");
        assert_eq!(a, b, "fractional literal must be deterministic");
        assert!((3.0..=4.0).contains(&a), "3.7 must yield 3 or 4");

        // Negative literal
        assert_eq!(get_number_in_range(&mut rng1, "-5"), -5.0);

        // Standard range "a-b"
        let mut rng3 = StdRng::seed_from_u64(2);
        let mut rng4 = StdRng::seed_from_u64(2);
        for _ in 0..50 {
            let x = get_number_in_range(&mut rng3, "90-100");
            let y = get_number_in_range(&mut rng4, "90-100");
            assert_eq!(x, y, "range must be deterministic");
            assert!((90.0..=100.0).contains(&x), "90-100 must yield [90,100]");
        }

        // Negative range "-2-5" → sign only on first bound (lo=-2, hi=5)
        // This is the L8 fix: previously sign was applied to both bounds.
        let mut rng5 = StdRng::seed_from_u64(3);
        let mut rng6 = StdRng::seed_from_u64(3);
        for _ in 0..50 {
            let x = get_number_in_range(&mut rng5, "-2-5");
            let y = get_number_in_range(&mut rng6, "-2-5");
            assert_eq!(x, y, "negative range must be deterministic");
            assert!((-2.0..=5.0).contains(&x), "-2-5 must yield [-2,5], got {x}");
        }

        // Malformed → 0 (defensive)
        assert_eq!(get_number_in_range(&mut rng1, "not-a-number"), 0.0);
        assert_eq!(get_number_in_range(&mut rng1, "10-"), 0.0);
    }

    /// `point_in_range("min-max", length)` maps a percentage range to a
    /// world-space coordinate. Returns `None` on non-ASCII or unparsable.
    /// Degenerate `max <= min` returns `Some(min)`. Exact match (`50-50`)
    /// returns that exact coordinate. Note: `"10-"` parses as min=10, max=10
    /// (the empty second part defaults to min), so it returns Some(min) not
    /// None. Deterministic for same RNG state.
    #[test]
    fn point_in_range_maps_percentages() {
        let mut rng1 = StdRng::seed_from_u64(4);
        let mut rng2 = StdRng::seed_from_u64(4);
        let len = 1000.0;

        // Standard range
        let a = point_in_range(&mut rng1, "10-90", len);
        let b = point_in_range(&mut rng2, "10-90", len);
        assert!(a.is_some() && b.is_some());
        assert_eq!(a, b, "deterministic");
        let v = a.unwrap();
        assert!((100.0..=900.0).contains(&v), "10-90% of 1000 → [100,900], got {v}");

        // Degenerate max <= min → Some(min)
        assert_eq!(point_in_range(&mut rng1, "50-50", len), Some(500.0));
        assert_eq!(point_in_range(&mut rng1, "80-20", len), Some(800.0));
        // "10-" → second part empty → defaults to min_pct (10) → max=min → Some(min)
        assert_eq!(point_in_range(&mut rng1, "10-", len), Some(100.0));

        // Non-ASCII / unparsable → None
        assert!(point_in_range(&mut rng1, "not-a-range", len).is_none());
    }

    /// `get_blob_power` / `get_line_power` tables are monotonic-ish (larger
    /// N → higher power, i.e. slower decay) and have a fallback for N beyond
    /// the table (0.98 / 0.81). The fallback ensures no panic at arbitrary
    /// future cell counts. These tables are copied verbatim from FMG.
    #[test]
    fn blob_and_line_power_tables() {
        // Sample points from the table
        assert_eq!(get_blob_power(1000), 0.93);
        assert_eq!(get_blob_power(2000), 0.95);
        assert_eq!(get_blob_power(60000), 0.995);
        // Fallback beyond table max (100000)
        assert_eq!(get_blob_power(200000), 0.98);
        assert_eq!(get_line_power(1000), 0.75);
        assert_eq!(get_line_power(2000), 0.77);
        assert_eq!(get_line_power(60000), 0.87);
        assert_eq!(get_line_power(200000), 0.81);
        // Monotonic-ish: larger N generally → larger power (not strictly
        // monotonic at every step, but the trend is upward).
        assert!(get_blob_power(30000) >= get_blob_power(5000));
        assert!(get_line_power(30000) >= get_line_power(5000));
    }

    /// `find_grid_cell` must map world-space `(x,y)` to a *spatially-close*
    /// cell id via the real `cells.spacing` grid (M7 fix). Queries near the
    /// four corners must return cells near those corners, not arbitrary cells
    /// from a RowMajor(√N) heuristic. OOB coordinates clamp to the nearest
    /// grid slot rather than panic. (Uses same 3× slot threshold as the mesh
    /// test `spacing_corners_map_nearby_cells`.)
    #[test]
    fn find_grid_cell_is_spatially_faithful() {
        let mesh = mesh::build(3000, 42);
        let view = MeshView::from_mesh(&mesh);
        let n = view.points.len();
        let sx = view.world_w / view.cells.cells_x as f64;
        let sy = view.world_h / view.cells.cells_y as f64;
        const THRESH: f64 = 3.0; // matches mesh test threshold

        // Top-left corner (slot 0,0)
        let tl = find_grid_cell(&view, 0.0, 0.0);
        assert!(tl < n, "tl cell id {tl} out of bounds");
        let [tl_x, tl_y] = view.points[tl];
        assert!(tl_x < THRESH * sx && tl_y < THRESH * sy, "tl cell at ({tl_x},{tl_y}) should be near (0,0)");

        // Bottom-right corner (last slot)
        let br = find_grid_cell(&view, view.world_w - 1.0, view.world_h - 1.0);
        assert!(br < n);
        let [br_x, br_y] = view.points[br];
        assert!(br_x > view.world_w - THRESH * sx && br_y > view.world_h - THRESH * sy, "br cell at ({br_x},{br_y}) should be near ({},{})", view.world_w, view.world_h);

        // Center
        let center = find_grid_cell(&view, view.world_w / 2.0, view.world_h / 2.0);
        let [cx, cy] = view.points[center];
        assert!((cx - view.world_w / 2.0).abs() < THRESH * sx && (cy - view.world_h / 2.0).abs() < THRESH * sy);

        // OOB negative coordinates clamp to slot 0 (top-left)
        let oob_neg = find_grid_cell(&view, -100.0, -100.0);
        assert!(oob_neg < n);

        // OOB beyond world clamp to last slot
        let oob_pos = find_grid_cell(&view, view.world_w + 100.0, view.world_h + 100.0);
        assert!(oob_pos < n);
    }

    /// Step 2.5.4: `pick_cell` returns the nearest cell to `(x, y)`, refining
    /// the `find_grid_cell` bucket result by checking the bucket + its
    /// neighbors. The returned cell's center must be at least as close to the
    /// query point as the bucket cell's center.
    #[test]
    fn pick_cell_returns_nearest() {
        let mesh = mesh::build(3000, 42);
        // Query the exact center of cell 100.
        let [cx, cy] = mesh.points[100];
        let picked = pick_cell(&mesh, cx, cy).expect("pick_cell should return Some");
        assert_eq!(picked, 100, "querying cell 100's center should return cell 100");
    }

    /// Step 2.5.4: `pick_cell` picks a cell closer to the query point than
    /// the raw bucket lookup when the query is on a Voronoi boundary.
    #[test]
    fn pick_cell_refines_bucket_lookup() {
        let mesh = mesh::build(3000, 42);
        // Query a point between two cells — pick_cell should return the closer one.
        let [x0, y0] = mesh.points[200];
        let neighbors: Vec<usize> = {
            let lo = mesh.cells.i[200] as usize;
            let hi = mesh.cells.i[201] as usize;
            mesh.cells.c[lo..hi].iter().map(|&n| n as usize).collect()
        };
        assert!(!neighbors.is_empty(), "cell 200 should have neighbors");
        // Find the nearest neighbor to cell 200.
        let nearest_nb = neighbors.iter().copied().min_by_key(|&nb| {
            let [nx, ny] = mesh.points[nb];
            ((nx - x0).powi(2) + (ny - y0).powi(2)) as i64
        }).unwrap();
        // Query a point 70% of the way from cell 200 toward its nearest neighbor.
        // That should pick the neighbor (it's closer to the midpoint).
        let [nx, ny] = mesh.points[nearest_nb];
        let qx = x0 + 0.7 * (nx - x0);
        let qy = y0 + 0.7 * (ny - y0);
        let picked = pick_cell(&mesh, qx, qy).expect("pick_cell should return Some");
        assert_eq!(picked, nearest_nb as u32, "query 70% toward nearest neighbor should pick the neighbor");
    }

    /// Step 2.5.4: `pick_cell` handles OOB coordinates gracefully (returns a
    /// valid cell id, not a panic).
    #[test]
    fn pick_cell_handles_out_of_bounds() {
        let mesh = mesh::build(500, 42);
        // Far OOB — should still return a valid cell.
        let picked = pick_cell(&mesh, -9999.0, -9999.0);
        assert!(picked.is_some(), "pick_cell should not return None for OOB");
        // Zero-cell mesh edge case.
        // (We can't easily create a zero-cell mesh via mesh::build, so we
        // rely on the `n == 0` guard inside pick_cell.)
    }

    /// `build_range` constructs a ridge path from `start` to `end` by greedy
    /// neighbor walk minimizing squared distance to `end`, with a `randomness`
    /// chance to halve the distance. Contracts: path starts at `start`,
    /// ends at `end` (or stops if no unvisited neighbor), contains no
    /// duplicate cells, and distance to `end` generally decreases along the
    /// path (stochastic `randomness` may occasionally increase it, but it must
    /// terminate). We test the deterministic core by setting `randomness=0.0`.
    #[test]
    fn build_range_walks_to_end() {
        let mesh = mesh::build(1000, 42);
        let view = MeshView::from_mesh(&mesh);
        let mut rng = StdRng::seed_from_u64(5);
        let start = 100;
        let end = 500;
        let path = build_range(&view, &mut rng, start, end, 0.0); // deterministic
        assert!(!path.is_empty());
        assert_eq!(path[0], start, "path must start at start");
        assert_eq!(*path.last().unwrap(), end, "path must reach end (with randomness=0)");
        // No duplicates
        let mut seen = std::collections::HashSet::new();
        for &c in &path {
            assert!(seen.insert(c), "duplicate cell {c} in ridge path");
        }
        // Distance to end should generally decrease (monotonic when randomness=0)
        let points = &view.points;
        let end_pt = points[end];
        let mut prev_dist = f64::INFINITY;
        for &c in &path {
            let p = points[c];
            let dx = p[0] - end_pt[0];
            let dy = p[1] - end_pt[1];
            let dist = dx * dx + dy * dy;
            if dist > prev_dist {
                // With randomness=0, greedy walk should be monotonic.
                panic!("distance to end increased at cell {c}: {dist} > {prev_dist} (randomness=0)");
            }
            prev_dist = dist;
        }
    }

    /// `add_hill` must raise the seed cell (and neighbors via BFS) above the
    /// water baseline. After a hill on a flat zero map, the start cell is
    /// strictly higher than the untouched baseline (0). `add_ridge` with
    /// `raise=true` (Range) raises a path; `raise=false` (Trough) lowers it.
    #[test]
    fn add_hill_raises_and_add_ridge_raise_vs_trough() {
        let mesh = mesh::build(1000, 42);
        let view = MeshView::from_mesh(&mesh);
        let mut rng = StdRng::seed_from_u64(6);
        let n = view.points.len();
        let blob_power = get_blob_power(n);
        let line_power = get_line_power(n);
        let hill_budget = n / 10;
        let ridge_budget = n / 10;

        // add_hill on flat zero map
        let mut h = vec![0u8; n];
        let start = 200;
        add_hill(&view, &mut h, &mut rng, start, blob_power, hill_budget);
        assert!(h[start] > 0, "hill start cell should be > 0, got {}", h[start]);
        // Some neighbors also raised (BFS spread)
        let lo = view.cells.i[start] as usize;
        let hi = view.cells.i[start + 1] as usize;
        let any_neighbor_raised = view.cells.c[lo..hi].iter().any(|&c| h[c as usize] > 0);
        assert!(any_neighbor_raised, "hill should spread to at least one neighbor");

        // add_ridge raise=true (Range) — path cells go up
        let mut rng2 = StdRng::seed_from_u64(7);
        let mut h2 = vec![0u8; n];
        let start2 = 100;
        let end2 = 400;
        add_ridge(&view, &mut h2, &mut rng2, start2, end2, 0.0, line_power, true, ridge_budget);
        assert!(h2[start2] > 0, "ridge start should be raised");
        assert!(h2[end2] > 0, "ridge end should be raised");

        // add_ridge raise=false (Trough) — path cells go down (below 0 → clamp to 0,
        // but the change is negative so neighbors are lower than untouched)
        let mut rng3 = StdRng::seed_from_u64(8);
        let mut h3 = vec![50u8; n]; // flat at 50
        add_ridge(&view, &mut h3, &mut rng3, start2, end2, 0.0, line_power, false, ridge_budget);
        assert!(h3[start2] < 50, "trough start should be lowered from 50, got {}", h3[start2]);
        assert!(h3[end2] < 50, "trough end should be lowered from 50, got {}", h3[end2]);
    }

    /// `multiply_land` scales only cells ≥ SEA_LEVEL (land). Water cells
    /// (<20) are untouched. The math: `new = (old - 20) * mult + 20`, clamped
    /// to [0,100]. A multiplier of 1.0 is identity; 2.0 doubles the height
    /// above sea level; 0.5 halves it.
    #[test]
    fn multiply_land_only_affects_land() {
        let mut h = vec![
            0u8,  // deep water
            10,   // shallow water
            20,   // exactly at sea level (land)
            30,   // land
            80,   // high land
            100,  // peak
        ];
        let original = h.clone();
        multiply_land(&mut h, 2.0);
        // Water unchanged
        assert_eq!(h[0], 0);
        assert_eq!(h[1], 10);
        // Sea level: (20-20)*2 + 20 = 20
        assert_eq!(h[2], 20);
        // 30: (30-20)*2 + 20 = 40
        assert_eq!(h[3], 40);
        // 80: (80-20)*2 + 20 = 140 → clamp 100
        assert_eq!(h[4], 100);
        // 100: (100-20)*2 + 20 = 180 → clamp 100
        assert_eq!(h[5], 100);

        // Identity multiplier
        let mut h2 = original.clone();
        multiply_land(&mut h2, 1.0);
        assert_eq!(h2, original);

        // Halve
        let mut h3 = original.clone();
        multiply_land(&mut h3, 0.5);
        assert_eq!(h3[2], 20);     // sea level unchanged
        assert_eq!(h3[3], 25);     // (30-20)*0.5 + 20 = 25
        assert_eq!(h3[4], 50);     // (80-20)*0.5 + 20 = 50
        assert_eq!(h3[5], 60);     // (100-20)*0.5 + 20 = 60
    }

    /// `parse_template` + `default_template` produce exactly 8 steps with the
    /// expected tool tags and non-empty count ranges. This guards against
    /// accidental template corruption.
    #[test]
    fn default_template_parses_correctly() {
        let steps = parse_template(default_template());
        assert_eq!(steps.len(), 8);
        let tools: Vec<_> = steps.iter().map(|s| match s.tool {
            Tool::Hill => "Hill",
            Tool::Pit => "Pit",
            Tool::Range => "Range",
            Tool::Trough => "Trough",
            Tool::Smooth => "Smooth",
            Tool::Mask => "Mask",
            Tool::Multiply => "Multiply",
        }).collect();
        assert_eq!(tools, ["Hill", "Range", "Range", "Range", "Trough", "Pit", "Smooth", "Mask"]);
        // Count fields present
        for s in &steps {
            assert!(!s.a2.is_empty(), "count field (a2) missing for {:?}", s.tool);
        }
    }

    /// `generate(&mesh, seed)` returns a `Vec<u8>` of length exactly N (the
    /// mesh's cell count). This is an explicit contract — downstream `Grid`
    /// construction expects `h.len() == N`.
    #[test]
    fn generate_output_length_equals_n() {
        for n in [1000u32, 3000, 10000, 60000] {
            let mesh = mesh::build(n, 42);
            let h = generate(&mesh, 42);
            assert_eq!(h.len(), mesh.points.len(), "N={n}: output length mismatch");
        }
    }

    /// Determinism holds for a second (mesh, seed) pair. The existing
    /// `deterministic_same_seed` only checks seed 42 @ N=2000. This test
    /// confirms bit-identical output across runs for a different seed and
    /// cell count, catching any path that accidentally depends on absolute
    /// seed value or mesh size rather than treating the RNG as an opaque key.
    #[test]
    fn deterministic_across_seeds_and_sizes() {
        for (n, seed) in [(1500u32, 12345u64), (5000, 99999), (10000, 7)] {
            let mesh = mesh::build(n, seed as u32);
            let a = generate(&mesh, seed);
            let b = generate(&mesh, seed);
            assert_eq!(a, b, "N={n} seed={seed}: heightmap not deterministic");
        }
    }

    /// `p(rng, prob)` returns `true` with the given probability. Boundary
    /// conditions: prob >= 1.0 → always true; prob <= 0.0 → always false.
    /// In-range: statistical sanity (not exact, but paired RNGs must agree).
    #[test]
    fn p_probability_boundaries() {
        let mut rng1 = StdRng::seed_from_u64(10);
        // >= 1.0 always true
        for _ in 0..10 {
            assert!(p(&mut rng1, 1.0));
            assert!(p(&mut rng1, 1.5));
        }
        // <= 0.0 always false
        for _ in 0..10 {
            assert!(!p(&mut rng1, 0.0));
            assert!(!p(&mut rng1, -0.5));
        }
        // In-range: paired RNGs produce identical sequences
        let mut rng3 = StdRng::seed_from_u64(11);
        let mut rng4 = StdRng::seed_from_u64(11);
        for _ in 0..100 {
            assert_eq!(p(&mut rng3, 0.5), p(&mut rng4, 0.5));
        }
        // Prob 0.5 should produce ~50% true (very loose bound)
        let mut rng5 = StdRng::seed_from_u64(12);
        let trues = (0..1000).filter(|_| p(&mut rng5, 0.5)).count();
        assert!((400..=600).contains(&trues), "0.5 prob yielded {trues}/1000 true");
    }
}

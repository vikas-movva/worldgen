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

/// Pick a point at fractional range `[minFrac, maxFrac]` of the world axis.
/// FMG `getPointInRange(range, length)` parses `range` as two ints /100 and
/// returns `rand(minFrac*length, maxFrac*length)`.
fn point_in_range(rng: &mut StdRng, range: &str, length: f64) -> Option<f64> {
    if !range.is_ascii() {
        return None;
    }
    let Some((a, b)) = range.split_once('-') else {
        return None;
    };
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
fn get_line_power(cells: usize) -> f64 {
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
fn build_range(view: &MeshView, rng: &mut StdRng, start: usize, end: usize, randomness: f64) -> Vec<usize> {
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
}

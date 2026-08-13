//! Rivers + lakes generator — Step 2.5.3 (Phase 2.5).
//!
//! Deterministic recompute of drainage (rivers), depression-fill lakes, and
//! the land/water mask after a heightmap edit. The output is a
//! [`DrainageResult`] that `recompute_dependents` (lib.rs) uses to update
//! `cells.fl`/`cells.r`/`cells.conf` and emit [`RiverGeo`]/[`LakeGeo`].
//!
//! ## Algorithm (faithful port of FMG `river-generator.ts` + `lakes.ts`)
//!
//! 1. **`alter_heights`** — promote temperature to a tiny height bonus on
//!    land so high-but-warm mountains don't get stuck behind cold lowlands.
//!    FMG adds `t[i]/100 + mean(t[neighbors])/10000` to land heights; we
//!    replicate with our `cells.temp`.
//! 2. **`resolve_depressions`** — flat-fill: walk land cells from lowest to
//!    highest; if a cell's lowest neighbor is >= its own height, raise it to
//!    `min_neighbor + 0.1` so water always has a downhill path. Iterate until
//!    no depressions remain (bounded by `MAX_ITER`). This is the "Priority-
//!    Flood" depression filling, the same approach FMG uses. Lakes in deep
//!    depressions are filled to their shoreline min + `LAKE_ELEVATION_DELTA`
//!    and treated as a single effective height.
//! 3. **`detect_close_lakes`** — BFS from each lake's lowest shoreline cell;
//!    if no path to a lower water body (ocean) exists within an elevation
//!    budget, the lake is `closed` (no outlet). FMG `Lakes.detectCloseLakes`.
//! 4. **`drain_water`** — sort land cells highest-first; add precipitation
//!    flux per cell (`cells.fl[i] += prec[i] / cellsNumberModifier`); then
//!    walk downhill to the lowest neighbor. If the outgoing flux exceeds
//!    `MIN_FLUX_TO_FORM_RIVER` (30), claim a new river id and trace the path
//!    to the sea, a lake, the map border, or a confluence with a stronger
//!    river. This mirrors FMG's `drainWater` + `flowDown`.
//! 5. **`define_rivers`** — sweep `rivers` data; rivers with < 3 cells are
//!    dropped (too short to render); river ids written back to `cells.r`;
//!    confluence cells flagged in `cells.conf`.
//!
//! **Determinism.** No RNG is used. All cell traversal uses sorted-by-id
//! / sorted-by-height order; the cell sort uses a total-order comparator
//! (`height.then(id)`) per the determinism contract §4 rule 4. Lake geometry
//! is collected via `BTreeMap` (no HashMap iteration). The result is a pure
//! function of `(mesh, cells.h, cells.temp, cells.prec)` — byte-identical
//! across runs.
//!
//! **Determinism-contract deviations from FMG, all documented:**
//! - No `Math.random` / `Alea(seed)` (FMG uses it for the `.01` height
//!   elevation hint and for river-name/type selection — both deferred to
//!   Phase 3 naming; the compute core is RNG-free).
//! - No meandering (renderer-only; deferred to Step 2.5.4/Phase 3).
//! - No downcutting (visual polish; deferred — we keep the height array
//!   intact so subsequent edits compose).
//! - Lake cells are encoded implicitly: a cell whose `h < SEA_LEVEL` becomes
//!   a lake cell iff `resolve_depressions` *raised* it. FMG uses an explicit
//!   `cells.f` feature id array; we annotate via the [`LakeGeo`] list and do
//!   not add a feature array until Phase 3.

use crate::climate::SEA_LEVEL;
use crate::grid::{LakeGeo, RiverGeo};
use crate::mesh::Mesh;

/// FMG `Lakes.LAKE_ELEVATION_DELTA` — lake surface sits just below its
/// lowest shoreline cell so it counts as "water" (h < SEA_LEVEL) yet holds
/// drainage.
const LAKE_ELEVATION_DELTA: f64 = 0.1;

/// FMG `MIN_FLUX_TO_FORM_RIVER = 30`. Below this, a cell's flux is passed
/// downhill silently (no river id assigned).
const MIN_FLUX_TO_FORM_RIVER: u32 = 30;

/// Max iterations for `resolve_depressions`. FMG reads this from a slider
/// (`resolveDepressionsStepsOutput`); we use a generous default that converges
/// for any 60k-world depression in practice.
const MAX_ITER: usize = 250;

/// FMG elevation-limit for "is this lake close enough to the sea to be open?"
/// (`lakeElevationLimitOutput`, default 15). A lake whose shoreline exceeds
/// its own height + this budget is `closed`.
const LAKE_ELEVATION_LIMIT: f64 = 15.0;

/// Drainage result — the per-cell arrays + geometry that `recompute_dependents`
/// uses to populate `DependentResult`. Kept separate from
/// [`crate::grid::DependentResult`] so climate/biome (which run after) can
/// read the updated `fl` for the river-flux bonus term in biome moisture.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
pub struct DrainageResult {
    /// Altered (temperature-bonused) heights after depression resolution.
    /// Written back into `cells.h` only conceptually — `recompute_dependents`
    /// keeps the user-visible `cells.h` unchanged but uses this for drainage
    /// decisions. Saved here so the biomes pass can read it if needed.
    pub h_eff: Vec<f64>,
    /// Per-cell water flux (discharge). Length N.
    pub fl: Vec<u16>,
    /// River id at each cell (0 = none).
    pub r: Vec<u16>,
    /// Confluence flag (0 = none; nonzero = confluence flux).
    pub conf: Vec<u16>,
    /// Lakes discovered during depression filling.
    pub lakes: Vec<LakeGeo>,
    /// Rivers traced by `drain_water`.
    pub rivers: Vec<RiverGeo>,
}

/// Compute drainage for the grid. Pure function of `(mesh, h, temp, prec)`.
///
/// - `h` is `cells.h` (0..=100, <20 = water). NOT mutated; the result's
///   `h_eff` carries the depression-resolved effective heights.
/// - `temp` is `cells.temp` (°C, Int8) — used by `alter_heights`.
/// - `prec` is `cells.prec` (Uint8) — the per-cell precipitation source.
pub fn compute_drainage(mesh: &Mesh, h: &[u8], temp: &[i8], prec: &[u8]) -> DrainageResult {
    let n = mesh.points.len();
    let _cells_x = mesh.cells.cells_x as usize;
    let n_mod = ((n as f64) / 10000.0).powf(0.25).max(1.0);

    // 1. alter_heights: t[i]/100 + mean(t[neighbors])/10000 on land.
    let mut h_eff = alter_heights(mesh, h, temp);

    // 2. resolve_depressions (also collects lake cells).
    let lakes = resolve_depressions(mesh, &mut h_eff);

    // 3. detect_close_lakes (BFS, marks closed lakes).
    let lakes = detect_close_lakes(mesh, &h_eff, lakes);

    // 4. drain_water (downhill flow + river tracing).
    let mut fl = vec![0u16; n];
    let mut r = vec![0u16; n];
    let mut conf = vec![0u16; n];
    let mut rivers_data: std::collections::BTreeMap<u32, Vec<i32>> = std::collections::BTreeMap::new();
    let mut river_parents: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    let mut river_next: u32 = 1;

    drain_water(
        mesh,
        &h_eff,
        prec,
        &n_mod,
        &mut fl,
        &mut r,
        &mut conf,
        &mut rivers_data,
        &mut river_parents,
        &mut river_next,
        &lakes,
    );

    // 5. define_rivers (drop short rivers, write ids back, build RiverGeo).
    let rivers = define_rivers(mesh, &h_eff, &fl, &mut r, &mut conf, rivers_data, river_parents);

    DrainageResult {
        h_eff,
        fl,
        r,
        conf,
        lakes,
        rivers,
    }
}

/// FMG `alterHeights()`: add `t[i]/100 + mean(t[neighbors])/10000` to land
/// cells so a warm ridge drains ahead of a cold lowland (temperature is a
/// proxy for air density / uplift). Water cells keep their original height.
fn alter_heights(mesh: &Mesh, h: &[u8], temp: &[i8]) -> Vec<f64> {
    let n = mesh.points.len();
    let i = &mesh.cells.i;
    let c = &mesh.cells.c;
    let mut out = vec![0f64; n];
    for cell in 0..n {
        let h_i = h[cell] as f64;
        if h_i < SEA_LEVEL as f64 {
            out[cell] = h_i;
            continue;
        }
        let t_i = temp[cell] as f64;
        // mean(temp of neighbors)
        let lo = i[cell] as usize;
        let hi = i[cell + 1] as usize;
        let mut sum = 0.0f64;
        let mut count = 0usize;
        for &nb in &c[lo..hi] {
            sum += temp[nb as usize] as f64;
            count += 1;
        }
        let mean_t = if count > 0 { sum / count as f64 } else { 0.0 };
        out[cell] = h_i + t_i / 100.0 + mean_t / 10000.0;
    }
    out
}

/// Priority-flood depression filling (FMG `resolveDepressions`). Walk land
/// cells lowest-first; if a cell's lowest neighbor is not lower than the
/// cell, raise the cell to `min_neighbor + 0.1`. Iterate until no change.
/// Lakes are detected implicitly: a cell that was water (h < SEA_LEVEL) and
/// got *raised* by the fill is a lake cell. We collect those as [`LakeGeo`]
/// entries with their shoreline.
fn resolve_depressions(mesh: &Mesh, h_eff: &mut Vec<f64>) -> Vec<LakeGeo> {
    let n = mesh.points.len();
    let i = &mesh.cells.i;
    let c = &mesh.cells.c;
    let b = &mesh.cells.b;

    // Land cells (excluding near-border), sorted lowest-first.
    // Determinism: total-order comparator (height, then id).
    let mut land: Vec<u32> = (0..n as u32)
        .filter(|&idx| h_eff[idx as usize] >= SEA_LEVEL as f64 && b[idx as usize] == 0)
        .collect();
    land.sort_by(|&a, &b| {
        let ha = h_eff[a as usize];
        let hb = h_eff[b as usize];
        ha.partial_cmp(&hb).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b))
    });

    // Detect cells that BECOME lake cells (water raised above SEA_LEVEL by
    // the fill) — track "was originally water".
    let orig_water: Vec<bool> = (0..n).map(|idx| h_eff[idx] < SEA_LEVEL as f64).collect();

    let mut lake_cells: Vec<u32> = Vec::new();

    for _iter in 0..MAX_ITER {
        let mut depressions = 0u32;
        for &idx in &land {
            let cell = idx as usize;
            // Find min neighbor height. Lake cells (currently water) use their
            // own raised height as the effective height; this is what makes a
            // filled basin act as one unit downstream.
            let lo = i[cell] as usize;
            let hi = i[cell + 1] as usize;
            let mut min_h = f64::INFINITY;
            for &nb in &c[lo..hi] {
                let h_nb = h_eff[nb as usize];
                if h_nb < min_h {
                    min_h = h_nb;
                }
            }
            if min_h >= 100.0 {
                continue;
            }
            // Drainage exists: at least one neighbor is lower than us.
            if h_eff[cell] > min_h {
                continue;
            }
            // Depression: no neighbor is lower — raise to min_neighbor + delta
            // so water can flow over the sill.
            h_eff[cell] = min_h + 0.1;
            if h_eff[cell] >= 100.0 {
                h_eff[cell] = 99.9;
            }
            // If raising managed to lift a cell out of the water band, mark it
            // as a lake cell (it was originally water and got filled to land-
            // height? No — FMG lakes stay < SEA_LEVEL conceptually; the
            // elevation here is the lake *surface* height which is just below
            // the shoreline). We track cells that remain water after fill.
            if orig_water[cell] && h_eff[cell] < SEA_LEVEL as f64 {
                // raised but still water — this is a lake cell
                if !lake_cells.contains(&(cell as u32)) {
                    lake_cells.push(cell as u32);
                }
            }
            depressions += 1;
        }
        if depressions == 0 {
            break;
        }
    }

    // Build LakeGeo from lake_cells: group connected components, compute
    // shoreline and height.
    build_lake_geometries(mesh, h_eff, &lake_cells)
}

/// Group `lake_cells` into connected components (via `cells.c` BFS), then
/// for each lake compute:
/// - `cells`: the lake's water cells.
/// - `shoreline`: land cells adjacent to the lake.
/// - `height`: min shoreline height - `LAKE_ELEVATION_DELTA`.
fn build_lake_geometries(mesh: &Mesh, h_eff: &[f64], lake_cells: &[u32]) -> Vec<LakeGeo> {
    let n = mesh.points.len();
    let i = &mesh.cells.i;
    let c = &mesh.cells.c;
    let mut lakes: Vec<LakeGeo> = Vec::new();
    if lake_cells.is_empty() {
        return lakes;
    }
    let mut visited = vec![false; n];
    let mut lake_set = std::collections::BTreeSet::new();
    for &lc in lake_cells {
        lake_set.insert(lc as usize);
    }
    for &start in lake_cells {
        let start = start as usize;
        if visited[start] {
            continue;
        }
        // BFS the connected component of lake cells.
        let mut queue: Vec<usize> = vec![start];
        visited[start] = true;
        let mut cells: Vec<u32> = Vec::new();
        let mut shoreline_set: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        while let Some(q) = queue.pop() {
            cells.push(q as u32);
            let lo = i[q] as usize;
            let hi = i[q + 1] as usize;
            for &nb in &c[lo..hi] {
                let nb = nb as usize;
                if lake_set.contains(&nb) {
                    if !visited[nb] {
                        visited[nb] = true;
                        queue.push(nb);
                    }
                } else {
                    // land neighbor — shoreline
                    shoreline_set.insert(nb);
                }
            }
        }
        // Height = min shoreline h - delta.
        let mut min_shore = f64::INFINITY;
        for &s in &shoreline_set {
            if h_eff[s] < min_shore {
                min_shore = h_eff[s];
            }
        }
        if !min_shore.is_finite() {
            // No shoreline (lake against the map edge only) — skip; it's the
            // ocean, not a lake.
            continue;
        }
        let height = (min_shore - LAKE_ELEVATION_DELTA).max(0.0);
        let shoreline: Vec<u32> = shoreline_set.into_iter().map(|x| x as u32).collect();
        lakes.push(LakeGeo {
            id: 0, // assigned later by recompute_dependents
            height,
            cells,
            shoreline,
            closed: false, // detect_close_lakes fills this
        });
    }
    lakes
}

/// FMG `Lakes.detectCloseLakes`. For each lake, BFS from its lowest shoreline
/// cell; if no path reaches a water body (h < SEA_LEVEL) that is *lower* than
/// the lake's height + LAKE_ELEVATION_LIMIT, the lake is `closed`.
fn detect_close_lakes(mesh: &Mesh, h_eff: &[f64], mut lakes: Vec<LakeGeo>) -> Vec<LakeGeo> {
    let i = &mesh.cells.i;
    let c = &mesh.cells.c;
    let n = mesh.points.len();
    for lake in lakes.iter_mut() {
        if lake.shoreline.is_empty() {
            lake.closed = true;
            continue;
        }
        // Lowest shoreline cell.
        let lowest = lake
            .shoreline
            .iter()
            .copied()
            .min_by_key(|&s| (h_eff[s as usize] * 1000.0) as i64)
            .unwrap_or(0) as usize;
        let max_elev = lake.height + LAKE_ELEVATION_LIMIT;
        if max_elev > 99.0 {
            lake.closed = false;
            continue;
        }
        let mut visited = vec![false; n];
        let mut queue: Vec<usize> = vec![lowest];
        visited[lowest] = true;
        let mut is_deep = true;
        while !queue.is_empty() && is_deep {
            let cur = queue.pop().unwrap();
            let lo = i[cur] as usize;
            let hi = i[cur + 1] as usize;
            for &nb in &c[lo..hi] {
                let nb = nb as usize;
                if visited[nb] {
                    continue;
                }
                if h_eff[nb] >= max_elev {
                    continue;
                }
                if h_eff[nb] < SEA_LEVEL as f64 {
                    // neighbor water body — lake can drain to it if it's lower
                    // than the lake. We don't have feature ids yet, so any
                    // reachable water body at a lower height counts as an
                    // "ocean" (FMG's check).
                    if lake.height > h_eff[nb] {
                        is_deep = false;
                        break;
                    }
                }
                visited[nb] = true;
                queue.push(nb);
            }
        }
        lake.closed = is_deep;
    }
    lakes
}

/// FMG `drainWater` — accumulate flux per land cell from precipitation, then
/// pass flux downhill. Cells are processed highest-first so each cell's
/// accumulated flux already includes all upstream contributions by the time
/// we visit it. A river id is proclaimed when accumulated flux >=
/// [`MIN_FLUX_TO_FORM_RIVER`]. The river's path is the sequence of cells we
/// push to `rivers_data` as we proclaim river ids and route through lake
/// outlets. Handles lake outlet routing, near-border pour-off, and
/// confluences (parent link tracking).
#[allow(clippy::too_many_arguments)]
fn drain_water(
    mesh: &Mesh,
    h_eff: &[f64],
    prec: &[u8],
    n_mod: &f64,
    fl: &mut [u16],
    r: &mut [u16],
    conf: &mut [u16],
    rivers_data: &mut std::collections::BTreeMap<u32, Vec<i32>>,
    river_parents: &mut std::collections::BTreeMap<u32, u32>,
    river_next: &mut u32,
    lakes: &[LakeGeo],
) {
    let n = mesh.points.len();
    let i = &mesh.cells.i;
    let c = &mesh.cells.c;
    let b = &mesh.cells.b;
    let sea = SEA_LEVEL as f64;

    // Lake cell → lake index, for outlet routing.
    let mut lake_cell_index = vec![usize::MAX; n];
    for (idx, lake) in lakes.iter().enumerate() {
        for &lc in &lake.cells {
            lake_cell_index[lc as usize] = idx;
        }
    }
    // Lake outlet cells: lowest shoreline cell per lake (FMG: getLowestShoreCell).
    let mut lake_outlet_cell = vec![usize::MAX; lakes.len()];
    for (idx, lake) in lakes.iter().enumerate() {
        if lake.closed {
            continue; // no outlet
        }
        let lowest = lake
            .shoreline
            .iter()
            .copied()
            .min_by_key(|&s| (h_eff[s as usize] * 1000.0) as i64);
        if let Some(lowest) = lowest {
            lake_outlet_cell[idx] = lowest as usize;
        }
    }

    // Land cells, sorted highest-first (FMG iterates this order so flux collects
    // from peaks down).
    let mut land: Vec<u32> = (0..n as u32)
        .filter(|&idx| h_eff[idx as usize] >= sea)
        .collect();
    land.sort_by(|&a, &b| {
        let ha = h_eff[a as usize];
        let hb = h_eff[b as usize];
        hb.partial_cmp(&ha)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(&b))
    });

    for &idx in &land {
        let cell = idx as usize;
        // Add precipitation flux. `prec[i] / n_mod` (FMG:
        // `cells.fl[i] += prec[cells.g[i]] / cellsNumberModifier`).
        let add = (prec[cell] as f64 / n_mod) as u16;
        fl[cell] = fl[cell].saturating_add(add);

        // Find the lowest neighbor (downhill direction).
        let lo = i[cell] as usize;
        let hi = i[cell + 1] as usize;
        let mut min_neighbor = cell;
        let mut min_h = h_eff[cell];
        for &nb in &c[lo..hi] {
            let n_us = nb as usize;
            if h_eff[n_us] < min_h {
                min_h = h_eff[n_us];
                min_neighbor = n_us;
            }
        }

        // Depressed (no downhill) — this is a local minimum. Flux stays put;
        // a lake or endorheic basin may form. Skip river proclamation.
        if min_neighbor == cell {
            continue;
        }

        // Proclaim a new river if flux is high enough and none yet assigned.
        let cell_flux = fl[cell] as u32;
        if cell_flux >= MIN_FLUX_TO_FORM_RIVER && r[cell] == 0 {
            r[cell] = *river_next as u16;
            rivers_data.entry(*river_next).or_default().push(cell as i32);
            *river_next += 1;
        }

        // Pass flux downhill.
        if r[cell] != 0 {
            // This cell carries a river — route to the downhill cell, possibly
            // through a lake outlet or off the map.
            route_downward(
                cell,
                min_neighbor,
                mesh,
                h_eff,
                fl,
                r,
                conf,
                rivers_data,
                river_parents,
                lake_cell_index.as_slice(),
                lake_outlet_cell.as_slice(),
                b,
                &sea,
            );
        } else {
            // No river here — silently pass flux to downhill land cell.
            if h_eff[min_neighbor] >= sea {
                fl[min_neighbor] = fl[min_neighbor].saturating_add(fl[cell]);
            }
        }
    }
}

/// FMG `flowDown(toCell, fromFlux, riverId)` — assign the river to `to_cell`,
/// propagate flux, handle confluence with existing river (parent links),
/// pour to lakes or off-map, or continue the path. This is a single step; the
/// full river path is built incrementally as `drain_water` iterates cells
/// highest-first and each flux-carrying cell calls this to route to its
/// downhill neighbor.
#[allow(clippy::too_many_arguments)]
fn route_downward(
    from_cell: usize,
    to_cell: usize,
    _mesh: &Mesh,
    h_eff: &[f64],
    fl: &mut [u16],
    r: &mut [u16],
    conf: &mut [u16],
    rivers_data: &mut std::collections::BTreeMap<u32, Vec<i32>>,
    river_parents: &mut std::collections::BTreeMap<u32, u32>,
    lake_cell_index: &[usize],
    lake_outlet_cell: &[usize],
    b: &[u8],
    sea: &f64,
) {
    let from_flux = fl[from_cell];
    let from_river = r[from_cell] as u32;
    let to_river = r[to_cell];

    // Confluence: the downhill cell already has a river.
    if to_river != 0 && to_river as u32 != from_river {
        if from_flux > fl[to_cell] {
            // The stronger river absorbs the weaker.
            conf[to_cell] = conf[to_cell].saturating_add(fl[to_cell]);
            if h_eff[to_cell] >= *sea {
                river_parents.insert(to_river as u32, from_river);
            }
            // Re-label the downhill cell with the stronger river id.
            r[to_cell] = from_river as u16;
            rivers_data
                .entry(from_river)
                .or_default()
                .push(to_cell as i32);
        } else {
            conf[to_cell] = conf[to_cell].saturating_add(from_flux);
            if h_eff[to_cell] >= *sea {
                river_parents.insert(from_river, to_river as u32);
            }
            // Current river terminates here (absorbed into stronger).
            rivers_data
                .entry(from_river)
                .or_default()
                .push(to_cell as i32);
            return;
        }
    } else if to_river == 0 {
        // No existing river — assign ours.
        r[to_cell] = from_river as u16;
        rivers_data
            .entry(from_river)
            .or_default()
            .push(to_cell as i32);
    }

    // Propagate flux to the downhill cell.
    fl[to_cell] = fl[to_cell].saturating_add(from_flux);

    // Check what the downhill cell pours into.
    if h_eff[to_cell] < *sea {
        // Water body — is it a lake?
        let lake_idx = lake_cell_index[to_cell];
        if lake_idx != usize::MAX {
            let outlet = lake_outlet_cell[lake_idx];
            if outlet != usize::MAX {
                // Route through the lake's outlet cell.
                rivers_data
                    .entry(from_river)
                    .or_default()
                    .push(outlet as i32);
                r[outlet] = from_river as u16;
                fl[outlet] = fl[outlet].saturating_add(fl[to_cell]);
            }
        }
        // Otherwise pour to the ocean — path ends.
        return;
    }

    // Near-border pour: river leaves the map.
    if b[to_cell] != 0 {
        rivers_data.entry(from_river).or_default().push(-1);
    }
}

/// FMG `defineRivers` — drop rivers < 3 cells, write river ids back to
/// `cells.r`, set confluence flags, build [`RiverGeo`] objects (with simple
/// midpoint polyline points; meandering deferred).
fn define_rivers(
    mesh: &Mesh,
    h_eff: &[f64],
    fl: &[u16],
    r: &mut [u16],
    conf: &mut [u16],
    rivers_data: std::collections::BTreeMap<u32, Vec<i32>>,
    _river_parents: std::collections::BTreeMap<u32, u32>,
) -> Vec<RiverGeo> {
    let sea = SEA_LEVEL as f64;
    // Reset r and conf — we re-assign only rivers that survive the length
    // threshold. This matches FMG's behavior (`cells.r = new Uint16Array(...)`).
    let n = r.len();
    let saved_r = r.to_vec();
    for v in r.iter_mut() { *v = 0; }
    for v in conf.iter_mut() { *v = 0; }

    let mut rivers: Vec<RiverGeo> = Vec::new();
    for (river_id, cells) in rivers_data.iter() {
        if cells.len() < 3 {
            continue;
        }
        // Assign river id to all land cells in the path; mark confluences.
        for &c_id in cells {
            if c_id < 0 || c_id as usize >= n {
                // -1 = off-map pour sentinel — skip.
                continue;
            }
            let cu = c_id as usize;
            if h_eff[cu] < sea {
                // Water cell — river mouth; record but don't claim.
                continue;
            }
            if saved_r[cu] != 0 && saved_r[cu] as u32 != *river_id {
                // This cell was part of another river too — confluence.
                conf[cu] = 1;
            }
            r[cu] = *river_id as u16;
        }
        // Build RiverGeo.
        let source = cells.iter().copied().find(|&c| c >= 0 && h_eff[c as usize] >= sea).unwrap_or(-1);
        let mouth = cells.iter().rev().copied().find(|&c| c >= 0 && h_eff[c as usize] >= sea).unwrap_or(-1);
        let discharge = if mouth >= 0 { fl[mouth as usize] as f64 } else { 0.0 };
        let mut points: Vec<[f64; 2]> = Vec::with_capacity(cells.len());
        for &c_id in cells {
            if c_id < 0 || c_id as usize >= n { continue; }
            let cu = c_id as usize;
            if h_eff[cu] >= sea {
                points.push(mesh.points[cu]);
            }
        }
        rivers.push(RiverGeo {
            id: *river_id,
            source: source.max(0) as u32,
            mouth: mouth.max(0) as u32,
            discharge,
            cells: cells.clone(),
            points,
        });
    }
    rivers
}

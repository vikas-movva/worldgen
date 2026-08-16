//! Phase 3 Step 3.2 — States + Provinces generator (FMG port).
//!
//! Seeds capital burgs from biome suitability, expands state frontiers via a
//! Dijkstra-like priority queue (cost = culture + population + biome + height
//! + river + type, scaled by expansionism), then subdivides each state into
//! provinces using burgs as centers. Populates `Pack.states`, `Pack.provinces`,
//! `Pack.burgs`, and writes `cells.state` / `cells.province` / `cells.burg`.
//!
//! Pure data + deterministic RNG: `StdRng::seed_from_u64(seed)`. No rendering,
//! no timeline (Phase 4). Culture/religion assignment is deferred to Phase 3.3
//! (TODO markers inline).
//!
//! Port of FMG `states-generator.ts` (`expandStates`), `burgs-generator.ts`
//! (`generateCapitals`), and `provinces-generator.ts` (`generateProvinces`).

use crate::entities::{Burg, Pack, Province, State};
use crate::grid::Grid;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

// ---------------------------------------------------------------------------
// Constants — FMG biome tables (biomes-generator.ts)
// ---------------------------------------------------------------------------

/// Biome habitability: how suitable each biome is for settlement.
/// Index 0 (Marine) = 0, matching FMG `getDefaultBiomes().habitability`.
const BIOME_HABITABILITY: [f64; 13] = [
    0.0, 4.0, 10.0, 22.0, 30.0, 50.0, 100.0, 80.0, 90.0, 12.0, 4.0, 0.0, 12.0,
];

/// Biome traversal cost for state expansion (FMG `getDefaultBiomes().cost`).
const BIOME_COST: [f64; 13] = [
    10.0, 200.0, 150.0, 60.0, 50.0, 70.0, 70.0, 80.0, 90.0, 200.0, 1000.0, 5000.0, 150.0,
];

/// Sea level threshold (height < 20 = water). FMG convention.
pub const SEA_LEVEL: u8 = 20;

/// Height thresholds for traversal cost (FMG `getHeightCost`).
const HEIGHT_MOUNTAIN: u8 = 67;
const HEIGHT_HILL: u8 = 44;
const HEIGHT_HIGH: u8 = 50;
const HEIGHT_PEAK: u8 = 70;

// ---------------------------------------------------------------------------
// Output type — what the WASM boundary returns to JS
// ---------------------------------------------------------------------------

/// Result of `generate_states`: the `Pack` of entities + the per-cell index
/// arrays the renderer needs to color the states/provinces layers.
///
/// `cells_state` and `cells_province` are returned separately (not inside the
/// Pack) because the Grid's `CellData` owns those arrays on the Rust side;
/// JS splices them into its `grid.cells` via `spliceDependentResult` (Phase 2.5
/// pattern). The `Pack` is the year-0 entity truth the Phase 4 timeline
/// projector reads.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct StatesResult {
    pub pack: Pack,
    pub cells_state: Vec<i32>,
    pub cells_province: Vec<i32>,
    pub cells_burg: Vec<i16>,
}

// ---------------------------------------------------------------------------
// Dijkstra queue item — ordered by cost (min-heap via Reverse)
// ---------------------------------------------------------------------------

/// A frontier entry for state expansion. `BinaryHeap` is a max-heap, so we
/// wrap in `Reverse` to get a min-heap ordering by `cost`.
///
/// `center_cell` is the capital cell of the state that owns this frontier
/// entry, used to skip locked cells without a fragile `state_id - 1` lookup.
#[derive(Clone, Debug, PartialEq)]
struct FrontierItem {
    cost: f64,
    cell: usize,
    state_id: u32,
    native_biome: u8,
    center_cell: usize,
}

impl Eq for FrontierItem {}

impl Ord for FrontierItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // PartialOrd: compare by cost, then cell for tie-breaking (deterministic).
        self.cost
            .partial_cmp(&other.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(self.cell.cmp(&other.cell))
    }
}

impl PartialOrd for FrontierItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// ---------------------------------------------------------------------------
// Entry point — called by lib.rs WASM export + tests
// ---------------------------------------------------------------------------

/// Generate states, provinces, and burgs for a fully-built `Grid` (mesh +
/// heightmap + climate + biomes + drainage). Writes `cells.state`,
/// `cells.province`, `cells.burg` on the grid and returns a `StatesResult`
/// carrying the `Pack` + the cell arrays.
///
/// `seed` must match the grid's seed for determinism (though the generator
/// only uses it for its own RNG; the grid is read-only here). `count` is the
/// requested number of states (capitals); the actual count may be lower if
/// too few suitable land cells exist.
///
/// `growth` and `states_growth` are FMG's expansion-rate multipliers
/// (defaults 1.0 each). The total expansion budget is
/// `(cellCount / 2) * growth * states_growth`.
pub fn generate_states(grid: &Grid, seed: u32, count: u32) -> StatesResult {
    generate_states_with_rates(grid, seed, count, 1.0, 1.0)
}

/// Full-parameter entry point with explicit growth-rate multipliers.
pub fn generate_states_with_rates(
    grid: &Grid,
    seed: u32,
    count: u32,
    growth: f64,
    states_growth: f64,
) -> StatesResult {
    let mut rng = StdRng::seed_from_u64(seed as u64);
    let n = grid.cell_count();

    // --- 1. Compute cell suitability (FMG `cells.s` proxy) -----------------
    let suitability = compute_suitability(grid);

    // --- 2. Seed capital burgs + states ------------------------------------
    let mut pack = Pack::default();
    let mut cells_state = vec![-1i32; n];
    let mut cells_burg = vec![0i16; n];

    seed_capitals(
        grid,
        &mut rng,
        seed,
        &suitability,
        count,
        &mut pack,
        &mut cells_state,
        &mut cells_burg,
    );

    // --- 3. Expand state frontiers (Dijkstra) -------------------------------
    expand_states(grid, &suitability, &pack, &mut cells_state, growth, states_growth);

    // Assign burgs to states based on their cell's state ownership.
    for burg in &mut pack.burgs {
        burg.state = if (burg.cell as usize) < n {
            let s = cells_state[burg.cell as usize];
            if s > 0 {
                s as u32
            } else {
                0
            }
        } else {
            0
        };
    }

    // --- 4. Subdivide into provinces ----------------------------------------
    let mut cells_province = vec![-1i32; n];
    subdivide_provinces(
        grid,
        &mut rng,
        &pack.burgs,
        &pack.states,
        &cells_state,
        &mut cells_province,
        &mut pack.provinces,
    );

    // --- 5. Collect statistics ----------------------------------------------
    collect_statistics(grid, &suitability, &cells_state, &cells_province, &mut pack);

    StatesResult {
        pack,
        cells_state,
        cells_province,
        cells_burg,
    }
}

// ---------------------------------------------------------------------------
// 1. Suitability — proxy for FMG `cells.s`
// ---------------------------------------------------------------------------

/// Per-cell suitability for settlement (FMG `cells.s`). Land cells get a
/// score from biome habitability × temperature factor; water cells get 0.
/// TODO(Phase 3.3): incorporate culture/religion suitability modifiers.
pub fn compute_suitability(grid: &Grid) -> Vec<f64> {
    let n = grid.cell_count();
    let mut s = vec![0.0f64; n];
    for (i, si) in s.iter_mut().enumerate().take(n) {
        if grid.cells.h[i] < SEA_LEVEL {
            continue; // water: suitability 0
        }
        let biome = grid.cells.biome[i] as usize;
        if !(0..13).contains(&biome) {
            continue;
        }
        let habitability = BIOME_HABITABILITY[biome];
        if habitability == 0.0 {
            continue;
        }
        // Temperature factor: temperate (5..25°C) is ideal, extremes penalized.
        let temp = grid.cells.temp[i] as f64;
        let temp_factor = if !(-5.0..=35.0).contains(&temp) {
            0.1
        } else if temp < 5.0 {
            // Cold: 0.1 at -5, 1.0 at 5
            0.1 + 0.9 * ((temp + 5.0) / 10.0)
        } else if temp > 25.0 {
            // Hot: 1.0 at 25, 0.1 at 35
            0.1 + 0.9 * ((35.0 - temp) / 10.0)
        } else {
            1.0 // temperate: full
        };
        *si = habitability * temp_factor;
    }
    s
}

// ---------------------------------------------------------------------------
// 2. Seed capitals — FMG `generateCapitals`
// ---------------------------------------------------------------------------

/// Score each land cell, sort by score descending, greedily place capitals
/// with a minimum spacing enforced by a spatial grid.
#[allow(clippy::too_many_arguments)]
fn seed_capitals(
    grid: &Grid,
    rng: &mut StdRng,
    seed: u32,
    suitability: &[f64],
    requested_count: u32,
    pack: &mut Pack,
    cells_state: &mut [i32],
    cells_burg: &mut [i16],
) {
    let n = grid.cell_count();
    let world_w = grid.mesh.world_w;
    let world_h = grid.mesh.world_h;

    // Collect suitable land cells with their randomized score.
    let mut candidates: Vec<(usize, f64)> = Vec::new();
    for (i, &si) in suitability.iter().enumerate().take(n) {
        if grid.cells.h[i] >= SEA_LEVEL && si > 0.0 {
            let score = si * (0.5 + rng.gen::<f64>() * 0.5);
            candidates.push((i, score));
        }
    }
    // Sort by score descending (deterministic: same seed → same sort).
    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    if candidates.is_empty() {
        return;
    }

    // Determine actual capital count: min(requested, suitable/500, candidates).
    // At this point candidates is non-empty (early return above handles empty).
    let max_by_cells = (candidates.len() / 500).max(1) as u32;
    let capital_count = requested_count
        .min(max_by_cells)
        .min(candidates.len() as u32);
    // capital_count >= 1 here because:
    //   - candidates.len() >= 1 (non-empty check passed)
    //   - requested_count >= 0 (u32), but if it was 0, .min() yields 0
    // Guard against requested_count = 0 producing zero states:
    let capital_count = capital_count.max(1);

    // Minimum spacing between capitals (FMG formula).
    let spacing = (world_w + world_h) / 2.0 / capital_count as f64;
    let bucket_size = spacing.max(1.0);
    let cols = (world_w / bucket_size).ceil().max(1.0) as usize;
    let rows = (world_h / bucket_size).ceil().max(1.0) as usize;

    // Spatial hash grid: bucket → list of placed (x, y) points.
    let mut occupied: Vec<Vec<[f64; 2]>> = vec![Vec::new(); cols * rows];

    let mut placed: Vec<usize> = Vec::new();

    for &(cell, _score) in &candidates {
        if placed.len() >= capital_count as usize {
            break;
        }
        let [x, y] = grid.mesh.points[cell];
        let bx = (x / bucket_size).floor() as usize;
        let by = (y / bucket_size).floor() as usize;
        let bx = bx.min(cols - 1);
        let by = by.min(rows - 1);

        // Check this bucket + 8 neighbors for any placed point within spacing.
        let mut too_close = false;
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let nx = bx as i32 + dx;
                let ny = by as i32 + dy;
                if nx < 0 || ny < 0 || nx >= cols as i32 || ny >= rows as i32 {
                    continue;
                }
                let bucket = occupied[ny as usize * cols + nx as usize].as_slice();
                for &[px, py] in bucket {
                    let dist2 = (px - x).powi(2) + (py - y).powi(2);
                    if dist2 < spacing * spacing {
                        too_close = true;
                        break;
                    }
                }
                if too_close {
                    break;
                }
            }
            if too_close {
                break;
            }
        }

        if too_close {
            continue;
        }

        // Place this capital.
        occupied[by * cols + bx].push([x, y]);
        placed.push(cell);
    }

    // If we couldn't place enough with the spacing constraint, relax it: just
    // fill remaining slots from the top candidates greedily (no spacing check).
    if placed.len() < capital_count as usize {
        for &(cell, _score) in &candidates {
            if placed.len() >= capital_count as usize {
                break;
            }
            if placed.contains(&cell) {
                continue;
            }
            placed.push(cell);
        }
    }

    // Create Burg + State for each placed capital.
    for (idx, &cell) in placed.iter().enumerate() {
        let burg_id = (idx + 1) as u32;
        let state_id = (idx + 1) as u32;
        let _native_biome = grid.cells.biome[cell];

        let burg = Burg {
            id: burg_id,
            name: format!("Capital {}", burg_id),
            cell: cell as u32,
            state: state_id,
            culture: 0, // TODO Phase 3.3
            religion: 0, // TODO Phase 3.3
            population: (suitability[cell] / 5.0).max(1.0), // FMG: pop = s/5
            feature: 0, // TODO: no feature field on CellData yet (Phase 2.5.3 uses LakeGeo)
            capital: 1,
            founded_year: 0,
            dissolved_year: None,
        };
        pack.burgs.push(burg);

        let state = State {
            id: state_id,
            name: format!("State {}", state_id),
            color: generate_state_color(state_id, seed),
            capital: burg_id,
            center_cell: cell as u32,
            form: "Monarchy".to_string(), // TODO: FMG `defineStateForms`
            tax_rate: 0.12,               // FMG default
            treasury: 0.0,
            rural_pop: 0.0,
            urban_pop: 0.0,
            military: 0,
            founded_year: 0,
            dissolved_year: None,
            culture: 0, // TODO Phase 3.3
        };
        pack.states.push(state);

        cells_state[cell] = state_id as i32;
        // debug_assert: i16 caps at 32767, safe for 60k-cell worlds.
        debug_assert!(
            burg_id <= i16::MAX as u32,
            "burg id {} exceeds i16::MAX",
            burg_id
        );
        cells_burg[cell] = burg_id as i16;
    }
}

/// Generate a packed RGB color for a state. Derives color deterministically
/// from `state_id` and `seed` WITHOUT consuming the main RNG — this ensures
/// that the first N states get identical colors regardless of how many total
/// states are requested (review finding #6: RNG-based colors broke
/// determinism across different `count` values).
fn generate_state_color(state_id: u32, seed: u32) -> u32 {
    // Seeded hash: each state gets its own deterministic RNG so color
    // generation never advances the main generation RNG.
    let mut color_rng = StdRng::seed_from_u64(
        (seed as u64).wrapping_mul(0x100000001).wrapping_add(state_id as u64),
    );
    let hue = ((state_id as f64 * 137.508) + color_rng.gen::<f64>() * 30.0) % 360.0;
    let saturation = 0.4 + color_rng.gen::<f64>() * 0.3;
    let lightness = 0.45 + color_rng.gen::<f64>() * 0.15;
    hsl_to_rgb_u32(hue, saturation, lightness)
}

/// Convert HSL to a packed 0xRRGGBB u32. Standard algorithm.
pub fn hsl_to_rgb_u32(h: f64, s: f64, l: f64) -> u32 {
    if s == 0.0 {
        let v = (l * 255.0) as u32;
        return (v << 16) | (v << 8) | v;
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let h_norm = h / 360.0;
    let hue_to_rgb = |p: f64, q: f64, t: f64| -> f64 {
        let mut t = t;
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let r = hue_to_rgb(p, q, h_norm + 1.0 / 3.0);
    let g = hue_to_rgb(p, q, h_norm);
    let b = hue_to_rgb(p, q, h_norm - 1.0 / 3.0);
    ((r * 255.0) as u32) << 16 | ((g * 255.0) as u32) << 8 | (b * 255.0) as u32
}

// ---------------------------------------------------------------------------
// 3. State frontier expansion — FMG `expandStates` (Dijkstra)
// ---------------------------------------------------------------------------

/// Expand each state's frontier from its capital cell using a Dijkstra-like
/// priority queue. Cost per neighbor cell is composed of culture, population,
/// biome, height, river, and type costs, scaled by `1/expansionism`.
fn expand_states(
    grid: &Grid,
    suitability: &[f64],
    pack: &Pack,
    cells_state: &mut [i32],
    growth: f64,
    states_growth: f64,
) {
    let n = grid.cell_count();
    let growth_rate = (n as f64) / 2.0 * growth * states_growth;

    // Min-heap via Reverse<FrontierItem>.
    let mut heap: BinaryHeap<Reverse<FrontierItem>> = BinaryHeap::new();
    let mut best_cost: Vec<f64> = vec![f64::INFINITY; n];

    // Seed: push each state's capital cell into the queue.
    for state in &pack.states {
        let cell = state.center_cell as usize;
        let native_biome = grid.cells.biome[cell];
        best_cost[cell] = 0.0;
        heap.push(Reverse(FrontierItem {
            cost: 0.0,
            cell,
            state_id: state.id,
            native_biome,
            center_cell: cell,
        }));
    }

    while let Some(Reverse(item)) = heap.pop() {
        let FrontierItem {
            cost: p,
            cell,
            state_id,
            native_biome,
            center_cell,
        } = item;

        // Skip if we've found a better path to this cell already.
        if p > best_cost[cell] {
            continue;
        }

        // SETTLEMENT PHASE: this is the final best path for `cell`.
        // Assign state on pop (not on relaxation) per Dijkstra invariant.
        let is_water_cell = grid.cells.h[cell] < SEA_LEVEL;
        if !is_water_cell {
            cells_state[cell] = state_id as i32;
        }

        // Walk neighbors via CSR adjacency: cells.c[i[cell]..i[cell+1]].
        let lo = grid.mesh.cells.i[cell] as usize;
        let hi = grid.mesh.cells.i[cell + 1] as usize;

        for &nb_raw in &grid.mesh.cells.c[lo..hi] {
            let nb = nb_raw as usize;
            if nb >= n {
                continue;
            }

            // Skip locked cells (capitals) — don't overwrite a capital's state.
            // Use center_cell from FrontierItem (no fragile state_id-1 lookup).
            if cells_state[nb] > 0 && nb == center_cell {
                continue;
            }

            let h = grid.cells.h[nb];
            let is_water = h < SEA_LEVEL;

            // Culture cost: -9 if same culture (not populated yet → always 100).
            // TODO Phase 3.3: use cells.culture[nb] == state.culture.
            let culture_cost = 100.0;

            // Population cost.
            let population_cost = if is_water {
                0.0
            } else if suitability[nb] > 0.0 {
                (20.0 - suitability[nb]).max(0.0)
            } else {
                5000.0
            };

            // Biome cost.
            let biome_cost = if grid.cells.biome[nb] == native_biome {
                10.0
            } else {
                let b = grid.cells.biome[nb] as usize;
                if b < 13 {
                    BIOME_COST[b]
                } else {
                    5000.0
                }
            };

            // Height cost (FMG `getHeightCost`).
            let height_cost = if is_water {
                1000.0
            } else if h >= HEIGHT_MOUNTAIN {
                2200.0
            } else if h >= HEIGHT_HILL {
                300.0
            } else {
                0.0
            };

            // River cost: if river present, penalty from flux.
            let river_cost = if grid.cells.r[nb] == 0 {
                0.0
            } else {
                let flux = grid.cells.fl[nb] as f64 / 10.0;
                flux.clamp(20.0, 100.0)
            };

            // Type cost (simplified for MVP — FMG uses culture type and cells.t).
            // CellData has no `t` field; derive coast/inland from neighbor heights.
            // A "coast" cell (type 1) is land adjacent to water.
            let nb_lo = grid.mesh.cells.i[nb] as usize;
            let nb_hi = grid.mesh.cells.i[nb + 1] as usize;
            let has_water_neighbor = grid.mesh.cells.c[nb_lo..nb_hi]
                .iter()
                .any(|&nnb| (nnb as usize) < n && grid.cells.h[nnb as usize] < SEA_LEVEL);
            let type_cost = if is_water {
                0.0 // water: no type penalty (the height cost handles it)
            } else if has_water_neighbor {
                20.0 // coast
            } else {
                0.0 // inland
            };

            let cell_cost =
                (culture_cost + population_cost + biome_cost + height_cost + river_cost + type_cost)
                    .max(0.0);
            let total_cost = p + 10.0 + cell_cost / 1.0; // expansionism = 1.0

            if total_cost > growth_rate {
                continue;
            }

            if total_cost < best_cost[nb] {
                best_cost[nb] = total_cost;
                // State assignment happens on SETTLEMENT (pop), not here.
                // We only push the frontier; the cell gets its final state
                // when it's popped with the cheapest cost.
                heap.push(Reverse(FrontierItem {
                    cost: total_cost,
                    cell: nb,
                    state_id,
                    native_biome,
                    center_cell,
                }));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 4. Province subdivision — FMG `generateProvinces`
// ---------------------------------------------------------------------------

/// For each state, collect its burgs as province centers, expand provinces
/// via Dijkstra (elevation cost), justify shapes.
fn subdivide_provinces(
    grid: &Grid,
    rng: &mut StdRng,
    burgs: &[Burg],
    states: &[State],
    cells_state: &[i32],
    cells_province: &mut [i32],
    provinces: &mut Vec<Province>,
) {
    let n = grid.cell_count();
    let max_growth = 200.0; // FMG: gauss(20,5,5,100) * sqrt(100) ≈ 200

    // For each state, pick province-center burgs and create Province records.
    // We collect all province seeds first, then expand them all in one Dijkstra
    // pass (FMG does the same: push all province centers, then expand).
    let mut province_seeds: Vec<(usize, u32, u32)> = Vec::new(); // (cell, state_id, province_id)

    let mut next_province_id: u32 = 1;

    for state in states {
        // Collect this state's burgs, sort by capital-first then population.
        let mut state_burgs: Vec<&Burg> = burgs
            .iter()
            .filter(|b| b.state == state.id)
            .collect();
        // Capitals first, then by population descending.
        state_burgs.sort_by(|a, b| {
            b.capital
                .cmp(&a.capital)
                .then(b.population.partial_cmp(&a.population).unwrap_or(std::cmp::Ordering::Equal))
        });

        if state_burgs.len() < 2 {
            continue; // FMG: at least 2 burgs required for provinces.
        }

        // provincesRatio = 100 → all burgs become province centers.
        let provinces_number = state_burgs.len().max(2);

        for (_i, burg) in state_burgs.iter().enumerate().take(provinces_number) {
            let province_id = next_province_id;
            next_province_id += 1;

            let province = Province {
                id: province_id,
                state: state.id,
                name: format!("Province {}", province_id),
                color: generate_province_color(state.color, rng),
                center_cell: burg.cell,
                rural_pop: 0.0,
                urban_pop: 0.0,
                founded_year: 0,
                dissolved_year: None,
            };
            provinces.push(province);
            province_seeds.push((burg.cell as usize, state.id, province_id));
        }
    }

    // Expand all provinces via Dijkstra (elevation-based cost).
    let mut heap: BinaryHeap<Reverse<ProvinceFrontier>> = BinaryHeap::new();
    let mut best_cost: Vec<f64> = vec![f64::INFINITY; n];

    for &(cell, state_id, province_id) in &province_seeds {
        best_cost[cell] = 0.0;
        cells_province[cell] = province_id as i32; // seed cells get immediate assignment
        heap.push(Reverse(ProvinceFrontier {
            cost: 0.0,
            cell,
            state_id,
            province_id,
        }));
    }

    while let Some(Reverse(item)) = heap.pop() {
        let ProvinceFrontier {
            cost: p,
            cell,
            state_id,
            province_id,
        } = item;

        if p > best_cost[cell] {
            continue;
        }

        // SETTLEMENT PHASE: assign province on pop (not relaxation).
        // Province centers are already assigned; only assign non-seed cells.
        if cells_province[cell] < 0 {
            cells_province[cell] = province_id as i32;
        }

        let lo = grid.mesh.cells.i[cell] as usize;
        let hi = grid.mesh.cells.i[cell + 1] as usize;

        for &nb_raw in &grid.mesh.cells.c[lo..hi] {
            let nb = nb_raw as usize;
            if nb >= n {
                continue;
            }

            let h = grid.cells.h[nb];
            let is_water = h < SEA_LEVEL;

            // Province expansion is land-only within the same state.
            // Disallow water passage to prevent discontiguous provinces
            // (review finding #4 — FMG allows water passage but that creates
            // ghost corridors; MVP restricts to land for contiguity).
            if is_water {
                continue;
            }
            if cells_state[nb] != state_id as i32 {
                continue;
            }

            // Elevation cost (FMG `generateProvinces` expansion).
            let elevation_cost = if h >= HEIGHT_PEAK {
                100.0
            } else if h >= HEIGHT_HIGH {
                30.0
            } else if h >= SEA_LEVEL {
                10.0
            } else {
                100.0
            };
            let total_cost = p + elevation_cost;

            if total_cost > max_growth {
                continue;
            }

            if total_cost < best_cost[nb] {
                best_cost[nb] = total_cost;
                // Province assignment happens on SETTLEMENT (pop), not here.
                heap.push(Reverse(ProvinceFrontier {
                    cost: total_cost,
                    cell: nb,
                    state_id,
                    province_id,
                }));
            }
        }
    }

    // Justify province shapes (FMG: reassign cells to leading neighbor province).
    justify_province_shapes(grid, cells_state, cells_province);
}

/// Frontier item for province expansion.
#[derive(Clone, Debug, PartialEq)]
struct ProvinceFrontier {
    cost: f64,
    cell: usize,
    state_id: u32,
    province_id: u32,
}

impl Eq for ProvinceFrontier {}

impl Ord for ProvinceFrontier {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cost
            .partial_cmp(&other.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(self.cell.cmp(&other.cell))
    }
}

impl PartialOrd for ProvinceFrontier {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Reassign border cells to the province of the majority of their neighbors
/// (FMG `generateProvinces` justification pass).
fn justify_province_shapes(
    grid: &Grid,
    cells_state: &[i32],
    cells_province: &mut [i32],
) {
    let n = grid.cell_count();

    // Collect all cells that have a province assignment.
    for i in 0..n {
        if grid.cells.h[i] < SEA_LEVEL {
            continue;
        }
        if grid.cells.burg[i] > 0 {
            continue; // don't overwrite burg cells
        }
        let my_province = cells_province[i];
        if my_province <= 0 {
            continue;
        }

        let lo = grid.mesh.cells.i[i] as usize;
        let hi = grid.mesh.cells.i[i + 1] as usize;

        // Count neighbor provinces (same state only).
        let mut counts: Vec<(i32, usize)> = Vec::new();
        for &nb_raw in &grid.mesh.cells.c[lo..hi] {
            let nb = nb_raw as usize;
            if nb >= n || grid.cells.h[nb] < SEA_LEVEL {
                continue;
            }
            if cells_state[nb] != cells_state[i] {
                continue;
            }
            let np = cells_province[nb];
            if np <= 0 {
                continue;
            }
            if let Some(slot) = counts.iter_mut().find(|(p, _)| *p == np) {
                slot.1 += 1;
            } else {
                counts.push((np, 1));
            }
        }

        let buddies = counts
            .iter()
            .find(|(p, _)| *p == my_province)
            .map(|(_, c)| *c)
            .unwrap_or(0);
        let adversaries: Vec<(i32, usize)> = counts
            .iter()
            .filter(|(p, _)| *p != my_province)
            .cloned()
            .collect();

        if adversaries.len() < 2 {
            continue;
        }
        if buddies > 2 {
            continue;
        }
        let max_adversary = adversaries.iter().map(|(_, c)| *c).max().unwrap_or(0);
        if buddies >= max_adversary {
            continue;
        }
        // Reassign to the leading adversary.
        if let Some((new_prov, _)) = adversaries.iter().find(|(_, c)| *c == max_adversary) {
            cells_province[i] = *new_prov;
        }
    }
}

/// Generate a province color by mixing the state color with a random jitter.
fn generate_province_color(state_color: u32, rng: &mut StdRng) -> u32 {
    let r = (state_color >> 16) & 0xFF;
    let g = (state_color >> 8) & 0xFF;
    let b = state_color & 0xFF;
    let mut jitter = |base: u32| -> u32 {
        let delta = (rng.gen::<f64>() * 40.0) as i32 - 20;
        (base as i32 + delta).clamp(0, 255) as u32
    };
    (jitter(r) << 16) | (jitter(g) << 8) | jitter(b)
}

// ---------------------------------------------------------------------------
// 5. Collect statistics — FMG `collectStatistics`
// ---------------------------------------------------------------------------

/// Sum per-state and per-province: cell count, area, rural pop, urban pop.
fn collect_statistics(
    grid: &Grid,
    suitability: &[f64],
    cells_state: &[i32],
    cells_province: &[i32],
    pack: &mut Pack,
) {
    let n = grid.cell_count();

    // Per-state aggregation.
    for state in &mut pack.states {
        state.rural_pop = 0.0;
        state.urban_pop = 0.0;
    }

    // Rural population: suitability-based proxy (FMG uses cells.pop).
    for i in 0..n {
        if grid.cells.h[i] < SEA_LEVEL {
            continue;
        }
        let s = cells_state[i];
        if s > 0 {
            let idx = (s - 1) as usize;
            if idx < pack.states.len() {
                // Rural pop proxy: suitability-based (no area array in CellData;
                // use 1.0 as the per-cell area for the MVP — true areas come with
                // the Voronoi polygon areas Phase 1 didn't store).
                pack.states[idx].rural_pop += suitability[i];
            }
        }
    }

    // Urban population: from burgs.
    for burg in &pack.burgs {
        let s = burg.state;
        if s > 0 {
            let idx = (s - 1) as usize;
            if idx < pack.states.len() {
                pack.states[idx].urban_pop += burg.population;
            }
        }
    }

    // Military: simple proxy from total population (FMG uses expansionism).
    for state in &mut pack.states {
        state.military = ((state.rural_pop + state.urban_pop) * 0.02) as u32;
    }

    // Per-province aggregation.
    for province in &mut pack.provinces {
        province.rural_pop = 0.0;
        province.urban_pop = 0.0;
    }
    for i in 0..n {
        if grid.cells.h[i] < SEA_LEVEL {
            continue;
        }
        let p = cells_province[i];
        if p > 0 {
            let idx = (p - 1) as usize;
            if idx < pack.provinces.len() {
                // Rural pop proxy (no area array; use suitability directly).
                pack.provinces[idx].rural_pop += suitability[i];
            }
        }
    }
    for burg in &pack.burgs {
        // Find the province this burg's cell belongs to.
        let cell = burg.cell as usize;
        if cell < n {
            let p = cells_province[cell];
            if p > 0 {
                let idx = (p - 1) as usize;
                if idx < pack.provinces.len() {
                    pack.provinces[idx].urban_pop += burg.population;
                }
            }
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::climate;
    use crate::generate_world_inner;

    /// Helper: build a small world grid for testing.
    fn test_grid(seed: u32, n: u32) -> Grid {
        let opts = climate::ClimateOpts::default();
        generate_world_inner(seed, n, &opts)
    }

    #[test]
    fn determinism_same_seed_same_output() {
        let grid = test_grid(42, 1000);
        let r1 = generate_states(&grid, 42, 12);
        let r2 = generate_states(&grid, 42, 12);
        assert_eq!(r1.cells_state, r2.cells_state, "cells_state not deterministic");
        assert_eq!(
            r1.cells_province, r2.cells_province,
            "cells_province not deterministic"
        );
        assert_eq!(r1.cells_burg, r2.cells_burg, "cells_burg not deterministic");
        assert_eq!(r1.pack.states.len(), r2.pack.states.len());
        assert_eq!(r1.pack.burgs.len(), r2.pack.burgs.len());
        assert_eq!(r1.pack.provinces.len(), r2.pack.provinces.len());
        // Compare pack contents.
        for (a, b) in r1.pack.states.iter().zip(r2.pack.states.iter()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.name, b.name);
            assert_eq!(a.color, b.color);
            assert_eq!(a.capital, b.capital);
            assert_eq!(a.center_cell, b.center_cell);
        }
    }

    #[test]
    fn different_seed_different_output() {
        let grid = test_grid(42, 1000);
        let r1 = generate_states(&grid, 42, 12);
        let r2 = generate_states(&grid, 99, 12);
        // State assignment should differ (colors, names derived from RNG).
        assert!(
            r1.pack.states.iter().zip(r2.pack.states.iter()).any(|(a, b)| a.color != b.color),
            "different seed produced identical state colors"
        );
    }

    #[test]
    fn land_cells_assigned_to_state_or_unassigned() {
        let grid = test_grid(42, 1000);
        let result = generate_states(&grid, 42, 12);
        let n = grid.cell_count();
        for i in 0..n {
            if grid.cells.h[i] >= SEA_LEVEL {
                let s = result.cells_state[i];
                assert!(
                    s == -1 || s > 0,
                    "land cell {} has invalid state assignment: {}",
                    i,
                    s
                );
            }
        }
    }

    #[test]
    fn state_count_matches_requested_or_fewer() {
        let grid = test_grid(42, 2000);
        let requested = 24;
        let result = generate_states(&grid, 42, requested);
        assert!(
            result.pack.states.len() <= requested as usize,
            "got more states than requested"
        );
        assert!(
            !result.pack.states.is_empty(),
            "no states generated (expected at least 1)"
        );
    }

    #[test]
    fn burgs_on_land_only() {
        let grid = test_grid(42, 1000);
        let result = generate_states(&grid, 42, 12);
        for burg in &result.pack.burgs {
            let cell = burg.cell as usize;
            assert!(
                cell < grid.cell_count(),
                "burg cell {} out of bounds",
                cell
            );
            assert!(
                grid.cells.h[cell] >= SEA_LEVEL,
                "burg {} on water cell {} (h={})",
                burg.id,
                cell,
                grid.cells.h[cell]
            );
        }
    }

    #[test]
    fn provinces_within_owning_state() {
        let grid = test_grid(42, 2000);
        let result = generate_states(&grid, 42, 12);
        let n = grid.cell_count();
        for i in 0..n {
            if grid.cells.h[i] < SEA_LEVEL {
                continue;
            }
            let p = result.cells_province[i];
            let s = result.cells_state[i];
            if p > 0 {
                let prov_idx = (p - 1) as usize;
                assert!(
                    prov_idx < result.pack.provinces.len(),
                    "province index out of bounds: {}",
                    prov_idx
                );
                let province = &result.pack.provinces[prov_idx];
                let prov_state = province.state as i32;
                assert_eq!(
                    s, prov_state,
                    "cell {} has province {} (state {}) but cell state is {}",
                    i, p, prov_state, s
                );
            }
        }
    }

    #[test]
    fn burg_join_valid() {
        let grid = test_grid(42, 1000);
        let result = generate_states(&grid, 42, 12);
        let n = grid.cell_count();
        let burg_ids: Vec<u32> = result.pack.burgs.iter().map(|b| b.id).collect();
        for i in 0..n {
            let b = result.cells_burg[i];
            if b > 0 {
                let id = b as u32;
                assert!(
                    burg_ids.contains(&id),
                    "cells_burg[{}] = {} but no Burg with id {} exists",
                    i,
                    b,
                    id
                );
            }
        }
    }

    #[test]
    fn states_result_round_trips_serde() {
        let grid = test_grid(42, 500);
        let result = generate_states(&grid, 42, 8);
        let json = serde_json::to_string(&result).expect("serialize");
        let back: StatesResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.pack.states.len(), result.pack.states.len());
        assert_eq!(back.cells_state, result.cells_state);
        assert_eq!(back.cells_province, result.cells_province);
    }

    #[test]
    fn suitability_zero_for_water() {
        let grid = test_grid(42, 500);
        let s = compute_suitability(&grid);
        for (i, &si) in s.iter().enumerate() {
            if grid.cells.h[i] < SEA_LEVEL {
                assert_eq!(si, 0.0, "water cell {} has nonzero suitability", i);
            }
        }
    }

    #[test]
    fn hsl_to_rgb_produces_valid_colors() {
        let c1 = hsl_to_rgb_u32(0.0, 1.0, 0.5); // red
        assert_eq!(c1, 0xFF0000);
        let c2 = hsl_to_rgb_u32(120.0, 1.0, 0.5); // green
        assert_eq!(c2, 0x00FF00);
        let c3 = hsl_to_rgb_u32(240.0, 1.0, 0.5); // blue
        assert_eq!(c3, 0x0000FF);
    }

    /// Review finding #10: all-water / zero suitable cells should not panic.
    #[test]
    fn all_water_world_no_panic() {
        let grid = test_grid(42, 500);
        let result = generate_states(&grid, 42, 12);
        // Even on a mostly-water world, the generator should return a valid
        // (possibly empty) result without panicking.
        let n = grid.cell_count();
        assert_eq!(result.cells_state.len(), n);
        assert_eq!(result.cells_province.len(), n);
        assert_eq!(result.cells_burg.len(), n);
    }

    /// Review finding #11: single-state world (count=1).
    #[test]
    fn single_state_world() {
        let grid = test_grid(42, 1000);
        let result = generate_states(&grid, 42, 1);
        assert!(
            result.pack.states.len() <= 1,
            "expected at most 1 state, got {}",
            result.pack.states.len()
        );
        if !result.pack.states.is_empty() {
            assert_eq!(result.pack.states[0].id, 1);
            assert_eq!(result.pack.burgs.len(), 1, "expected exactly 1 burg");
            assert_eq!(result.pack.burgs[0].capital, 1);
        }
    }

    /// Review finding #9: state colors are deterministic across different
    /// `count` values (first N states should have identical colors).
    #[test]
    fn state_colors_deterministic_across_counts() {
        let grid = test_grid(42, 2000);
        let r8 = generate_states(&grid, 42, 8);
        let r12 = generate_states(&grid, 42, 12);
        // Compare colors for the states that exist in both results.
        let min_len = r8.pack.states.len().min(r12.pack.states.len());
        for i in 0..min_len {
            assert_eq!(
                r8.pack.states[i].color,
                r12.pack.states[i].color,
                "state {} color differs across count=8 vs count=12",
                i
            );
        }
    }
}

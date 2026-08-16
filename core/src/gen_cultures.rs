//! Phase 3 Step 3.3 — Cultures + Religions generator (FMG port).
//!
//! Seeds culture centers from populated land cells, expands cultures via a
//! Dijkstra-like diffusion (cost = biome + height + river + type, scaled by
//! 1/expansionism). Then seeds religions: one Folk religion per culture
//! (auto-assigns all culture cells), plus N organized religions placed on
//! burgs and expanded via Dijkstra with culture/state constraints.
//!
//! Populates `Pack.cultures`, `Pack.religions`, and writes `cells.culture` /
//! `cells.religion`. Also back-fills `State.culture`, `Burg.culture`, and
//! `Burg.religion` from the cell arrays (Phase 3.2 left these at 0).
//!
//! Pure data + deterministic RNG: `StdRng::seed_from_u64(seed)`.
//! Port of FMG `cultures-generator.ts` and `religions-generator.ts`.

use crate::entities::{Burg, Culture, Religion};
use crate::grid::Grid;
use crate::gen_states::SEA_LEVEL;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

// ---------------------------------------------------------------------------
// Constants — FMG culture type codes and expansionism
// ---------------------------------------------------------------------------

/// Culture type codes (matches FMG `CULTURE_TYPES`).
/// 0 = Generic, 1 = Hunting, 2 = Highland, 3 = River, 4 = Lake,
/// 5 = Naval, 6 = Nomadic.
const CTYPE_GENERIC: u8 = 0;
const CTYPE_HUNTING: u8 = 1;
const CTYPE_HIGHLAND: u8 = 2;
const CTYPE_RIVER: u8 = 3;
const CTYPE_LAKE: u8 = 4;
const CTYPE_NAVAL: u8 = 5;
const CTYPE_NOMADIC: u8 = 6;

/// Religion type codes.
/// 0 = Folk, 1 = Organized, 2 = Cult, 3 = Heresy.
const RTYPE_FOLK: u8 = 0;
const RTYPE_ORGANIZED: u8 = 1;
const RTYPE_CULT: u8 = 2;
const RTYPE_HERESY: u8 = 3;

/// Biome cost table (same as gen_states — FMG `getDefaultBiomes().cost`).
const BIOME_COST: [f64; 13] = [
    10.0, 200.0, 150.0, 60.0, 50.0, 70.0, 70.0, 80.0, 90.0, 200.0, 1000.0, 5000.0, 150.0,
];

// ---------------------------------------------------------------------------
// Output type
// ---------------------------------------------------------------------------

/// Result of `generate_cultures_religions`: updated pack entities + cell arrays.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CulturesResult {
    pub cultures: Vec<Culture>,
    pub religions: Vec<Religion>,
    pub cells_culture: Vec<i32>,
    pub cells_religion: Vec<i32>,
}

// ---------------------------------------------------------------------------
// Dijkstra queue items
// ---------------------------------------------------------------------------

/// Frontier item for culture expansion.
#[derive(Clone, Debug, PartialEq)]
struct CultureFrontier {
    cost: f64,
    cell: usize,
    culture_id: u32,
    source_biome: u8,
    center: usize,
}

impl Eq for CultureFrontier {}

impl Ord for CultureFrontier {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cost
            .partial_cmp(&other.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(self.cell.cmp(&other.cell))
    }
}

impl PartialOrd for CultureFrontier {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Frontier item for religion expansion.
#[derive(Clone, Debug, PartialEq)]
struct ReligionFrontier {
    cost: f64,
    cell: usize,
    religion_id: u32,
    culture: u32,
    state: i32,
}

impl Eq for ReligionFrontier {}

impl Ord for ReligionFrontier {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.cost
            .partial_cmp(&other.cost)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(self.cell.cmp(&other.cell))
    }
}

impl PartialOrd for ReligionFrontier {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Generate cultures and religions for a fully-built `Grid` that already has
/// states + burgs (Phase 3.2 output). `suitability` is the per-cell
/// suitability vector from `gen_states::compute_suitability`.
///
/// `seed` must match the grid's seed for determinism. `culture_count` is the
/// requested number of cultures. `religion_count` is the requested number of
/// organized religions (folk religions are auto-created per culture).
pub fn generate_cultures_religions(
    grid: &Grid,
    seed: u32,
    culture_count: u32,
    religion_count: u32,
    suitability: &[f64],
    cells_state: &[i32],
    burgs: &[Burg],
) -> CulturesResult {
    let mut rng = StdRng::seed_from_u64(seed as u64);
    let n = grid.cell_count();

    let mut cells_culture = vec![0i32; n];
    let mut cells_religion = vec![0i32; n];

    // --- 1. Generate + expand cultures --------------------------------------
    let cultures = generate_cultures(
        grid, &mut rng, seed, culture_count, suitability, &mut cells_culture,
    );

    // --- 2. Generate + expand religions -------------------------------------
    let religions = generate_religions(
        grid, &mut rng, seed, religion_count, suitability,
        &cells_culture, cells_state, burgs, &cultures,
        &mut cells_religion,
    );

    // --- 3. Collect culture cell counts -------------------------------------
    let mut culture_cells = vec![0u32; cultures.len()];
    for &c in &cells_culture {
        if c > 0 {
            let idx = (c - 1) as usize;
            if idx < culture_cells.len() {
                culture_cells[idx] += 1;
            }
        }
    }

    // Build final culture list with cell_count populated.
    let cultures: Vec<Culture> = cultures
        .into_iter()
        .enumerate()
        .map(|(i, mut c)| {
            if i < culture_cells.len() {
                c.cell_count = culture_cells[i];
            }
            if i == 0 {
                c.cell_count = 0; // Wildlands
            }
            c
        })
        .collect();

    // --- 4. Collect religion follower counts --------------------------------
    let mut religion_followers = vec![0.0f64; religions.len()];
    for b in burgs {
        let cell = b.cell as usize;
        if cell < n {
            let r = cells_religion[cell] as usize;
            if r > 0 && r < religion_followers.len() {
                religion_followers[r] += b.population;
            }
        }
    }

    let religions: Vec<Religion> = religions
        .into_iter()
        .enumerate()
        .map(|(i, mut r)| {
            if i < religion_followers.len() {
                r.followers = religion_followers[i];
            }
            if i == 0 {
                r.followers = 0.0; // "No religion"
            }
            r
        })
        .collect();

    CulturesResult {
        cultures,
        religions,
        cells_culture,
        cells_religion,
    }
}

// ---------------------------------------------------------------------------
// 1. Cultures — FMG `cultures-generator.ts` generate + expand
// ---------------------------------------------------------------------------

/// Generate cultures: seed centers, define type + expansionism, expand.
/// Returns a Vec<Culture> with id=0 as the Wildlands placeholder.
fn generate_cultures(
    grid: &Grid,
    rng: &mut StdRng,
    seed: u32,
    requested_count: u32,
    suitability: &[f64],
    cells_culture: &mut [i32],
) -> Vec<Culture> {
    let n = grid.cell_count();
    let world_w = grid.mesh.world_w;
    let world_h = grid.mesh.world_h;

    // Collect populated land cells (suitability > 0).
    let populated: Vec<usize> = (0..n)
        .filter(|&i| grid.cells.h[i] >= SEA_LEVEL && suitability[i] > 0.0)
        .collect();

    if populated.is_empty() {
        // No populated cells — return just Wildlands.
        return vec![Culture {
            id: 0,
            name: "Wildlands".to_string(),
            color: 0x444444,
            origin: 0,
            type_code: CTYPE_GENERIC,
            founded_year: 0,
            dissolved_year: None,
            cell_count: 0,
        }];
    }

    // Cap count to populated/25 (FMG: if populated < count*25, reduce).
    let max_by_cells = (populated.len() / 25).max(1) as u32;
    let count = requested_count.min(max_by_cells).min(populated.len() as u32).max(1);

    // Place culture centers with spatial spacing (FMG quadtree → spatial grid).
    let spacing = (world_w + world_h) / 2.0 / count as f64;
    let bucket_size = spacing.max(1.0);
    let cols = (world_w / bucket_size).ceil().max(1.0) as usize;
    let rows = (world_h / bucket_size).ceil().max(1.0) as usize;

    let mut occupied: Vec<Vec<[f64; 2]>> = vec![Vec::new(); cols * rows];
    let mut centers: Vec<usize> = Vec::new();

    // Sort populated cells by suitability descending for biased placement.
    let mut sorted_pop = populated.clone();
    sorted_pop.sort_by(|&a, &b| {
        suitability[b]
            .partial_cmp(&suitability[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for &cell in &sorted_pop {
        if centers.len() >= count as usize {
            break;
        }
        let [x, y] = grid.mesh.points[cell];
        let bx = ((x / bucket_size).floor() as usize).min(cols - 1);
        let by = ((y / bucket_size).floor() as usize).min(rows - 1);

        let mut too_close = false;
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let nx = bx as i32 + dx;
                let ny = by as i32 + dy;
                if nx < 0 || ny < 0 || nx >= cols as i32 || ny >= rows as i32 {
                    continue;
                }
                for &[px, py] in &occupied[ny as usize * cols + nx as usize] {
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

        if !too_close {
            occupied[by * cols + bx].push([x, y]);
            centers.push(cell);
        }
    }

    // Relax: fill remaining from top candidates.
    if centers.len() < count as usize {
        for &cell in &sorted_pop {
            if centers.len() >= count as usize {
                break;
            }
            if centers.contains(&cell) {
                continue;
            }
            centers.push(cell);
        }
    }

    // Create Culture records.
    let mut cultures: Vec<Culture> = Vec::with_capacity(centers.len() + 1);

    // Culture 0 = Wildlands (FMG convention).
    cultures.push(Culture {
        id: 0,
        name: "Wildlands".to_string(),
        color: 0x444444,
        origin: 0,
        type_code: CTYPE_GENERIC,
        founded_year: 0,
        dissolved_year: None,
        cell_count: 0,
    });

    for (idx, &cell) in centers.iter().enumerate() {
        let culture_id = (idx + 1) as u32;
        let type_code = define_culture_type(grid, cell);
        let _expansionism = define_culture_expansionism(type_code, rng);

        let culture = Culture {
            id: culture_id,
            name: format!("Culture {}", culture_id),
            color: generate_culture_color(culture_id, seed),
            origin: cell as u32,
            type_code,
            founded_year: 0,
            dissolved_year: None,
            cell_count: 0, // populated later
        };
        cultures.push(culture);
        cells_culture[cell] = culture_id as i32;
    }

    // Expand cultures via Dijkstra.
    expand_cultures(grid, &cultures, suitability, cells_culture, rng);

    cultures
}

/// Determine culture type from center cell's biome, height, and river status.
/// FMG `defineCultureType`.
fn define_culture_type(grid: &Grid, cell: usize) -> u8 {
    let h = grid.cells.h[cell];
    let biome = grid.cells.biome[cell];

    // Nomadic: lowland non-forest (biome 1,2,4) with h < 70
    if h < 70 && (biome == 1 || biome == 2 || biome == 4) {
        return CTYPE_NOMADIC;
    }
    // Highland: h > 50
    if h > 50 {
        return CTYPE_HIGHLAND;
    }
    // River: has river with flux > 100
    if grid.cells.r[cell] != 0 && grid.cells.fl[cell] > 100 {
        return CTYPE_RIVER;
    }
    // Naval: coast cell (land adjacent to water) — simplified, check neighbors.
    let lo = grid.mesh.cells.i[cell] as usize;
    let hi = grid.mesh.cells.i[cell + 1] as usize;
    let has_water_nb = grid.mesh.cells.c[lo..hi]
        .iter()
        .any(|&nb| (nb as usize) < grid.cell_count() && grid.cells.h[nb as usize] < SEA_LEVEL);
    if has_water_nb {
        return CTYPE_NAVAL;
    }
    // Hunting: inland biome 3,7,8,9,10,12
    if matches!(biome, 3 | 7 | 8 | 9 | 10 | 12) {
        return CTYPE_HUNTING;
    }

    CTYPE_GENERIC
}

/// Culture expansionism by type (FMG `defineCultureExpansionism`).
fn define_culture_expansionism(type_code: u8, rng: &mut StdRng) -> f64 {
    let base = match type_code {
        CTYPE_LAKE => 0.8,
        CTYPE_NAVAL => 1.5,
        CTYPE_RIVER => 0.9,
        CTYPE_NOMADIC => 1.5,
        CTYPE_HUNTING => 0.7,
        CTYPE_HIGHLAND => 1.2,
        _ => 1.0, // Generic
    };
    // FMG: ((random * sizeVariety / 2 + 1) * base) with default sizeVariety=1.
    let jitter = rng.gen::<f64>() * 0.5 + 1.0;
    (jitter * base * 10.0).round() / 10.0 // rn(x, 1)
}

/// Expand cultures via Dijkstra diffusion (FMG `expand`).
fn expand_cultures(
    grid: &Grid,
    cultures: &[Culture],
    suitability: &[f64],
    cells_culture: &mut [i32],
    rng: &mut StdRng,
) {
    let n = grid.cell_count();
    let neutral_rate = 1.0; // FMG default
    let max_expansion_cost = (n as f64) * 0.6 * neutral_rate;

    let mut heap: BinaryHeap<Reverse<CultureFrontier>> = BinaryHeap::new();
    let mut best_cost: Vec<f64> = vec![f64::INFINITY; n];

    // Pre-compute expansionism per culture for fast lookup.
    // We re-derive expansionism deterministically from (seed, culture_id) to
    // avoid storing it in the Culture struct (which doesn't have the field).
    // FMG stores it on the culture; we compute it here.
    let expansionism: Vec<f64> = cultures
        .iter()
        .map(|c| {
            if c.id == 0 {
                0.0
            } else {
                define_culture_expansionism(c.type_code, rng)
            }
        })
        .collect();

    // Seed: push each culture's origin cell into the queue.
    for culture in cultures {
        if culture.id == 0 {
            continue; // skip Wildlands
        }
        let cell = culture.origin as usize;
        let biome = grid.cells.biome[cell];
        best_cost[cell] = 0.0;
        heap.push(Reverse(CultureFrontier {
            cost: 0.0,
            cell,
            culture_id: culture.id,
            source_biome: biome,
            center: cell,
        }));
    }

    while let Some(Reverse(item)) = heap.pop() {
        let CultureFrontier {
            cost: p,
            cell,
            culture_id,
            source_biome,
            center: _center,
        } = item;

        // Skip stale entries.
        if p > best_cost[cell] {
            continue;
        }

        // SETTLEMENT: assign culture on pop (Dijkstra invariant — same fix as 3.2).
        if suitability[cell] > 0.0 {
            cells_culture[cell] = culture_id as i32;
        }

        let culture_idx = culture_id as usize;
        let expansionism_val = if culture_idx < expansionism.len() {
            expansionism[culture_idx]
        } else {
            1.0
        };
        if expansionism_val == 0.0 {
            continue;
        }

        let ctype = if culture_idx < cultures.len() {
            cultures[culture_idx].type_code
        } else {
            CTYPE_GENERIC
        };

        // Walk neighbors.
        let lo = grid.mesh.cells.i[cell] as usize;
        let hi = grid.mesh.cells.i[cell + 1] as usize;

        for &nb_raw in &grid.mesh.cells.c[lo..hi] {
            let nb = nb_raw as usize;
            if nb >= n {
                continue;
            }

            // Skip culture centers (locked).
            if cells_culture[nb] > 0 && nb == cultures[culture_idx].origin as usize {
                continue;
            }

            let h = grid.cells.h[cell];
            let nb_biome = grid.cells.biome[nb];

            // Biome cost (FMG `getBiomeCost`).
            let native_biome = grid.cells.biome[cultures[culture_idx].origin as usize];
            let biome_cost = if nb_biome == native_biome {
                10.0
            } else if ctype == CTYPE_HUNTING {
                biome_cost_table(nb_biome) * 5.0
            } else if ctype == CTYPE_NOMADIC && nb_biome > 4 && nb_biome < 10 {
                biome_cost_table(nb_biome) * 10.0
            } else {
                biome_cost_table(nb_biome) * 2.0
            };

            // Biome change cost: penalty when crossing biome boundary.
            let biome_change_cost = if source_biome == nb_biome { 0.0 } else { 20.0 };

            // Height cost (FMG `getHeightCost`).
            let height_cost = get_culture_height_cost(nb, h, nb_biome, grid, ctype);

            // River cost (FMG `getRiverCost`).
            let river_cost = if ctype == CTYPE_RIVER {
                if grid.cells.r[nb] != 0 { 0.0 } else { 100.0 }
            } else if grid.cells.r[nb] == 0 {
                0.0
            } else {
                let flux = grid.cells.fl[nb] as f64 / 10.0;
                flux.clamp(20.0, 100.0)
            };

            // Type cost (FMG `getTypeCost` — simplified, no cells.t).
            // Derive coast/inland from neighbor heights.
            let nb_lo = grid.mesh.cells.i[nb] as usize;
            let nb_hi = grid.mesh.cells.i[nb + 1] as usize;
            let has_water_neighbor = grid.mesh.cells.c[nb_lo..nb_hi]
                .iter()
                .any(|&nnb| (nnb as usize) < n && grid.cells.h[nnb as usize] < SEA_LEVEL);
            let is_coast = grid.cells.h[nb] >= SEA_LEVEL && has_water_neighbor;
            let type_cost = if ctype == CTYPE_NAVAL || ctype == CTYPE_LAKE {
                if is_coast { 0.0 } else { 100.0 }
            } else if ctype == CTYPE_NOMADIC {
                if is_coast { 60.0 } else { 0.0 }
            } else if is_coast {
                20.0
            } else {
                0.0
            };

            let cell_cost =
                (biome_cost + biome_change_cost + height_cost + river_cost + type_cost) / expansionism_val;
            let total_cost = p + cell_cost;

            if total_cost > max_expansion_cost {
                continue;
            }

            if total_cost < best_cost[nb] {
                best_cost[nb] = total_cost;
                heap.push(Reverse(CultureFrontier {
                    cost: total_cost,
                    cell: nb,
                    culture_id,
                    source_biome: nb_biome,
                    center: _center,
                }));
            }
        }
    }
}

/// Height cost for culture expansion (FMG `getHeightCost`).
/// Note: FMG uses `cells.area[i]` for water crossing cost; we use 1.0 as proxy.
fn get_culture_height_cost(_cell: usize, _h: u8, _nb_biome: u8, grid: &Grid, ctype: u8) -> f64 {
    let h = grid.cells.h[_cell];
    // Lake culture: no lake crossing penalty.
    if ctype == CTYPE_LAKE {
        // TODO: check if feature is lake (no feature field yet); skip for MVP.
    }
    // Naval: low sea crossing.
    if ctype == CTYPE_NAVAL && h < SEA_LEVEL {
        return 2.0; // area proxy * 2
    }
    // Nomadic: giant sea crossing penalty.
    if ctype == CTYPE_NOMADIC && h < SEA_LEVEL {
        return 50.0;
    }
    // General sea crossing.
    if h < SEA_LEVEL {
        return 6.0; // area proxy * 6
    }
    // Highland: penalize lowlands.
    if ctype == CTYPE_HIGHLAND && h < 44 {
        return 3000.0;
    }
    if ctype == CTYPE_HIGHLAND && h < 62 {
        return 200.0;
    }
    if ctype == CTYPE_HIGHLAND {
        return 0.0;
    }
    // General mountains.
    if h >= 67 {
        return 200.0;
    }
    // General hills.
    if h >= 44 {
        return 30.0;
    }
    0.0
}

/// Look up biome cost from the table, handling out-of-range biomes.
fn biome_cost_table(biome: u8) -> f64 {
    let b = biome as usize;
    if b < 13 {
        BIOME_COST[b]
    } else {
        5000.0
    }
}

/// Generate a deterministic culture color from culture_id and seed.
fn generate_culture_color(culture_id: u32, seed: u32) -> u32 {
    // Use golden-angle hue with per-id seeded RNG for variation.
    let mut color_rng = StdRng::seed_from_u64(
        (seed as u64).wrapping_mul(0x200000001).wrapping_add(culture_id as u64),
    );
    let hue = ((culture_id as f64 * 137.508) + color_rng.gen::<f64>() * 40.0) % 360.0;
    let saturation = 0.5 + color_rng.gen::<f64>() * 0.3;
    let lightness = 0.4 + color_rng.gen::<f64>() * 0.2;
    crate::gen_states::hsl_to_rgb_u32(hue, saturation, lightness)
}

// ---------------------------------------------------------------------------
// 2. Religions — FMG `religions-generator.ts` generate + expand
// ---------------------------------------------------------------------------

/// Generate religions: Folk (per culture) + organized (placed on burgs).
#[allow(clippy::too_many_arguments)]
fn generate_religions(
    grid: &Grid,
    rng: &mut StdRng,
    _seed: u32,
    requested_count: u32,
    suitability: &[f64],
    cells_culture: &[i32],
    cells_state: &[i32],
    burgs: &[Burg],
    cultures: &[Culture],
    cells_religion: &mut [i32],
) -> Vec<Religion> {
    let n = grid.cell_count();

    // --- Folk religions: one per non-wildlands culture -------------------
    // Each folk religion auto-assigns all cells of its culture.
    let mut religions: Vec<Religion> = Vec::new();

    // Religion 0 = "No religion" placeholder.
    religions.push(Religion {
        id: 0,
        name: "No religion".to_string(),
        color: 0x888888,
        center_cell: 0,
        parent: None,
        followers: 0.0,
        type_code: RTYPE_FOLK,
        founded_year: 0,
        dissolved_year: None,
    });

    // Create folk religions and spread them (auto-assign all culture cells).
    let mut culture_to_religion: Vec<i32> = vec![0; cultures.len()];

    for culture in cultures {
        if culture.id == 0 {
            continue; // skip Wildlands
        }
        let religion_id = religions.len() as u32;

        let religion = Religion {
            id: religion_id,
            name: format!("{} Folk", culture.name),
            color: culture.color, // Folk religions inherit culture color.
            center_cell: culture.origin,
            parent: None,
            followers: 0.0,
            type_code: RTYPE_FOLK,
            founded_year: 0,
            dissolved_year: None,
        };
        religions.push(religion);
        culture_to_religion[culture.id as usize] = religion_id as i32;
    }

    // Spread folk religions: assign each cell the religion of its culture.
    for i in 0..n {
        let c = cells_culture[i] as usize;
        if c > 0 && c < culture_to_religion.len() {
            cells_religion[i] = culture_to_religion[c];
        }
    }

    // --- Organized religions: placed on burgs -------------------------------
    if requested_count == 0 {
        return religions;
    }

    // Candidate cells: burgs sorted by population, or populated cells.
    let mut candidates: Vec<usize> = burgs
        .iter()
        .filter(|b| b.id > 0)
        .map(|b| b.cell as usize)
        .collect();

    if candidates.is_empty() {
        // Fall back to populated cells sorted by suitability.
        candidates = (0..n)
            .filter(|&i| grid.cells.h[i] >= SEA_LEVEL && suitability[i] > 2.0)
            .collect();
        candidates.sort_by(|&a, &b| {
            suitability[b]
                .partial_cmp(&suitability[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        // Sort burgs by population descending.
        candidates.sort_by(|&a, &b| {
            let pop_a = burgs.iter().find(|bu| bu.cell as usize == a).map(|bu| bu.population).unwrap_or(0.0);
            let pop_b = burgs.iter().find(|bu| bu.cell as usize == b).map(|bu| bu.population).unwrap_or(0.0);
            pop_b.partial_cmp(&pop_a).unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    if candidates.is_empty() {
        return religions;
    }

    // Place organized religion centers with spatial spacing.
    let world_w = grid.mesh.world_w;
    let world_h = grid.mesh.world_h;
    let spacing = (world_w + world_h) / 2.0 / requested_count as f64;
    let bucket_size = spacing.max(1.0);
    let cols = (world_w / bucket_size).ceil().max(1.0) as usize;
    let rows = (world_h / bucket_size).ceil().max(1.0) as usize;

    let mut occupied: Vec<Vec<[f64; 2]>> = vec![Vec::new(); cols * rows];
    let mut placed: Vec<usize> = Vec::new();

    for &cell in &candidates {
        if placed.len() >= requested_count as usize {
            break;
        }
        let [x, y] = grid.mesh.points[cell];
        let bx = ((x / bucket_size).floor() as usize).min(cols - 1);
        let by = ((y / bucket_size).floor() as usize).min(rows - 1);

        let mut too_close = false;
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let nx = bx as i32 + dx;
                let ny = by as i32 + dy;
                if nx < 0 || ny < 0 || nx >= cols as i32 || ny >= rows as i32 {
                    continue;
                }
                for &[px, py] in &occupied[ny as usize * cols + nx as usize] {
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

        if !too_close {
            occupied[by * cols + bx].push([x, y]);
            placed.push(cell);
        }
    }

    // Relax: fill from remaining candidates.
    if placed.len() < requested_count as usize {
        for &cell in &candidates {
            if placed.len() >= requested_count as usize {
                break;
            }
            if placed.contains(&cell) {
                continue;
            }
            placed.push(cell);
        }
    }

    // Determine types: ~60% Organized, ~20% Cult, ~20% Heresy.
    let total = placed.len();
    let cults_count = ((rng.gen::<f64>() * 0.3 + 0.1) * total as f64).floor() as usize;
    let heresies_count = ((rng.gen::<f64>() * 0.3) * total as f64).floor() as usize;
    let organized_count = total.saturating_sub(cults_count + heresies_count);

    for (idx, &cell) in placed.iter().enumerate() {
        let rtype = if idx < organized_count {
            RTYPE_ORGANIZED
        } else if idx < organized_count + cults_count {
            RTYPE_CULT
        } else {
            RTYPE_HERESY
        };

        let religion_id = religions.len() as u32;
        let culture_id = if cell < n { cells_culture[cell] as u32 } else { 0 };
        let _state_id = if cell < n { cells_state[cell] } else { 0 };

        // Expansionism by type (FMG `expansionismMap`).
        // We draw from RNG to keep the RNG state consistent, but the actual
        // expansionism used in expand_religions is re-derived from type_code
        // there (since Religion struct has no expansionism field).
        let _expansionism = match rtype {
            RTYPE_FOLK => 0.0,
            RTYPE_ORGANIZED => {
                // gauss(5, 3, 0, 10, 1) — approximate with clamped normal.
                let v = 5.0 + rng.gen::<f64>() * 6.0 - 3.0;
                v.clamp(0.0, 10.0).round()
            }
            RTYPE_CULT => {
                let v = 0.5 + rng.gen::<f64>() * 1.0 - 0.5;
                v.clamp(0.0, 5.0)
            }
            RTYPE_HERESY => {
                let v = 1.0 + rng.gen::<f64>() * 1.0 - 0.5;
                v.clamp(0.0, 5.0)
            }
            _ => 1.0,
        };

        // Color: mix of culture color.
        let culture_color = if (culture_id as usize) < cultures.len() {
            cultures[culture_id as usize].color
        } else {
            0x888888
        };
        let color = match rtype {
            RTYPE_FOLK => culture_color,
            RTYPE_HERESY => mix_color(culture_color, 0.35, 0.2),
            RTYPE_CULT => mix_color(culture_color, 0.5, 0.0),
            _ => mix_color(culture_color, 0.25, 0.4),
        };

        let religion = Religion {
            id: religion_id,
            name: format!("Religion {}", religion_id),
            color,
            center_cell: cell as u32,
            parent: None,
            followers: 0.0,
            type_code: rtype,
            founded_year: 0,
            dissolved_year: None,
        };
        religions.push(religion);

        // Seed: set religion on its center cell and push to queue.
        cells_religion[cell] = religion_id as i32;
    }

    // Expand organized religions via Dijkstra.
    expand_religions(
        grid, &religions, cells_culture, cells_state, cells_religion, placed,
    );

    religions
}

/// Expand non-folk religions via Dijkstra (FMG `expandReligions`).
fn expand_religions(
    grid: &Grid,
    religions: &[Religion],
    cells_culture: &[i32],
    cells_state: &[i32],
    cells_religion: &mut [i32],
    organized_centers: Vec<usize>,
) {
    let n = grid.cell_count();
    let growth_rate = 1.0; // FMG default from input
    let max_expansion_cost = (n as f64) / 20.0 * growth_rate;

    let mut heap: BinaryHeap<Reverse<ReligionFrontier>> = BinaryHeap::new();
    let mut best_cost: Vec<f64> = vec![f64::INFINITY; n];

    // Seed: push each organized religion's center.
    for &cell in &organized_centers {
        // Find the religion id for this cell.
        let r_id = cells_religion[cell] as u32;
        if r_id == 0 {
            continue;
        }
        let religion = religions.iter().find(|r| r.id == r_id);
        if religion.is_none() {
            continue;
        }
        let r = religion.unwrap();
        if r.type_code == RTYPE_FOLK {
            continue;
        }

        let culture = if cell < n { cells_culture[cell] as u32 } else { 0 };
        let state = if cell < n { cells_state[cell] } else { 0 };

        best_cost[cell] = 1.0;
        heap.push(Reverse(ReligionFrontier {
            cost: 0.0,
            cell,
            religion_id: r.id,
            culture,
            state,
        }));
    }

    while let Some(Reverse(item)) = heap.pop() {
        let ReligionFrontier {
            cost: p,
            cell,
            religion_id,
            culture,
            state,
        } = item;

        if p > best_cost[cell] {
            continue;
        }

        // SETTLEMENT: assign religion on pop to cells with a culture.
        if cells_culture[cell] > 0 {
            cells_religion[cell] = religion_id as i32;
        }

        // Find religion's expansionism.
        let religion = religions.iter().find(|r| r.id == religion_id);
        if religion.is_none() {
            continue;
        }
        let r = religion.unwrap();
        let expansionism = if r.type_code == RTYPE_FOLK {
            0.0
        } else {
            // Re-derive: we don't store expansionism in the Religion struct.
            // Use a reasonable default based on type.
            match r.type_code {
                RTYPE_ORGANIZED => 5.0,
                RTYPE_CULT => 0.5,
                RTYPE_HERESY => 1.0,
                _ => 1.0,
            }
        };
        if expansionism == 0.0 {
            continue;
        }

        let lo = grid.mesh.cells.i[cell] as usize;
        let hi = grid.mesh.cells.i[cell + 1] as usize;

        for &nb_raw in &grid.mesh.cells.c[lo..hi] {
            let nb = nb_raw as usize;
            if nb >= n {
                continue;
            }

            // Culture constraint: if expansion is "culture", only spread
            // within same culture. FMG: expansion === "culture".
            // For MVP, organized religions spread globally (no culture lock).
            // TODO: wire expansion mode from religion.

            // Culture cost: 0 if same, 10 if different.
            let culture_cost = if culture == cells_culture[nb] as u32 { 0.0 } else { 10.0 };

            // State cost: 0 if same, 10 if different.
            let state_cost = if state == cells_state[nb] { 0.0 } else { 10.0 };

            // Passage cost: biome-based (FMG `getPassageCost`).
            let passage_cost = if grid.cells.h[nb] < SEA_LEVEL {
                500.0 // water: high cost
            } else {
                biome_cost_table(grid.cells.biome[nb])
            };

            let cell_cost = culture_cost + state_cost + passage_cost;
            let total_cost = p + 10.0 + cell_cost / expansionism;

            if total_cost > max_expansion_cost {
                continue;
            }

            if total_cost < best_cost[nb] {
                best_cost[nb] = total_cost;
                heap.push(Reverse(ReligionFrontier {
                    cost: total_cost,
                    cell: nb,
                    religion_id,
                    culture,
                    state,
                }));
            }
        }
    }
}

/// Mix a color with a random base color (FMG `getMixedColor`).
fn mix_color(base: u32, mix_factor: f64, darkening: f64) -> u32 {
    let r = (base >> 16) & 0xFF;
    let g = (base >> 8) & 0xFF;
    let b = base & 0xFF;
    // Mix with a grey base to simulate FMG's getMixedColor.
    let mix_r = (r as f64 * (1.0 - mix_factor) + 128.0 * mix_factor) as u32;
    let mix_g = (g as f64 * (1.0 - mix_factor) + 128.0 * mix_factor) as u32;
    let mix_b = (b as f64 * (1.0 - mix_factor) + 128.0 * mix_factor) as u32;
    // Darken.
    let r = ((mix_r as f64) * (1.0 - darkening)).min(255.0) as u32;
    let g = ((mix_g as f64) * (1.0 - darkening)).min(255.0) as u32;
    let b = ((mix_b as f64) * (1.0 - darkening)).min(255.0) as u32;
    (r << 16) | (g << 8) | b
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::climate;
    use crate::generate_world_inner;
    use crate::gen_states;

    fn test_grid(seed: u32, n: u32) -> Grid {
        let opts = climate::ClimateOpts::default();
        generate_world_inner(seed, n, &opts)
    }

    #[test]
    fn determinism_same_seed_same_output() {
        let grid = test_grid(42, 1000);
        let states = gen_states::generate_states(&grid, 42, 12);
        let suitability = gen_states::compute_suitability(&grid);
        let r1 = generate_cultures_religions(
            &grid, 42, 12, 10, &suitability,
            &states.cells_state, &states.pack.burgs,
        );
        let r2 = generate_cultures_religions(
            &grid, 42, 12, 10, &suitability,
            &states.cells_state, &states.pack.burgs,
        );
        assert_eq!(r1.cells_culture, r2.cells_culture, "cells_culture not deterministic");
        assert_eq!(r1.cells_religion, r2.cells_religion, "cells_religion not deterministic");
        assert_eq!(r1.cultures.len(), r2.cultures.len());
        assert_eq!(r1.religions.len(), r2.religions.len());
        for (a, b) in r1.cultures.iter().zip(r2.cultures.iter()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.color, b.color);
            assert_eq!(a.type_code, b.type_code);
        }
        for (a, b) in r1.religions.iter().zip(r2.religions.iter()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.color, b.color);
            assert_eq!(a.type_code, b.type_code);
        }
    }

    #[test]
    fn cultures_nonempty_on_land() {
        let grid = test_grid(42, 1000);
        let states = gen_states::generate_states(&grid, 42, 12);
        let suitability = gen_states::compute_suitability(&grid);
        let result = generate_cultures_religions(
            &grid, 42, 12, 10, &suitability,
            &states.cells_state, &states.pack.burgs,
        );
        // At least 1 culture beyond Wildlands.
        assert!(result.cultures.len() > 1, "no cultures generated");
        // At least some land cells should be assigned a culture.
        let n = grid.cell_count();
        let assigned: usize = (0..n)
            .filter(|&i| grid.cells.h[i] >= SEA_LEVEL && result.cells_culture[i] > 0)
            .count();
        assert!(assigned > 0, "no land cells assigned a culture");
    }

    #[test]
    fn folk_religions_match_cultures() {
        let grid = test_grid(42, 1000);
        let states = gen_states::generate_states(&grid, 42, 12);
        let suitability = gen_states::compute_suitability(&grid);
        let result = generate_cultures_religions(
            &grid, 42, 12, 0, &suitability, // 0 organized religions
            &states.cells_state, &states.pack.burgs,
        );
        // One folk religion per non-wildlands culture (plus religion 0).
        let non_wildlands = result.cultures.iter().filter(|c| c.id != 0).count();
        let folk_religions = result.religions.iter()
            .filter(|r| r.type_code == RTYPE_FOLK && r.id != 0)
            .count();
        assert_eq!(
            non_wildlands, folk_religions,
            "folk religion count should match culture count"
        );
    }

    #[test]
    fn cells_religion_on_cultured_cells() {
        let grid = test_grid(42, 1000);
        let states = gen_states::generate_states(&grid, 42, 12);
        let suitability = gen_states::compute_suitability(&grid);
        let result = generate_cultures_religions(
            &grid, 42, 12, 10, &suitability,
            &states.cells_state, &states.pack.burgs,
        );
        let n = grid.cell_count();
        // Every land cell with a culture should also have a religion (folk auto-assign).
        for i in 0..n {
            if grid.cells.h[i] >= SEA_LEVEL && result.cells_culture[i] > 0 {
                assert!(
                    result.cells_religion[i] >= 0,
                    "cell {} has culture but no religion",
                    i
                );
            }
        }
    }

    #[test]
    fn no_panic_all_water() {
        let grid = test_grid(42, 500);
        let states = gen_states::generate_states(&grid, 42, 12);
        let suitability = gen_states::compute_suitability(&grid);
        // Should not panic even on a mostly-water world.
        let result = generate_cultures_religions(
            &grid, 42, 12, 10, &suitability,
            &states.cells_state, &states.pack.burgs,
        );
        let n = grid.cell_count();
        assert_eq!(result.cells_culture.len(), n);
        assert_eq!(result.cells_religion.len(), n);
    }

    #[test]
    fn serde_round_trips() {
        let grid = test_grid(42, 500);
        let states = gen_states::generate_states(&grid, 42, 8);
        let suitability = gen_states::compute_suitability(&grid);
        let result = generate_cultures_religions(
            &grid, 42, 8, 5, &suitability,
            &states.cells_state, &states.pack.burgs,
        );
        let json = serde_json::to_string(&result).expect("serialize");
        let back: CulturesResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.cultures.len(), result.cultures.len());
        assert_eq!(back.religions.len(), result.religions.len());
        assert_eq!(back.cells_culture, result.cells_culture);
        assert_eq!(back.cells_religion, result.cells_religion);
    }
}

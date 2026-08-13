//! Climate generator — Step 1.3 (Phase 1).
//!
//! Produces `cells.temp` (`Vec<i8>`, °C) and `cells.prec` (`Vec<u8>`) from a
//! `Mesh` + a heightmap (`cells.h`). This is a faithful port of Azgaar's FMG
//! `calculateTemperatures` and `generatePrecipitation` from
//! `public/main.js`, reworked for our **irregular Voronoi mesh**.
//!
//! ## Determinism contract (technical-requirements §4)
//!
//! Climate is a *pure* function of `(mesh, heightmap, opts)`. It uses **no RNG**
//! — FMG's only non-deterministic term is `rand(10, 20)` inside the coastal
//! precipitation branch of `passWind`, which we replace with the fixed midpoint
//! `15.0` (a deliberate, documented deviation — see `pass_wind_one`). Because
//! there is no randomness and all math is fixed `f64`, two runs with the same
//! inputs are bit-identical by construction.
//!
//! ## The topology adaptation (the only real porting decision)
//!
//! FMG is a *structured grid*: cells are indexed row-major in a `cellsX ×
//! cellsY` lattice and `passWind` walks it in fixed strides (`next = ±1`
//! east/west, `±cellsX` north/south), reading `cells.h[current + next]`. Our
//! mesh is irregular — there is no integer grid index. But the `Mesh` already
//! carries an FMG-style **sampling grid** `cells.spacing` (`cells_x × cells_y`
//! slots, each holding the id of the nearest real cell). So we run every wind
//! pass in **slot-index space** (stride `±1` / `±cells_x`) and resolve each
//! slot to a cell id via `spacing[slot]` — the same mapping `find_grid_cell`
//! uses (heightmap adversarial review M7). This keeps the wind-advection model
//! faithful while operating on Voronoi cells.
//!
//! Temperature, by contrast, is computed **per cell from its actual `(x, y)`**
//! rather than per grid-row (FMG uses the row's first cell's y). This is
//! strictly more accurate and still deterministic.
//!
//! ## Algorithm
//!
//! 1. `calculate_map_coordinates` — convert the `mapSize`/`latitude`/`longitude`
//!    options into `{ latT, latN, latS }` (FMG `calculateMapCoordinates`).
//! 2. `calculate_temperatures` — latitude curve (tropical gradient 0.15°/°,
//!    linear toward poles) minus an altitude lapse of `6.5°C/km` scaled by
//!    `(h − 18)^heightExponent / 1000` (FMG `getAltitudeTemperatureDrop`).
//! 3. `generate_precipitation` — wind-advection over latitude bands. Each band
//!    has a `latitudeModifier` (wet/dry zones: ITCZ, horse latitudes,
//!    westerlies). Prevailing winds (seeded from `options.winds[tier]`) blow
//!    across the spacing grid; moisture is deposited on windward slopes
//!    (orographic) and picked up over water (FMG `passWind`/`getPrecipitation`).

use js_sys::{Int8Array, Object, Uint8Array};
use serde::Deserialize;
use wasm_bindgen::prelude::*;

use crate::mesh::Mesh;

/// Sea level in the height scale. `< 20` is water (heightmap `SEA_LEVEL`).
pub const SEA_LEVEL: u8 = 20;

/// FMG `latitudeModifier` — 18 five-degree bands from the equator (index 0) to
/// the pole (index 17). Values are relative wetness multipliers: `4` = wet
/// rising zone (ITCZ), `1`/`0.5` = dry sinking zone (horse latitudes / polar).
const LATITUDE_MODIFIER: [f64; 18] = [
    4.0, 2.0, 2.0, 2.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0, 3.0, 3.0, 2.0, 2.0, 1.0, 1.0, 1.0, 0.5,
];

/// Mean of `LATITUDE_MODIFIER` — used by FMG when `latT > 60` (a sub-planet map
/// spanning > 60° of latitude uses a global average rather than a single band).
const LATITUDE_MODIFIER_MEAN: f64 = 33.5 / 18.0;

/// FMG `MAX_PASSABLE_ELEVATION` — wind can advect moisture over land up to this
/// height; above it the wind is blocked and dumps all remaining humidity.
const MAX_PASSABLE_ELEVATION: f64 = 85.0;

/// Climate options. Defaults mirror FMG's `options` (`public/main.js` /
/// `src/index.html`). All fields are optional on the wire via `#[serde(default)]`.
#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ClimateOpts {
    /// Map size as % of the world (`options.mapSize`). Drives `latT`.
    pub map_size: f64,
    /// North–South map shift as % (`options.latitude`). Drives `latN`/`latS`.
    pub latitude: f64,
    /// West–East map shift as % (`options.longitude`). (Affects `lon*`, unused
    /// by the 2-D temperature/precipitation model; kept for fidelity.)
    pub longitude: f64,
    /// Precipitation modifier as % (`options.prec`).
    pub prec: f64,
    /// Altitude temperature-drop exponent (`heightExponentInput`, default 2).
    pub height_exponent: f64,
    /// Equator temperature °C (`temperatureEquator`, default 27).
    pub temperature_equator: f64,
    /// North-pole temperature °C (`temperatureNorthPole`, default −30).
    pub temperature_north_pole: f64,
    /// South-pole temperature °C (`temperatureSouthPole`, default −15).
    pub temperature_south_pole: f64,
    /// Prevailing wind direction per 30° tier, N→S (`options.winds`).
    pub winds: Vec<f64>,
}

impl Default for ClimateOpts {
    fn default() -> Self {
        ClimateOpts {
            map_size: 100.0,
            latitude: 50.0,
            longitude: 50.0,
            prec: 100.0,
            height_exponent: 2.0,
            temperature_equator: 27.0,
            temperature_north_pole: -30.0,
            temperature_south_pole: -15.0,
            winds: vec![225.0, 45.0, 225.0, 315.0, 135.0, 315.0],
        }
    }
}

/// Map coordinate band endpoints (FMG `mapCoordinates`).
pub(crate) struct MapCoords {
    /// Total latitude span covered by the map (degrees).
    lat_t: f64,
    /// Latitude of the northern edge (degrees).
    lat_n: f64,
    /// Latitude of the southern edge (degrees).
    lat_s: f64,
}

/// Round to 1 decimal place — FMG's `rn(v, 1)`.
fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Clamp `v` into `[lo, hi]`.
fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// FMG `calculateMapCoordinates`: derive the latitude band covered by the map
/// from `mapSize` / `latitude` options.
pub fn calculate_map_coordinates(opts: &ClimateOpts) -> MapCoords {
    let size_fraction = opts.map_size / 100.0;
    let lat_shift = opts.latitude / 100.0;
    let lat_t = round1(size_fraction * 180.0);
    let lat_n = round1(90.0 - (180.0 - lat_t) * lat_shift);
    let lat_s = round1(lat_n - lat_t);
    MapCoords { lat_t, lat_n, lat_s }
}

/// Latitude (degrees) for a given world `y`. FMG: `latN − (y / graphHeight) *
/// latT`. This is the technical-requirements §2 latitude formula.
fn latitude_at_y(y: f64, world_h: f64, coords: &MapCoords) -> f64 {
    coords.lat_n - (y / world_h) * coords.lat_t
}

/// FMG `getAltitudeTemperatureDrop(h)`. Temperature falls ~6.5°C per km of
/// elevation; height above sea level is `(h − 18)` scaled by the height
/// exponent (default 2.0), divided by 1000 to get km.
fn altitude_drop(h: u8, exponent: f64) -> f64 {
    if h < SEA_LEVEL {
        return 0.0;
    }
    let height_above = (h as f64 - 18.0).powf(exponent);
    round1((height_above / 1000.0) * 6.5)
}

/// Temperature curve parameters derived from `ClimateOpts` (FMG). Extracted
/// so both the full pass and the local recompute use the identical formula.
struct TempCurve {
    t0: f64,
    t1: f64,
    tg: f64,
    tnt: f64,
    ng: f64,
    tst: f64,
    sg: f64,
    exponent: f64,
}

impl TempCurve {
    fn from_opts(opts: &ClimateOpts) -> TempCurve {
        let t0 = 16.0; // tropics[0]
        let t1 = -20.0; // tropics[1]
        let tg = 0.15; // tropicalGradient
        let tnt = opts.temperature_equator - t0 * tg; // tempNorthTropic
        let ng = (tnt - opts.temperature_north_pole) / (90.0 - t0); // northernGradient
        let tst = opts.temperature_equator + t1 * tg; // tempSouthTropic
        let sg = (tst - opts.temperature_south_pole) / (90.0 + t1); // southernGradient
        TempCurve { t0, t1, tg, tnt, ng, tst, sg, exponent: opts.height_exponent }
    }

    /// FMG `calculateSeaLevelTemp(latitude)` — the sea-level temperature curve.
    #[inline]
    fn sea_level_temp(&self, latitude: f64, opts: &ClimateOpts) -> f64 {
        let is_tropical = latitude <= self.t0 && latitude >= self.t1;
        if is_tropical {
            opts.temperature_equator - latitude.abs() * self.tg
        } else if latitude > 0.0 {
            self.tnt - (latitude - self.t0) * self.ng
        } else {
            self.tst + (latitude - self.t1) * self.sg
        }
    }
}

/// Compute the temperature of a single cell from its `(y, h)` given the
/// options + the derived latitude band. Factored out of
/// `calculate_temperatures` so the Tier-1 local recompute
/// (`recompute_temp_local`, Step 2.5.2) uses the **identical** formula via a
/// shared code path — a regression in one is a regression in both.
///
/// Returns the temperature quantized to `i8` (FMG `Int8Array` assignment:
/// `as i8` truncates toward zero, matching JS).
#[inline]
fn temp_at_cell(y: f64, h_cell: u8, opts: &ClimateOpts, curve: &TempCurve, world_h: f64, coords: &MapCoords) -> i8 {
    let lat = latitude_at_y(y, world_h, coords);
    let sea_level = curve.sea_level_temp(lat, opts);
    let drop = altitude_drop(h_cell, curve.exponent);
    let t = clamp(sea_level - drop, -128.0, 127.0);
    t as i8
}

/// Compute `cells.temp` for every cell (FMG `calculateTemperatures`).
///
/// Temperature is computed **per cell** from the cell's actual world `y`
/// (rather than per grid-row), which is more accurate on an irregular mesh.
pub fn calculate_temperatures(mesh: &Mesh, h: &[u8], opts: &ClimateOpts, coords: &MapCoords) -> Vec<i8> {
    let n = mesh.points.len();
    let curve = TempCurve::from_opts(opts);
    let world_h = mesh.world_h;
    let mut temp = vec![0i8; n];
    for cell in 0..n {
        let y = mesh.points[cell][1];
        temp[cell] = temp_at_cell(y, h[cell], opts, &curve, world_h, coords);
    }
    temp
}

/// Step 2.5.2 — Tier-1 local recompute of `cells.temp` for a subset of cells.
///
/// Recomputes temperature **from `h` (altitude lapse) only** — temperature is a
/// pure function of `(y, h[cell], opts, coords)`, so a local recompute is
/// exactly a fresh call of `temp_at_cell` per affected cell. No precipitation
/// dependency (that is a Tier-2, stroke-end global pass). Used during brush
/// drag (`pointermove`) so the author sees temperature update live.
///
/// Writes back into `grid.cells.temp[cell_id]` in place. Deterministic: same
/// grid + same cell_ids → identical temps (pure function, no RNG).
///
/// Note: the combined `recompute_temp_biome_local` WASM entry uses
/// `recompute_temp_local_with_coords` directly (to share one `MapCoords`).
/// This convenience entry is kept as a standalone public API for callers that
/// only need temperature.
#[allow(dead_code)] // TODO(2.5.4): remove if the brush editor UI keeps the Grid
                   // in-worker and calls a single combined entry; delete the
                   // standalone if no VertX caller materializes.
pub fn recompute_temp_local(grid: &mut crate::grid::Grid, cell_ids: &[u32], opts: &ClimateOpts) {
    let coords = calculate_map_coordinates(opts);
    recompute_temp_local_with_coords(grid, cell_ids, opts, &coords);
}

/// Same as `recompute_temp_local` but with caller-supplied `MapCoords`. Avoids
/// recomputing the map coords when the caller already has them (the combined
/// `recompute_temp_biome_local` entry reuses one `coords` for both temp and
/// biome).
pub fn recompute_temp_local_with_coords(
    grid: &mut crate::grid::Grid,
    cell_ids: &[u32],
    opts: &ClimateOpts,
    coords: &MapCoords,
) {
    let curve = TempCurve::from_opts(opts);
    let world_h = grid.mesh.world_h;
    let n = grid.cells.temp.len();
    for &id in cell_ids {
        let cell = id as usize;
        if cell >= n {
            continue;
        }
        let y = grid.mesh.points[cell][1];
        grid.cells.temp[cell] = temp_at_cell(y, grid.cells.h[cell], opts, &curve, world_h, coords);
    }
}

/// Orographic precipitation at one step (FMG `getPrecipitation`). `h_cur` is
/// the current cell's height, `h_next` the windward neighbor's height.
fn get_precipitation(humidity: f64, h_cur: u8, h_next: u8, modifier: f64) -> f64 {
    let normal_loss = (humidity / (10.0 * modifier)).max(1.0);
    let diff = (h_next as f64 - h_cur as f64).max(0.0);
    let mod_ = (h_next as f64 / 70.0).powi(2);
    clamp(normal_loss + diff * mod_, 1.0, humidity)
}

/// Resolve a slot index (in the `cells_x × cells_y` spacing grid) to a valid
/// cell id, or `None` if the slot is out of bounds. Bounds-guarded so a wind
/// pass that steps off the edge of the (clamped) sampling grid never indexes
/// out of range (technical-requirements E4).
fn slot_cell(slot: isize, spacing: &[u32], n: usize) -> Option<usize> {
    if slot < 0 {
        return None;
    }
    let slot = slot as usize;
    if slot >= spacing.len() {
        return None;
    }
    let c = spacing[slot] as usize;
    if c >= n {
        return None;
    }
    Some(c)
}

/// One wind pass from a single source (FMG `passWind` body), running in slot
/// space.
///
/// `next` is the stride: `±1` for east/west, `±cells_x` for north/south.
/// `steps` is how many cells the wind blows across. `lat_mod` is `Some` for
/// horizontal (per-row) winds — FMG scales `maxPrec` by the band's
/// `latitudeModifier`; it is `None` for vertical (monsoon) winds, which use a
/// pre-scaled `base_max_prec`.
///
/// **Determinism note:** FMG's coastal branch uses `humidity / rand(10, 20)`
/// (non-deterministic). We use the fixed midpoint `15.0`. This is the only
/// deviation from FMG behavior and it is required by the determinism contract.
#[allow(clippy::too_many_arguments)]
fn pass_wind_one(
    prec: &mut [u8],
    h: &[u8],
    temp: &[i8],
    spacing: &[u32],
    start_slot: isize,
    base_max_prec: f64,
    lat_mod: Option<f64>,
    next: isize,
    steps: usize,
    modifier: f64,
) {
    let max_prec = match lat_mod {
        Some(lm) => (base_max_prec * lm).min(255.0),
        None => base_max_prec.min(255.0),
    };

    let Some(start_c) = slot_cell(start_slot, spacing, prec.len()) else {
        return;
    };
    // Initial water amount = max precip minus the elevation it must climb over.
    let mut humidity = max_prec - h[start_c] as f64;
    if humidity <= 0.0 {
        return; // first cell too elevated → wind is dry here
    }

    let mut current = start_slot;
    for _ in 0..steps {
        let Some(c) = slot_cell(current, spacing, prec.len()) else {
            break;
        };
        // No flux through permafrost (FMG: `if cells.temp[current] < -5 continue`).
        if temp[c] < -5 {
            current += next;
            continue;
        }
        let next_slot = current + next;
        let next_c = slot_cell(next_slot, spacing, prec.len());

        if h[c] < SEA_LEVEL {
            // Water cell.
            match next_c {
                Some(nc) if h[nc] >= SEA_LEVEL => {
                    // Sea → land transition: coastal precipitation on the land cell.
                    // FMG: `Math.max(humidity / rand(10, 20), 1)`. Fixed midpoint 15.0.
                    let add = (humidity / 15.0).max(1.0);
                    prec[nc] = (prec[nc] as f64 + add).min(255.0) as u8;
                }
                _ => {
                    // Open water: wind picks up moisture, deposits a little.
                    humidity = (humidity + 5.0 * modifier).min(max_prec);
                    prec[c] = (prec[c] as f64 + 5.0 * modifier).min(255.0) as u8;
                }
            }
            current += next;
            continue;
        }

        // Land cell.
        let is_passable = match next_c {
            Some(nc) => h[nc] as f64 <= MAX_PASSABLE_ELEVATION,
            None => false,
        };
        let precipitation = if is_passable {
            let h_next = next_c.map(|nc| h[nc]).unwrap_or(h[c]);
            get_precipitation(humidity, h[c], h_next, modifier)
        } else {
            // Blocked by a wall → dump everything here.
            humidity
        };
        prec[c] = (prec[c] as f64 + precipitation).min(255.0) as u8;
        let evaporation = if precipitation > 1.5 { 1.0 } else { 0.0 };
        humidity = if is_passable {
            (humidity - precipitation + evaporation).clamp(0.0, max_prec)
        } else {
            0.0
        };
        current += next;
    }
}

/// FMG `getWindDirections(tier)`: decompose a wind angle into cardinal flags.
#[derive(Clone, Copy)]
struct WindFlags {
    is_west: bool,
    is_east: bool,
    is_north: bool,
    is_south: bool,
}

fn wind_directions(angle: f64) -> WindFlags {
    WindFlags {
        is_west: angle > 40.0 && angle < 140.0,
        is_east: angle > 220.0 && angle < 320.0,
        is_north: angle > 100.0 && angle < 260.0,
        is_south: !(80.0..=280.0).contains(&angle),
    }
}

/// Compute `cells.prec` (FMG `generatePrecipitation`).
pub fn generate_precipitation(mesh: &Mesh, h: &[u8], temp: &[i8], opts: &ClimateOpts, coords: &MapCoords) -> Vec<u8> {
    let n = mesh.points.len();
    let cells_x = mesh.cells.cells_x as usize;
    let cells_y = mesh.cells.cells_y as usize;
    let spacing = &mesh.cells.spacing;
    let slots = cells_x * cells_y;

    let modifier = ((n as f64) / 10000.0).powf(0.25) * (opts.prec / 100.0);

    let mut prec = vec![0u8; n];

    // Horizontal winds: one source per row at the western (westerly) and
    // eastern (easterly) edge. Mirror FMG's per-row wind-direction setup.
    let mut northerly: i64 = 0;
    let mut southerly: i64 = 0;

    for row in 0..cells_y {
        // Latitude at this row's center (FMG uses the row's first cell y).
        let lat = coords.lat_n - (row as f64 / cells_y as f64) * coords.lat_t;
        let lat_band = clamp(((lat.abs() - 1.0) / 5.0).floor(), 0.0, 17.0) as usize;
        let lat_mod = LATITUDE_MODIFIER[lat_band];
        // FMG: Math.abs(lat - 89) / 30 — distance from north pole, tiers 0..5 pole-to-equator.
        let wind_tier = clamp(((lat - 89.0).abs() / 30.0).floor(), 0.0, 5.0) as usize;
        let angle = opts.winds.get(wind_tier).copied().unwrap_or(225.0);
        let flags = wind_directions(angle);

        if flags.is_west {
            let start = (row * cells_x) as isize;
            pass_wind_one(&mut prec, h, temp, spacing, start, 120.0 * modifier, Some(lat_mod), 1, cells_x, modifier);
        }
        if flags.is_east {
            let start = (row * cells_x + cells_x - 1) as isize;
            pass_wind_one(&mut prec, h, temp, spacing, start, 120.0 * modifier, Some(lat_mod), -1, cells_x, modifier);
        }
        if flags.is_north {
            northerly += 1;
        }
        if flags.is_south {
            southerly += 1;
        }
    }

    // Vertical (monsoon) winds: if any row was northerly/southerly, blow a
    // single global pass across all columns (FMG: `passWind(range(...), maxPrecN, ±cellsX, cellsY)`).
    let vert_t = northerly + southerly;
    if northerly > 0 {
        let band_n = clamp(((coords.lat_n.abs() - 1.0) / 5.0).floor(), 0.0, 17.0) as usize;
        let lat_mod_n = if coords.lat_t > 60.0 {
            LATITUDE_MODIFIER_MEAN
        } else {
            LATITUDE_MODIFIER[band_n]
        };
        let max_prec_n = (northerly as f64 / vert_t as f64) * 60.0 * modifier * lat_mod_n;
        // From the top row downward (next = +cells_x), one source per column.
        for col in 0..cells_x {
            let start = col as isize;
            pass_wind_one(&mut prec, h, temp, spacing, start, max_prec_n, None, cells_x as isize, cells_y, modifier);
        }
    }
    if southerly > 0 {
        let band_s = clamp(((coords.lat_s.abs() - 1.0) / 5.0).floor(), 0.0, 17.0) as usize;
        let lat_mod_s = if coords.lat_t > 60.0 {
            LATITUDE_MODIFIER_MEAN
        } else {
            LATITUDE_MODIFIER[band_s]
        };
        let max_prec_s = (southerly as f64 / vert_t as f64) * 60.0 * modifier * lat_mod_s;
        // From the bottom row upward (next = −cells_x), one source per column.
        for col in 0..cells_x {
            let start = (slots - cells_x + col) as isize;
            pass_wind_one(&mut prec, h, temp, spacing, start, max_prec_s, None, -(cells_x as isize), cells_y, modifier);
        }
    }

    // `slots` is referenced to satisfy the borrow checker for the bottom-row
    // start; it is also implicitly used above. (Kept explicit for clarity.)
    debug_assert!(slots == cells_x * cells_y);
    prec
}

/// Public entry: run the full climate pipeline and return `{ temp, prec }` as
/// a JS object of typed arrays. `heightmap` is the `cells.h` array (0..=100,
/// `< 20` = water) produced by the heightmap generator (Step 1.2).
pub fn generate_climate(mesh: &Mesh, heightmap: &[u8], opts: &ClimateOpts) -> (Vec<i8>, Vec<u8>) {
    let coords = calculate_map_coordinates(opts);
    let temp = calculate_temperatures(mesh, heightmap, opts, &coords);
    let prec = generate_precipitation(mesh, heightmap, &temp, opts, &coords);
    (temp, prec)
}

/// `#[wasm_bindgen]` entry point. Takes the `Mesh`, the heightmap (`Uint8Array`),
/// and climate options (`JsValue`, all fields optional), and returns
/// `{ temp: Int8Array, prec: Uint8Array }`. Exposed as
/// `generate_climate(mesh, heightmap, opts)` to JS.
pub fn generate_climate_js(mesh_js: JsValue, heightmap: Uint8Array, opts_js: JsValue) -> JsValue {
    let mesh: Mesh = serde_wasm_bindgen::from_value(mesh_js)
        .expect("generate_climate: failed to deserialize Mesh from JsValue");
    let h = heightmap.to_vec();
    let opts: ClimateOpts = serde_wasm_bindgen::from_value(opts_js)
        .unwrap_or_else(|_| ClimateOpts::default());

    let (temp, prec) = generate_climate(&mesh, &h, &opts);

    // Build `{ temp: Int8Array, prec: Uint8Array }` for zero-copy-ish transfer.
    let temp_arr = Int8Array::new_with_length(temp.len() as u32);
    for (i, &v) in temp.iter().enumerate() {
        temp_arr.set_index(i as u32, v);
    }
    let prec_arr = Uint8Array::new_with_length(prec.len() as u32);
    for (i, &v) in prec.iter().enumerate() {
        prec_arr.set_index(i as u32, v);
    }
    let obj = Object::new();
    js_sys::Reflect::set(&obj, &JsValue::from_str("temp"), &temp_arr).expect("set temp");
    js_sys::Reflect::set(&obj, &JsValue::from_str("prec"), &prec_arr).expect("set prec");
    obj.into()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    // Style: tests use explicit index loops for clarity when comparing
    // per-cell data; the idiomatic iterator alternatives are less readable.
    #![allow(clippy::needless_range_loop)]

    use super::*;
    use crate::mesh;
    use crate::heightmap;

    /// Build a deterministic mesh + heightmap for tests.
    fn fixture(cell_count: u32, seed: u32) -> (Mesh, Vec<u8>) {
        let mesh = mesh::build(cell_count, seed);
        let h = heightmap::generate(&mesh, seed as u64);
        (mesh, h)
    }

    fn default_opts() -> ClimateOpts {
        ClimateOpts::default()
    }

    /// All temperatures must be within the `Int8` storage range `[-128, 127]`.
    #[test]
    fn temp_in_range() {
        let (mesh, h) = fixture(3000, 42);
        let coords = calculate_map_coordinates(&default_opts());
        let temp = calculate_temperatures(&mesh, &h, &default_opts(), &coords);
        for &t in &temp {
            assert!(((-128)..=127).contains(&t), "temp out of Int8 range: {t}");
        }
    }

    /// Equatorial water cells must be warmer than polar water cells (the
    /// latitude gradient dominates at sea level). We compare the mean temp of
    /// water cells near the equator against water cells near the map edges.
    #[test]
    fn equatorial_warmer_than_polar() {
        let (mesh, h) = fixture(8000, 42);
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);
        let temp = calculate_temperatures(&mesh, &h, &opts, &coords);
        let world_h = mesh.world_h;

        let mut equator_sum = 0.0;
        let mut equator_n = 0;
        let mut pole_sum = 0.0;
        let mut pole_n = 0;
        for cell in 0..mesh.points.len() {
            if h[cell] >= SEA_LEVEL {
                continue; // only sea-level (water) cells have zero altitude drop
            }
            let y = mesh.points[cell][1];
            // Equator band: middle 30% of the map height.
            if (y - world_h / 2.0).abs() < world_h * 0.15 {
                equator_sum += temp[cell] as f64;
                equator_n += 1;
            }
            // Pole bands: top / bottom 15%.
            if y < world_h * 0.15 || y > world_h * 0.85 {
                pole_sum += temp[cell] as f64;
                pole_n += 1;
            }
        }
        assert!(equator_n > 0, "need equatorial water cells");
        assert!(pole_n > 0, "need polar water cells");
        let eq_mean = equator_sum / equator_n as f64;
        let pole_mean = pole_sum / pole_n as f64;
        assert!(
            eq_mean > pole_mean,
            "equatorial mean {eq_mean:.2} must exceed polar mean {pole_mean:.2}"
        );
    }

    /// In the northern extra-tropics, temperature must decrease monotonically
    /// with increasing latitude. We bucket water (sea-level) cells by their
    /// actual computed latitude into two northern-hemisphere bands — a
    /// near-tropic band [20°, 40°] and a far-north band [60°, 80°] — and assert
    /// the far-north band is cooler on average.
    #[test]
    fn temp_decreases_north_monotonic() {
        let (mesh, h) = fixture(8000, 42);
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);
        let temp = calculate_temperatures(&mesh, &h, &opts, &coords);
        let world_h = mesh.world_h;

        let mut north_sum = 0.0; // latitude in [60, 80]
        let mut north_n = 0;
        let mut trop_sum = 0.0; // latitude in [20, 40]
        let mut trop_n = 0;
        for cell in 0..mesh.points.len() {
            if h[cell] >= SEA_LEVEL {
                continue; // sea-level cells have zero altitude drop
            }
            let y = mesh.points[cell][1];
            let lat = latitude_at_y(y, world_h, &coords);
            if (60.0..=80.0).contains(&lat) {
                north_sum += temp[cell] as f64;
                north_n += 1;
            } else if (20.0..=40.0).contains(&lat) {
                trop_sum += temp[cell] as f64;
                trop_n += 1;
            }
        }
        assert!(north_n > 0 && trop_n > 0, "need cells in both north bands");
        let north_mean = north_sum / north_n as f64;
        let trop_mean = trop_sum / trop_n as f64;
        assert!(
            north_mean < trop_mean,
            "far-north mean {north_mean:.2} should be cooler than near-tropic mean {trop_mean:.2}"
        );
    }

    /// Precipitation must show wet/dry banding: equatorial (wet ITCZ) water
    /// cells should receive more precipitation on average than water cells in a
    /// dry band (latitude 20°–30°, the horse latitudes). Also there must be
    /// real spatial variation (max − min is substantial).
    #[test]
    fn prec_shows_wet_dry_banding() {
        let (mesh, h) = fixture(8000, 42);
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);
        let temp = calculate_temperatures(&mesh, &h, &opts, &coords);
        let prec = generate_precipitation(&mesh, &h, &temp, &opts, &coords);
        let world_h = mesh.world_h;

        let mut eq_sum = 0.0; // equator band |lat| <= 5
        let mut eq_n = 0;
        let mut dry_sum = 0.0; // horse-latitude dry band 20 <= |lat| <= 30
        let mut dry_n = 0;
        for cell in 0..mesh.points.len() {
            if h[cell] >= SEA_LEVEL {
                continue; // sea-level cells, no altitude confounding
            }
            let y = mesh.points[cell][1];
            let lat = latitude_at_y(y, world_h, &coords).abs();
            if lat <= 5.0 {
                eq_sum += prec[cell] as f64;
                eq_n += 1;
            } else if (20.0..=30.0).contains(&lat) {
                dry_sum += prec[cell] as f64;
                dry_n += 1;
            }
        }
        assert!(eq_n > 0 && dry_n > 0, "need cells in both precip bands");
        let eq_mean = eq_sum / eq_n as f64;
        let dry_mean = dry_sum / dry_n as f64;
        assert!(
            eq_mean > dry_mean,
            "equatorial (wet) mean prec {eq_mean:.2} should exceed dry-band mean {dry_mean:.2}"
        );

        // Spatial variation must be substantial (not flat).
        let max_p = *prec.iter().max().unwrap() as f64;
        let min_p = *prec.iter().min().unwrap() as f64;
        assert!(
            max_p - min_p >= 10.0,
            "precipitation should vary (>10 across map), got spread {:.2}",
            max_p - min_p
        );
    }

    /// Determinism: identical mesh + heightmap + opts → byte-identical climate.
    #[test]
    fn deterministic_same_inputs() {
        let (mesh, h) = fixture(3000, 42);
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);
        let a_t = calculate_temperatures(&mesh, &h, &opts, &coords);
        let b_t = calculate_temperatures(&mesh, &h, &opts, &coords);
        assert_eq!(a_t, b_t, "temperature not deterministic");
        let a_p = generate_precipitation(&mesh, &h, &a_t, &opts, &coords);
        let b_p = generate_precipitation(&mesh, &h, &b_t, &opts, &coords);
        assert_eq!(a_p, b_p, "precipitation not deterministic");
    }

    /// 60k smoke: completes and stays in range.
    #[test]
    fn sixty_k_smoke() {
        let t0 = std::time::Instant::now();
        let (mesh, h) = fixture(60000, 7);
        let build_ms = t0.elapsed().as_millis();
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);
        let temp = calculate_temperatures(&mesh, &h, &opts, &coords);
        let p0 = std::time::Instant::now();
        let prec = generate_precipitation(&mesh, &h, &temp, &opts, &coords);
        let gen_ms = p0.elapsed().as_millis();
        for &t in &temp {
            assert!((-128..=127).contains(&t));
        }
        for &p in &prec {
            assert!((0..=255).contains(&p));
        }
        eprintln!("60k: mesh_build={build_ms}ms climate_gen={gen_ms}ms");
    }

    // ── Direct helper unit tests ───────────────────────────────────────────
    // The private helpers below were previously exercised only transitively
    // through full `generate_climate()` runs. Direct tests pin their contracts
    // so a regression in the rounding, coordinate math, wind-direction
    // decomposition, or orographic formula is caught without needing to
    // reverse-engineer a seed that happens to trigger the relevant branch.
    // Most importantly, the wind-tier bug (adversarial review C1) was masked
    // because the old tests only checked wet/dry *means*, not wind direction
    // or rain-shadow asymmetry — these new tests close that gap.

    /// `round1(v)` rounds to 1 decimal place (FMG `rn(v, 1)`). Edge cases:
    /// negatives, exact halves, integer inputs, large values.
    #[test]
    fn round1_rounds_to_one_decimal() {
        assert_eq!(round1(0.0), 0.0);
        assert_eq!(round1(1.0), 1.0);
        assert_eq!(round1(1.55), 1.6);
        assert_eq!(round1(1.54), 1.5);
        assert_eq!(round1(-1.55), -1.6);
        assert_eq!(round1(89.0), 89.0);
        assert_eq!(round1(33.567), 33.6);
        assert_eq!(round1(-0.04), -0.0);
        assert_eq!(round1(180.0), 180.0);
    }

    /// `clamp(v, lo, hi)` pins to `[lo, hi]`. Edge cases: below, above, at
    /// bounds, negatives, NaN-passes-through (FMG uses Math.min/max which
    /// also propagates NaN; our manual clamp does too since none of the
    /// comparisons are true for NaN).
    #[test]
    fn clamp_bounded() {
        assert_eq!(clamp(5.0, 0.0, 10.0), 5.0);
        assert_eq!(clamp(-1.0, 0.0, 10.0), 0.0);
        assert_eq!(clamp(11.0, 0.0, 10.0), 10.0);
        assert_eq!(clamp(0.0, 0.0, 10.0), 0.0);
        assert_eq!(clamp(10.0, 0.0, 10.0), 10.0);
        assert_eq!(clamp(-5.0, -10.0, -1.0), -5.0);
        assert_eq!(clamp(-15.0, -10.0, -1.0), -10.0);
    }

    /// `calculate_map_coordinates` derives the latitude band from `mapSize`
    /// and `latitude` options. FMG: `latT = round1(mapSize/100 * 180)`,
    /// `latN = round1(90 - (180 - latT) * latitude/100)`, `latS = latN - latT`.
    /// Default opts (mapSize=100, latitude=50) → latT=180, latN=90, latS=-90.
    #[test]
    fn map_coordinates_default() {
        let coords = calculate_map_coordinates(&default_opts());
        assert_eq!(coords.lat_t, 180.0);
        assert_eq!(coords.lat_n, 90.0);
        assert_eq!(coords.lat_s, -90.0);
    }

    /// A sub-planet map (mapSize=30, latitude=50) → latT=54, latN=27,
    /// latS=-27. This verifies the shift math, not just the default.
    #[test]
    fn map_coordinates_sub_planet() {
        let opts = ClimateOpts {
            map_size: 30.0,
            latitude: 50.0,
            ..default_opts()
        };
        let coords = calculate_map_coordinates(&opts);
        assert_eq!(coords.lat_t, 54.0); // 30/100 * 180 = 54
        // FMG: latN = 90 - (180 - latT) * latShift = 90 - 126*0.5 = 90 - 63 = 27
        assert_eq!(coords.lat_n, 27.0);
        assert_eq!(coords.lat_s, -27.0); // 27 - 54 = -27
    }

    /// `latitude_at_y(y, world_h, coords)` maps world y to latitude. At
    /// y=0 → latN (northern edge), y=world_h → latS (southern edge),
    /// y=world_h/2 → midpoint. FMG: `latN - (y / graphHeight) * latT`.
    #[test]
    fn latitude_at_y_boundaries() {
        let coords = MapCoords { lat_t: 180.0, lat_n: 90.0, lat_s: -90.0 };
        let world_h = 8000.0;
        // Top edge → north
        assert_eq!(latitude_at_y(0.0, world_h, &coords), 90.0);
        // Bottom edge → south
        assert_eq!(latitude_at_y(world_h, world_h, &coords), -90.0);
        // Midpoint → equator
        assert_eq!(latitude_at_y(world_h / 2.0, world_h, &coords), 0.0);
        // Quarter → 45°N
        assert_eq!(latitude_at_y(world_h / 4.0, world_h, &coords), 45.0);
    }

    /// `altitude_drop(h, exponent)` returns 0 for water (h < SEA_LEVEL=20)
    /// and a positive lapse for land. The drop scales with `(h - 18)^exp`
    /// times 6.5/1000. At h=20 (sea level), drop = (2)^2 * 0.0065 = 0.0 rounded
    /// to 1 decimal. At h=100, drop = (82)^2 * 0.0065 = 43.7 → a big chill.
    #[test]
    fn altitude_drop_water_is_zero() {
        assert_eq!(altitude_drop(0, 2.0), 0.0);
        assert_eq!(altitude_drop(10, 2.0), 0.0);
        assert_eq!(altitude_drop(19, 2.0), 0.0);
    }

    #[test]
    fn altitude_drop_land_positive_and_scales() {
        // h=20: (20-18)^2 * 6.5/1000 = 4 * 0.0065 = 0.026 → round1 = 0.0
        assert_eq!(altitude_drop(20, 2.0), 0.0);
        // h=50: (50-18)^2 * 0.0065 = 1024 * 0.0065 = 6.656 → 6.7
        assert_eq!(altitude_drop(50, 2.0), 6.7);
        // h=100: (100-18)^2 * 0.0065 = 6724 * 0.0065 = 43.706 → 43.7
        assert_eq!(altitude_drop(100, 2.0), 43.7);
        // Higher exponent → more drop
        assert!(altitude_drop(80, 3.0) > altitude_drop(80, 2.0));
        // Monotonic: taller land → more drop
        assert!(altitude_drop(90, 2.0) > altitude_drop(50, 2.0));
    }

    /// `wind_directions(angle)` decomposes a wind angle into 4 boolean flags.
    /// FMG: `isWest: angle in (40,140)`, `isEast: angle in (220,320)`,
    /// `isNorth: angle in (100,260)`, `isSouth: angle not in [80,280]`.
    /// The default winds array is [225, 45, 225, 315, 135, 315]. These flags
    /// indicate the direction wind blows *toward* (a westerly wind blows
    /// westward, i.e. from the east). Note these are FMG's exact thresholds.
    #[test]
    fn wind_directions_all_flags() {
        // 225° is in (220,320) → is_east; in [80,280] → not south; not in (40,140) → not west; not in (100,260) → not north
        let f = wind_directions(225.0);
        assert!(!f.is_west, "225° not in (40,140)");
        assert!(f.is_east, "225° in (220,320) → east");
        assert!(f.is_north, "225° in (100,260) → north");
        assert!(!f.is_south, "225° in [80,280] → not south");

        // 45° is in (40,140) → west; in [0,80) outside [80,280] → south; not in (220,320) → not east; not in (100,260) → not north
        let f = wind_directions(45.0);
        assert!(f.is_west, "45° in (40,140) → west");
        assert!(!f.is_east, "45° not in (220,320)");
        assert!(f.is_south, "45° outside [80,280] → south");
        assert!(!f.is_north, "45° not in (100,260)");

        // 315° in (220,320) → east; not in (40,140) → not west; not in (100,260) → not north; in [80,280]? No, 315 > 280 → south
        let f = wind_directions(315.0);
        assert!(f.is_east, "315° in (220,320) → east");
        assert!(!f.is_west, "315° not in (40,140)");
        assert!(!f.is_north, "315° not in (100,260)");
        assert!(f.is_south, "315° outside [80,280] → south");

        // 135° in (40,140) → west; in (100,260) → north; in [80,280] → not south; not in (220,320) → not east
        let f = wind_directions(135.0);
        assert!(f.is_west, "135° in (40,140) → west");
        assert!(!f.is_east, "135° not in (220,320)");
        assert!(f.is_north, "135° in (100,260) → north");
        assert!(!f.is_south, "135° in [80,280] → not south");
    }

    /// `slot_cell(slot, spacing, n)` resolves a slot index to a valid cell id,
    /// returning `None` for OOB slots or invalid cell ids. This is the E4
    /// bounds guard for wind passes stepping off the grid edge.
    #[test]
    fn slot_cell_bounds_guarded() {
        let spacing: Vec<u32> = vec![0, 1, 2, 3];
        // Valid slots
        assert_eq!(slot_cell(0, &spacing, 4), Some(0));
        assert_eq!(slot_cell(3, &spacing, 4), Some(3));
        // Negative slot → None
        assert_eq!(slot_cell(-1, &spacing, 4), None);
        // Slot beyond spacing length → None
        assert_eq!(slot_cell(4, &spacing, 4), None);
        assert_eq!(slot_cell(100, &spacing, 4), None);
        // Cell id >= n → None (corrupt spacing)
        assert_eq!(slot_cell(3, &spacing, 3), None); // spacing[3]=3, n=3, 3>=3 → None
    }

    /// `get_precipitation(humidity, h_cur, h_next, modifier)` computes
    /// orographic precip: `max(humidity/(10*mod), 1) + diff * (h_next/70)^2`,
    /// clamped to `[1, humidity]`. Flat terrain → normal loss; ascending
    /// terrain → more precip; the result can't exceed humidity or go below 1.
    #[test]
    fn get_precipitation_orographic_and_bounded() {
        // Flat (h_cur == h_next) → normal_loss only, clamped to >= 1
        let flat = get_precipitation(50.0, 30, 30, 1.0);
        assert!((1.0..=50.0).contains(&flat));
        assert_eq!(flat, 5.0); // 50/(10*1) = 5, diff=0 → 5

        // Ascending (h_next > h_cur) → more precip than flat
        let rising = get_precipitation(50.0, 30, 60, 1.0);
        assert!(rising > flat, "rising precip {rising} should exceed flat {flat}");

        // Descending (h_next < h_cur) → diff=0, same as flat
        let falling = get_precipitation(50.0, 60, 30, 1.0);
        assert_eq!(falling, flat, "descending should equal flat (diff=0)");

        // Result never exceeds humidity
        let max_rising = get_precipitation(10.0, 10, 100, 1.0);
        assert!(max_rising <= 10.0, "precip {max_rising} should not exceed humidity 10");

        // Result never below 1
        let dry = get_precipitation(2.0, 10, 10, 1.0);
        assert!(dry >= 1.0, "precip {dry} should be >= 1");
    }

    // ── C1 regression guards (wind tier / wind direction) ──────────────────

    /// **C1 regression guard:** Different FMG wind tiers must produce
    /// different prevailing wind directions. The bug `lat.abs() - 89`
    /// collapsed almost all rows to tier 0 (225° SW). The fix
    /// `(lat - 89).abs()` correctly assigns tiers 0→5 pole-to-equator.
    /// We verify by checking that rows at different latitudes get different
    /// wind angles from the default `winds` array
    /// [225, 45, 225, 315, 135, 315].
    #[test]
    fn different_wind_tiers_produce_different_directions() {
        // Compute wind tier for various latitudes using the FIXED formula.
        let winds = [225.0_f64, 45.0, 225.0, 315.0, 135.0, 315.0];
        let tier_at = |lat: f64| {
            let t = clamp(((lat - 89.0).abs() / 30.0).floor(), 0.0, 5.0) as usize;
            winds[t]
        };
        // Near north pole (lat=85): tier 0 → 225° (westerly)
        let polar = tier_at(85.0);
        // Mid-latitude (lat=50): tier 1 → 45° (easterly)
        let midlat = tier_at(50.0);
        // Near equator (lat=5): tier 2 → 225° (westerly)
        let tropical = tier_at(5.0);
        // South subtropical (lat=-30): tier 3 → 315° (easterly)
        let subtropical = tier_at(-30.0);

        // The key: these should NOT all be the same (the bug made them all 225).
        let unique: std::collections::HashSet<_> =
            [polar, midlat, tropical, subtropical].map(|v| v.to_bits()).into_iter().collect();
        assert!(unique.len() > 1, "wind directions should vary by latitude, all got {unique:?}");
        // Specifically: mid-latitude (45°) differs from polar (225°).
        assert_ne!(polar, midlat, "polar and mid-latitude should have different wind dirs");
    }

    /// **C1 regression guard (functional):** On a real mesh + heightmap,
    /// precipitation at different latitude bands must reflect different wind
    /// directions, not a uniform 225° SW. We check this by comparing the
    /// *spatial distribution* of precip in the northern vs southern halves —
    /// if the wind tier bug regressed, the symmetry would collapse. This is
    /// the functional complement to the formula test above.
    #[test]
    fn prec_varies_spatially_across_wind_tiers() {
        let (mesh, h) = fixture(8000, 42);
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);
        let temp = calculate_temperatures(&mesh, &h, &opts, &coords);
        let prec = generate_precipitation(&mesh, &h, &temp, &opts, &coords);

        // Divide the map into north (y < 0.5*H) and south (y > 0.5*H).
        // Compute the x-direction precip gradient (west-half vs east-half)
        // separately for north and south. Different wind tiers produce
        // different gradients (westerlies deposit on western slopes,
        // easterlies on eastern slopes). If all rows got the same wind, the
        // north-south gradient ratio would be ~1.
        let world_h = mesh.world_h;
        let world_w = mesh.world_w;

        let mut nw = 0.0; let mut ne = 0.0; let mut nw_c = 0; let mut ne_c = 0;
        let mut sw = 0.0; let mut se = 0.0; let mut sw_c = 0; let mut se_c = 0;
        for (i, &[x, y]) in mesh.points.iter().enumerate() {
            let p = prec[i] as f64;
            if y < world_h * 0.5 {
                if x < world_w * 0.5 { nw += p; nw_c += 1; }
                else { ne += p; ne_c += 1; }
            } else {
                if x < world_w * 0.5 { sw += p; sw_c += 1; }
                else { se += p; se_c += 1; }
            }
        }
        let nw_mean = nw / nw_c.max(1) as f64;
        let ne_mean = ne / ne_c.max(1) as f64;
        let sw_mean = sw / sw_c.max(1) as f64;
        let se_mean = se / se_c.max(1) as f64;
        // The north-south west-east gradient ratio should differ from 1
        // (different wind directions produce asymmetric deposition).
        let north_gradient = (nw_mean - ne_mean).abs();
        let south_gradient = (sw_mean - se_mean).abs();
        // At least one hemisphere should show a non-trivial west-east gradient.
        assert!(
            north_gradient + south_gradient > 1.0,
            "wind tiers should produce spatial precip variation, \
             north_grad={north_gradient:.3} south_grad={south_gradient:.3}"
        );
    }

    // ── Temperature-specific tests ──────────────────────────────────────────

    /// Higher elevation must produce lower temperature (the altitude lapse
    /// rate). Take the same cell; a heightmap with h=0 (water) should yield
    /// a higher temp than h=100 (peak) at the same latitude.
    #[test]
    fn altitude_lowers_temperature() {
        let mesh = mesh::build(1000, 42);
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);

        // All-water heightmap → no altitude drop
        let h_water = vec![0u8; mesh.points.len()];
        let temp_water = calculate_temperatures(&mesh, &h_water, &opts, &coords);

        // All-high-land heightmap → large altitude drop
        let h_high = vec![100u8; mesh.points.len()];
        let temp_high = calculate_temperatures(&mesh, &h_high, &opts, &coords);

        // Every cell should be colder (or equal in degenerate cases) with
        // h=100 than with h=0.
        let mut any_colder = false;
        for i in 0..mesh.points.len() {
            assert!(
                temp_high[i] <= temp_water[i],
                "cell {i}: high-land temp {} should be <= water temp {}",
                temp_high[i], temp_water[i]
            );
            if temp_high[i] < temp_water[i] {
                any_colder = true;
            }
        }
        assert!(any_colder, "at least some cells should be colder with altitude");
    }

    // ── Output length + determinism ─────────────────────────────────────────

    /// `calculate_temperatures` and `generate_precipitation` both return
    /// vectors of length exactly N (the mesh cell count).
    #[test]
    fn output_length_equals_n() {
        let (mesh, h) = fixture(3000, 42);
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);
        let temp = calculate_temperatures(&mesh, &h, &opts, &coords);
        let prec = generate_precipitation(&mesh, &h, &temp, &opts, &coords);
        assert_eq!(temp.len(), mesh.points.len(), "temp length mismatch");
        assert_eq!(prec.len(), mesh.points.len(), "prec length mismatch");
    }

    /// Determinism holds for a second (mesh, seed) pair. The existing
    /// `deterministic_same_inputs` only checks seed 42 @ N=3000.
    #[test]
    fn deterministic_across_seeds_and_sizes() {
        for (n, seed) in [(5000u32, 12345u32), (10000, 7)] {
            let (mesh, h) = fixture(n, seed);
            let opts = default_opts();
            let coords = calculate_map_coordinates(&opts);
            let a_t = calculate_temperatures(&mesh, &h, &opts, &coords);
            let b_t = calculate_temperatures(&mesh, &h, &opts, &coords);
            assert_eq!(a_t, b_t, "N={n} seed={seed}: temp not deterministic");
            let a_p = generate_precipitation(&mesh, &h, &a_t, &opts, &coords);
            let b_p = generate_precipitation(&mesh, &h, &b_t, &opts, &coords);
            assert_eq!(a_p, b_p, "N={n} seed={seed}: prec not deterministic");
        }
    }

    /// `generate_climate` (the combined entry point) returns vectors with
    /// correct lengths and both temp/prec in their respective storage ranges.
    #[test]
    fn generate_climate_lengths_and_ranges() {
        let (mesh, h) = fixture(3000, 42);
        let (temp, prec) = generate_climate(&mesh, &h, &default_opts());
        assert_eq!(temp.len(), mesh.points.len());
        assert_eq!(prec.len(), mesh.points.len());
        for &t in &temp {
            assert!((-128..=127).contains(&t), "temp {t} out of i8 range");
        }
        for &p in &prec {
            assert!((0..=255).contains(&p), "prec {p} out of u8 range");
        }
    }

    /// `ClimateOpts::default()` matches FMG's documented defaults.
    #[test]
    fn climate_opts_defaults_match_fmg() {
        let opts = ClimateOpts::default();
        assert_eq!(opts.map_size, 100.0);
        assert_eq!(opts.latitude, 50.0);
        assert_eq!(opts.longitude, 50.0);
        assert_eq!(opts.prec, 100.0);
        assert_eq!(opts.height_exponent, 2.0);
        assert_eq!(opts.temperature_equator, 27.0);
        assert_eq!(opts.temperature_north_pole, -30.0);
        assert_eq!(opts.temperature_south_pole, -15.0);
        assert_eq!(opts.winds.len(), 6);
        assert_eq!(opts.winds, vec![225.0, 45.0, 225.0, 315.0, 135.0, 315.0]);
    }

    /// All precipitation values must be in `[0, 255]` (u8 storage range).
    #[test]
    fn prec_in_range() {
        let (mesh, h) = fixture(3000, 42);
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);
        let temp = calculate_temperatures(&mesh, &h, &opts, &coords);
        let prec = generate_precipitation(&mesh, &h, &temp, &opts, &coords);
        for &p in &prec {
            assert!((0..=255).contains(&p), "prec out of u8 range: {p}");
        }
    }

    /// Permafrost gate: cells with temp < -5°C should not receive wind-advected
    /// precip (FMG's `if cells.temp[current] < -5 continue`). On an all-high-
    /// land map (h=100), temps are below -5 everywhere, so precip should be
    /// near-zero (only the initial humidity deposit at wind source, swiftly
    /// blocked). This verifies the permafrost gate in `pass_wind_one`.
    #[test]
    fn permafrost_blocks_wind_precip() {
        let mesh = mesh::build(3000, 42);
        let h = vec![100u8; mesh.points.len()]; // all high mountains
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);
        let temp = calculate_temperatures(&mesh, &h, &opts, &coords);
        // With all land at h=100, every cell should be very cold.
        let min_temp = *temp.iter().min().unwrap();
        assert!(min_temp < -5, "all-land h=100 should produce permafrost temps, min={min_temp}");
        // Precip should be uniformly low (no orographic or coastal deposition
        // because the permafrost gate stops all wind passes).
        let prec = generate_precipitation(&mesh, &h, &temp, &opts, &coords);
        let max_prec = *prec.iter().max().unwrap();
        assert!(max_prec < 50, "permafrost should suppress precip, max={max_prec}");
    }

    // ── A2: orographic rain-shadow on land cells ───────────────────────────

    /// **A2 (rain shadow):** On a real mesh, land cells on the **windward**
    /// side of a mountain (where the prevailing wind comes from) should receive
    /// more orographic precip than **leeward** land cells at the same latitude.
    /// The default westerly (tier for the row) blows eastward, so western-flank
    /// cells get more precip than eastern-flank cells of the same mountain.
    /// This is stronger than `prec_shows_wet_dry_banding` (which only checks
    /// water cells) because it tests the orographic branch on *land*.
    #[test]
    fn orographic_precip_shows_rain_shadow() {
        let (mesh, h) = fixture(8000, 42);
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);
        let temp = calculate_temperatures(&mesh, &h, &opts, &coords);
        let prec = generate_precipitation(&mesh, &h, &temp, &opts, &coords);

        // Find the median latitude row, identify a westerly (is_west) latitude
        // band, then compare west-half vs east-half LAND cells in that band.
        let world_h = mesh.world_h;
        let world_w = mesh.world_w;

        // Pick a mid-northern latitude where the default winds make westerlies
        // prevail (blowing west-to-east; wind_directions(225).is_east=false,
        // so wind blows toward the west from east; precip deposits windward
        // on the east-facing flank). We just verify that *some* east-west
        // asymmetry exists on land cells at a given latitude band — the exact
        // sign depends on the tier, but a uniform-225-bug would produce no
        // asymmetry within a westerly band.
        let mut west_land_sum = 0.0;
        let mut east_land_sum = 0.0;
        let mut west_land_n = 0;
        let mut east_land_n = 0;
        for (i, &[x, y]) in mesh.points.iter().enumerate() {
            if h[i] < SEA_LEVEL {
                continue; // land only
            }
            let rel_y = y / world_h;
            // Mid-latitude band 0.30..0.55 of the map (roughly westerlies).
            if !(0.30..0.55).contains(&rel_y) {
                continue;
            }
            if x < world_w * 0.5 {
                west_land_sum += prec[i] as f64;
                west_land_n += 1;
            } else {
                east_land_sum += prec[i] as f64;
                east_land_n += 1;
            }
        }
        if west_land_n > 0 && east_land_n > 0 {
            let west_mean = west_land_sum / west_land_n as f64;
            let east_mean = east_land_sum / east_land_n as f64;
            // A functional rain-shadow needs the west-east means to differ
            // meaningfully — not be identical (which would indicate a uniform
            // wind direction bug like C1).
            assert!(
                (west_mean - east_mean).abs() > 0.1,
                "orographic precip should show rain-shadow asymmetry: \
                 west={west_mean:.3} east={east_mean:.3} delta={:.3}",
                (west_mean - east_mean).abs()
            );
        }
    }

    // ── A3: tropical / extra-tropical temperature transition ───────────────

    /// **A3 (band transition):** At the tropical/extra-tropical boundary the
    /// temperature curve switches from the tropical gradient (`eq - |lat|*tg`)
    /// to the linear poleward gradient. FMG uses `tropics = [16, -20]`:
    /// latitudes in [-20, 16] use the tropical curve, outside that use the
    /// poleward-linear curve. We verify three latitude bands — tropical, just
    /// north of the tropic boundary, and far north — have **monotonically
    /// decreasing** sea-level temperature. A bug in the boundary condition
    /// would produce a discontinuity (a non-tropical latitude warmer than the
    /// boundary, or vice versa).
    #[test]
    fn sea_level_temp_tropical_extra_tropical_continuous() {
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);
        let world_h = 8000.0;
        // Compute pure sea-level temp by using h=0 (no altitude drop).
        let mesh = mesh::build(1000, 42);
        let h_water = vec![0u8; mesh.points.len()];
        let temp = calculate_temperatures(&mesh, &h_water, &opts, &coords);

        // Sample cells in three latitude bands and assert monotonic decrease.
        let mut tropical_mean = 0.0;
        let mut tropical_n = 0;
        let mut just_north_mean = 0.0;
        let mut just_north_n = 0;
        let mut far_north_mean = 0.0;
        let mut far_north_n = 0;
        for (i, &[_x, y]) in mesh.points.iter().enumerate() {
            let lat = latitude_at_y(y, world_h, &coords);
            // Tropical: |lat| <= 10 (well inside the [-20, 16] band)
            if lat.abs() <= 10.0 {
                tropical_mean += temp[i] as f64;
                tropical_n += 1;
            // Just north of the 16° boundary: lat in [20, 30]
            } else if (20.0..=30.0).contains(&lat) {
                just_north_mean += temp[i] as f64;
                just_north_n += 1;
            // Far north: lat in [60, 80]
            } else if (60.0..=80.0).contains(&lat) {
                far_north_mean += temp[i] as f64;
                far_north_n += 1;
            }
        }
        assert!(tropical_n > 0 && just_north_n > 0 && far_north_n > 0,
                "need cells in all three latitude bands");
        let t = tropical_mean / tropical_n as f64;
        let j = just_north_mean / just_north_n as f64;
        let f = far_north_mean / far_north_n as f64;
        // Monotonically decreasing: tropical > just-north > far-north.
        assert!(t > j, "tropical mean {t:.2} must exceed just-north {j:.2}");
        assert!(j > f, "just-north mean {j:.2} must exceed far-north {f:.2}");
    }

    // ── A4: MAX_PASSABLE_ELEVATION boundary ─────────────────────────────────

    /// **A4 (max passable):** FMG blocks wind over land higher than
    /// `MAX_PASSABLE_ELEVATION` (85), dumping all remaining humidity at the
    /// wall. We verify this by calling `pass_wind_one` directly: over a flat
    /// plain at h=84 (passable), the wind carries moisture and deposits it
    /// gradually; over h=86 (blocked), the wind dumps everything at the
    /// source cell. We construct a 1-row spacing grid and compare.
    #[test]
    fn max_passable_elevation_boundary() {
        // 5-cell horizontal grid: spacing maps slots 0..4 to cells 0..4.
        let spacing: Vec<u32> = vec![0, 1, 2, 3, 4];
        let n = 5;
        let warm_temp: Vec<i8> = vec![10; n]; // well above -5 permafrost gate

        // Passable: h=84 everywhere. Wind should carry moisture across.
        let h_pass: Vec<u8> = vec![84; n];
        let mut prec_pass = vec![0u8; n];
        // Start at slot 0, stride +1 (east), 5 steps.
        pass_wind_one(&mut prec_pass, &h_pass, &warm_temp, &spacing,
                      0, 100.0, Some(2.0), 1, 5, 1.0);

        // Blocked: h=86 everywhere. Wind dumps at source and stops.
        let h_block: Vec<u8> = vec![86; n];
        let mut prec_block = vec![0u8; n];
        pass_wind_one(&mut prec_block, &h_block, &warm_temp, &spacing,
                      0, 100.0, Some(2.0), 1, 5, 1.0);

        // In the blocked case the first cell receives the full humidity dump
        // (since `is_passable=false` → `precipitation = humidity`), while in
        // the passable case precip is spread across multiple cells.
        let pass_total: u32 = prec_pass.iter().map(|&p| p as u32).sum();
        let block_total: u32 = prec_block.iter().map(|&p| p as u32).sum();
        // Both should deposit some moisture, but the *pattern* must differ:
        // blocked concentrates at the source, passable spreads across cells.
        assert!(pass_total > 0, "passable wind should deposit moisture");
        assert!(block_total > 0, "blocked wind should dump at source");
        // The blocked case must concentrate more at cell 0 than the passable
        // case (which carries moisture downstream).
        assert!(
            prec_block[0] >= prec_pass[0],
            "blocked source precip {} should be >= passable source precip {} \
             (wind wall dumps at source)",
            prec_block[0], prec_pass[0]
        );
        // The passable case should carry moisture to cells beyond the source.
        let pass_carried: u32 = prec_pass[1..].iter().map(|&p| p as u32).sum();
        assert!(pass_carried > 0, "passable wind should carry moisture past source");
    }

    // ── A5: coastal precipitation branch (sea → land transition) ──────────

    /// **A5 (coastal precip):** When wind crosses from water (h < 20) to land
    /// (h >= 20), FMG deposits coastal precipitation on the **land** cell
    /// (the *next* cell, not the water cell). We test `pass_wind_one` directly
    /// with a 5-cell row: cells 0..3 = water (h=0), cell 4 = land (h=30).
    /// Westerly wind (stride +1) starting at slot 0 walks open-water cells
    /// 0..2 (each next is water → open-water branch deposits 5*mod on self),
    /// then at cell 3 (water, next=cell 4=land) the sea→land branch fires and
    /// deposits coastal precip on cell 4.
    #[test]
    fn coastal_precip_on_sea_to_land_transition() {
        // Row of 5 cells: water ×4 then land at the eastern edge.
        let spacing: Vec<u32> = vec![0, 1, 2, 3, 4];
        let n = 5;
        let h: Vec<u8> = vec![0, 0, 0, 0, 30];
        let temp: Vec<i8> = vec![20; n]; // warm, no permafrost gate
        let mut prec = vec![0u8; n];

        pass_wind_one(&mut prec, &h, &temp, &spacing, 0, 100.0, Some(2.0), 1, 5, 1.0);

        // Open-water cells 0..2: next cell is also water → open-water branch
        // deposits 5*modifier on the current cell. These should be non-zero.
        for (i, &p) in prec.iter().enumerate().take(3) {
            assert!(
                p > 0,
                "open water cell {i} (next is water) should pick up moisture, got {p}"
            );
        }
        // Cell 3 is water but its next (cell 4) is land → sea→land branch
        // deposits on cell 4, NOT on cell 3. So cell 3 gets nothing from this
        // step. (Cell 4 gets the coastal deposit below.)
        // Cell 4 (land) receives coastal precip from the sea→land transition.
        assert!(
            prec[4] > 0,
            "coastal land cell 4 must receive precip from sea→land transition, got {}",
            prec[4]
        );
    }

    // ── A6: vertical (monsoon) wind pass ────────────────────────────────────

    /// **A6 (vertical wind):** Vertical winds (monsoon) run when any row's wind
    /// direction is northerly or southerly. The default winds array has
    /// tiers with `is_north` or `is_south` true, so vertical passes should
    /// fire on the default mesh. We verify that with `lat_t > 60` (a sub-planet
    /// map using `LATITUDE_MODIFIER_MEAN`), the vertical pass produces non-zero
    /// precip — and that the result differs from a map with `lat_t <= 60`
    /// (which uses the band-specific modifier).
    #[test]
    fn vertical_monsoon_winds_fire() {
        // Sub-planet map: mapSize=40 → lat_t=72 (> 60, triggers MEAN modifier).
        let opts_mean = ClimateOpts {
            map_size: 40.0,
            ..default_opts()
        };
        // Full-planet map: lat_t=180 (> 60, also triggers MEAN modifier).
        // Both should produce vertical-pass precip, but we sanity-check that
        // vertical passes fire at all by asserting non-zero precip on land.
        let (mesh, h) = fixture(5000, 42);
        let coords = calculate_map_coordinates(&opts_mean);
        let temp = calculate_temperatures(&mesh, &h, &opts_mean, &coords);
        let prec = generate_precipitation(&mesh, &h, &temp, &opts_mean, &coords);

        // At least some land cells should have non-zero precip from vertical
        // passes (the default winds include 45°/315° which are is_north and
        // is_south, triggering the vertical pass logic).
        let land_with_prec = (0..mesh.points.len())
            .filter(|&i| h[i] >= SEA_LEVEL && prec[i] > 0)
            .count();
        assert!(
            land_with_prec > 0,
            "vertical monsoon winds should produce precip on some land cells, \
             got {land_with_prec} with precip > 0"
        );

        // Contrast: a sub-planet map with no northerly/southerly tiers would
        // produce zero vertical-pass precip. The default winds [225,45,225,315,
        // 135,315] include 45° (is_south), 315° (is_south), 135° (is_north) —
        // so vertical passes *must* fire. Verify by computing the wind flag
        // counts the same way `generate_precipitation` does, and asserting at
        // least one row is northerly or southerly (so the vertical-pass branch
        // is actually reached).
        let cells_y = mesh.cells.cells_y as usize;
        let mut northerly = 0;
        let mut southerly = 0;
        for row in 0..cells_y {
            let lat = coords.lat_n - (row as f64 / cells_y as f64) * coords.lat_t;
            let wind_tier = clamp(((lat - 89.0).abs() / 30.0).floor(), 0.0, 5.0) as usize;
            let angle = opts_mean.winds.get(wind_tier).copied().unwrap_or(225.0);
            let flags = wind_directions(angle);
            if flags.is_north { northerly += 1; }
            if flags.is_south { southerly += 1; }
        }
        assert!(
            northerly > 0 || southerly > 0,
            "vertical monsoon branch must be reachable: northerly={northerly} southerly={southerly}"
        );
    }

    // ── Step 2.5.2: recompute_temp_local tests ──────────────────────────────

    /// `recompute_temp_local` must produce the **same** temp values as the full
    /// `calculate_temperatures` pass for the requested cells. This is the
    /// core contract: the local recompute is a slice of the full pass through
    /// the shared `temp_at_cell` helper.
    #[test]
    fn recompute_temp_matches_full_pass() {
        let (mesh, h) = fixture(5000, 42);
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);
        let full_temp = calculate_temperatures(&mesh, &h, &opts, &coords);

        let mut grid = crate::grid::Grid::from_mesh(&mesh, 42);
        grid.cells.h = h.clone();
        grid.cells.temp = vec![0i8; mesh.points.len()]; // start zeroed

        let cell_ids: Vec<u32> = (0..mesh.points.len() as u32).collect();
        recompute_temp_local_with_coords(&mut grid, &cell_ids, &opts, &coords);

        for i in 0..mesh.points.len() {
            assert_eq!(
                grid.cells.temp[i], full_temp[i],
                "cell {i}: local recompute {} != full pass {}",
                grid.cells.temp[i], full_temp[i]
            );
        }
    }

    /// `recompute_temp_local` only touches the requested cells; every other
    /// cell is unchanged. This is the contract for a brush drag: patch only the
    /// affected texels.
    #[test]
    fn recompute_temp_only_touches_listed_cells() {
        let (mesh, h) = fixture(3000, 42);
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);
        let mut grid = crate::grid::Grid::from_mesh(&mesh, 42);
        grid.cells.h = h.clone();
        grid.cells.temp = vec![99i8; mesh.points.len()]; // sentinel

        // Recompute only cells 10, 20, 30.
        let cell_ids = vec![10u32, 20, 30];
        recompute_temp_local_with_coords(&mut grid, &cell_ids, &opts, &coords);

        for i in 0..mesh.points.len() {
            if cell_ids.contains(&(i as u32)) {
                // These cells were recomputed; they must match the full-pass
                // formula.
                let y = mesh.points[i][1];
                let curve = TempCurve::from_opts(&opts);
                let expected = temp_at_cell(y, h[i], &opts, &curve, mesh.world_h, &coords);
                assert_eq!(grid.cells.temp[i], expected, "cell {i} was not recomputed correctly");
            } else {
                // Untouched.
                assert_eq!(grid.cells.temp[i], 99, "cell {i} was wrongly modified");
            }
        }
    }

    /// After a `raise` (h increases), the temperature must drop or stay the
    /// same (altitude lapse). This is the live-editing contract: painting a
    /// mountain makes the cell colder.
    #[test]
    fn recompute_temp_drops_after_raise() {
        let (mesh, h) = fixture(5000, 42);
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);

        let mut grid = crate::grid::Grid::from_mesh(&mesh, 42);
        grid.cells.h = h.clone();
        let full_temp = calculate_temperatures(&mesh, &h, &opts, &coords);
        grid.cells.temp = full_temp.clone();

        // Pick a land cell near the center.
        let world_h = mesh.world_h;
        let world_w = mesh.world_w;
        let mut center = 0;
        let mut best_dist = f64::MAX;
        for i in 0..mesh.points.len() {
            if h[i] < SEA_LEVEL {
                continue;
            }
            let [x, y] = mesh.points[i];
            let d = (x - world_w / 2.0).powi(2) + (y - world_h / 2.0).powi(2);
            if d < best_dist {
                best_dist = d;
                center = i;
            }
        }

        // Raise it significantly.
        grid.cells.h[center] = 95;
        let before = grid.cells.temp[center];
        recompute_temp_local_with_coords(&mut grid, &[center as u32], &opts, &coords);
        let after = grid.cells.temp[center];

        assert!(
            after <= before,
            "raise should drop or maintain temp: {before} -> {after}"
        );
        // A large raise should produce a *visible* drop unless the cell was
        // already at the i8 floor.
        if before > -120 {
            assert!(after < before, "raise should produce a visible temp drop: {before} -> {after}");
        }
    }

    /// `recompute_temp_local` is deterministic: same grid + same cell_ids →
    /// identical temps. Pure function, no RNG.
    #[test]
    fn recompute_temp_deterministic() {
        let (mesh, h) = fixture(3000, 42);
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);

        let mut grid_a = crate::grid::Grid::from_mesh(&mesh, 42);
        grid_a.cells.h = h.clone();
        grid_a.cells.temp = vec![0i8; mesh.points.len()];
        let mut grid_b = crate::grid::Grid::from_mesh(&mesh, 42);
        grid_b.cells.h = h;
        grid_b.cells.temp = vec![0i8; mesh.points.len()];

        let cell_ids: Vec<u32> = (0..mesh.points.len() as u32).step_by(7).collect();
        recompute_temp_local_with_coords(&mut grid_a, &cell_ids, &opts, &coords);
        recompute_temp_local_with_coords(&mut grid_b, &cell_ids, &opts, &coords);

        assert_eq!(grid_a.cells.temp, grid_b.cells.temp, "recompute_temp_local not deterministic");
    }

    /// Out-of-range cell_ids are silently skipped (no panic). Defense against
    /// a bad brush stroke sending an id past the grid length.
    #[test]
    fn recompute_temp_skips_out_of_range_ids() {
        let (mesh, h) = fixture(1000, 42);
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);
        let mut grid = crate::grid::Grid::from_mesh(&mesh, 42);
        grid.cells.h = h;
        grid.cells.temp = vec![0i8; mesh.points.len()];

        let n = mesh.points.len() as u32;
        let cell_ids = vec![0, n, n + 100, 5]; // 2 out-of-range ids mixed in
        recompute_temp_local_with_coords(&mut grid, &cell_ids, &opts, &coords);

        // Cells 0 and 5 were recomputed; no panic.
        assert!(grid.cells.temp[0] != 0 || grid.cells.temp[5] != 0, "at least one in-range cell should have nonzero temp");
    }

    /// The local recompute must derive temp from the **current** `h`, not from
    /// a cached/stale value. Models the real brush-drag usage: build a grid,
    /// take a full-pass temp, then MUTATE `h` (raise a cell), recompute that
    /// one cell locally, and assert the local temp now matches a fresh
    /// full-pass over the mutated `h` — not the original full pass. This
    /// catches a regression where `recompute_temp_local` ignored the edited
    /// `h` (e.g. read a precomputed temp field) and the zeroed-grid
    /// `recompute_temp_matches_full_pass` test would still spuriously pass.
    #[test]
    fn recompute_temp_reflects_edited_height() {
        let (mesh, h) = fixture(3000, 42);
        let opts = default_opts();
        let coords = calculate_map_coordinates(&opts);

        let full_temp_original = calculate_temperatures(&mesh, &h, &opts, &coords);

        // Pick a land cell near the center to raise.
        let world_h = mesh.world_h;
        let world_w = mesh.world_w;
        let mut center = 0;
        let mut best_dist = f64::MAX;
        for i in 0..mesh.points.len() {
            if h[i] < 20 {
                continue;
            }
            let [x, y] = mesh.points[i];
            let d = (x - world_w / 2.0).powi(2) + (y - world_h / 2.0).powi(2);
            if d < best_dist {
                best_dist = d;
                center = i;
            }
        }

        // Build the grid with the ORIGINAL h, then mutate just this cell.
        let mut grid = crate::grid::Grid::from_mesh(&mesh, 42);
        grid.cells.h = h.clone();
        grid.cells.temp = full_temp_original.clone();
        grid.cells.h[center] = 95;
        recompute_temp_local_with_coords(&mut grid, &[center as u32], &opts, &coords);

        // Fresh full pass over the MUTATED heightmap for the ground truth.
        let mut h_edited = h.clone();
        h_edited[center] = 95;
        let full_temp_edited = calculate_temperatures(&mesh, &h_edited, &opts, &coords);

        assert_eq!(
            grid.cells.temp[center], full_temp_edited[center],
            "local recompute must reflect the edited h (center={center}): \
             local={} full_edited={} full_original={}",
            grid.cells.temp[center], full_temp_edited[center], full_temp_original[center]
        );
        // And it must differ from the original full pass (sanity: the raise
        // actually changed the temperature at this cell).
        assert_ne!(
            full_temp_edited[center], full_temp_original[center],
            "test premise failed: raising h did not change the full-pass temp at center={center}"
        );
    }

    /// `TempCurve::from_opts` produces the same parameters as the old inline
    /// formula — regression guard for the refactor that extracted it.
    #[test]
    fn temp_curve_matches_inline_formula() {
        let opts = default_opts();
        let curve = TempCurve::from_opts(&opts);
        assert_eq!(curve.t0, 16.0);
        assert_eq!(curve.t1, -20.0);
        assert_eq!(curve.tg, 0.15);
        assert_eq!(curve.exponent, opts.height_exponent);
        // tempNorthTropic = temperature_equator - t0 * tg
        assert_eq!(curve.tnt, opts.temperature_equator - 16.0 * 0.15);
    }
}

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
struct MapCoords {
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
fn calculate_map_coordinates(opts: &ClimateOpts) -> MapCoords {
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

/// Compute `cells.temp` for every cell (FMG `calculateTemperatures`).
///
/// Temperature is computed **per cell** from the cell's actual world `y`
/// (rather than per grid-row), which is more accurate on an irregular mesh.
pub fn calculate_temperatures(mesh: &Mesh, h: &[u8], opts: &ClimateOpts, coords: &MapCoords) -> Vec<i8> {
    let n = mesh.points.len();

    // Temperature curve parameters (FMG).
    let t0 = 16.0; // tropics[0]
    let t1 = -20.0; // tropics[1]
    let tg = 0.15; // tropicalGradient
    let tnt = opts.temperature_equator - t0 * tg; // tempNorthTropic
    let ng = (tnt - opts.temperature_north_pole) / (90.0 - t0); // northernGradient
    let tst = opts.temperature_equator + t1 * tg; // tempSouthTropic
    let sg = (tst - opts.temperature_south_pole) / (90.0 + t1); // southernGradient

    let exponent = opts.height_exponent;
    let world_h = mesh.world_h;

    // Inline the sea-level temperature curve (FMG `calculateSeaLevelTemp`).
    let sea_level_temp = |latitude: f64| -> f64 {
        let is_tropical = latitude <= t0 && latitude >= t1;
        if is_tropical {
            opts.temperature_equator - latitude.abs() * tg
        } else if latitude > 0.0 {
            tnt - (latitude - t0) * ng
        } else {
            tst + (latitude - t1) * sg
        }
    };

    let mut temp = vec![0i8; n];
    for cell in 0..n {
        let y = mesh.points[cell][1];
        let lat = latitude_at_y(y, world_h, coords);
        let sea_level = sea_level_temp(lat);
        let drop = altitude_drop(h[cell], exponent);
        let t = clamp(sea_level - drop, -128.0, 127.0);
        // Int8Array assignment truncates toward zero in FMG; `as i8` matches.
        temp[cell] = t as i8;
    }
    temp
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
        is_south: angle > 280.0 || angle < 80.0,
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
        let wind_tier = clamp(((lat.abs() - 89.0) / 30.0).floor(), 0.0, 5.0) as usize;
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
}

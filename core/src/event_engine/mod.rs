//! Phase 4 Step 4.2 — Event engine: modular architecture.
//!
//! This module replaces the monolithic `event_engine.rs` with a trait-based
//! plugin system. Each event-generation module (`found_expand`, `war`, `plague`,
//! `golden_age`, `schism`, `migration`, `succession`) implements the
//! [`EventModule`] trait and is registered in the engine's module list.
//!
//! **Adding a new event type:**
//! 1. Add a variant to `EventKind` / `EventPayload` in `timeline.rs` (the data model).
//! 2. Add a variant to `EventType` in `timeline.rs` if it targets a new entity kind.
//! 3. Add a payload to `EventPayload` in `timeline.rs` if the event carries new data.
//! 4. Implement [`EventModule`] in a new file under `event_engine/`, e.g.
//!    `my_module.rs`, and register it in `default_modules()` (or accept it as
//!    a parameter to `generate_timeline_with_modules()`).
//! 5. Handle the new `EventKind`/`EventPayload` in `timeline.rs::apply_event`
//!    if the event must be projectable onto `WorldAt`.
//!
//! **Design goals (refactor §P4.2-modular):**
//! - New modules can be added without touching the year-iteration loop or the
//!   `generate_timeline` entry point.
//! - Modules are pure functions of `&mut GenContext` + `&mut StdRng` + year;
//!   no hidden state beyond the context's shared event sink.
//! - Module ordering is explicit in `default_modules()` — reorder by changing
//!   the list, not by editing the loop body.
//! - Modules can be selectively enabled/disabled by the caller (the engine
//!   accepts `&[&dyn EventModule]`).

pub mod context;
pub mod params;

pub use context::GenContext;
pub use params::TimelineParams;

use crate::entities::Pack;
use crate::timeline::Timeline;
use rand::SeedableRng;
use crate::event_engine::succession::SuccessionModule;
use crate::event_engine::war::WarModule;
use crate::event_engine::plague::PlagueModule;
use crate::event_engine::golden_age::GoldenAgeModule;
use crate::event_engine::schism::SchismModule;
use crate::event_engine::found_expand::FoundExpandModule;
use crate::event_engine::migration::MigrationModule;

// Re-export entity module types for external consumers.
pub mod found_expand;
pub mod war;
pub mod plague;
pub mod golden_age;
pub mod schism;
pub mod migration;
pub mod succession;

use rand::rngs::StdRng;

/// Trait for a deterministic event-generation module.
///
/// Each module is invoked once per year (for every `year` in `[era_start, era_end)`)
/// by the engine loop in [`generate_timeline`]. The module may emit zero or more
/// `Event`s into `ctx.events` and may mutate `ctx` (entity fields, cell arrays)
/// so that later modules in the same year see updated state.
///
/// Modules must be **pure** with respect to their external inputs: given the same
/// `GenContext` state (including the same RNG state at call time), the module
/// must produce identical events. The engine reseeds the RNG per year from the
/// timeline seed (see `generate_timeline`), so module determinism is preserved
/// regardless of how many events prior modules emitted in earlier years.
pub trait EventModule {
    /// Human-readable name for logging/debugging.
    fn name(&self) -> &'static str;

    /// Run this module for the given year.
    ///
    /// `ctx` is the shared mutable working context (Pack + cell arrays + event sink).
    /// `rng` is the per-year reseeded `StdRng`. `year` is the current in-universe year.
    fn run(&self, ctx: &mut GenContext, rng: &mut StdRng, year: i32);
}

/// The ordered list of event modules used by `generate_timeline`.
///
/// The order matches the plan's module dependency order:
/// found → war → plague → golden_age → schism → migration → succession.
/// Each module applies accepted events to the working world before later modules run.
pub fn default_modules() -> Vec<Box<dyn EventModule>> {
    vec![
        Box::new(FoundExpandModule),
        Box::new(WarModule),
        Box::new(PlagueModule),
        Box::new(GoldenAgeModule),
        Box::new(SchismModule),
        Box::new(MigrationModule),
        Box::new(SuccessionModule),
    ]
}

/// Generate a deterministic `Timeline` (sorted by `(year, id)`) from a year-0
/// `Pack` + cell ownership arrays + era bounds + seed.
///
/// This is the public entry point (mirrors `generate_timeline` in `lib.rs`).
/// It accepts the standard module set from [`default_modules`].
///
/// The `cells_*` arrays use the `i32` (`-1` = unassigned) / `i16` (`0` = none)
/// convention from `StatesResult` / `CulturesResult`. `cells_h` uses the
/// `u8` heightmap (FMG sea level = 20).
///
/// Returns a `Timeline` sorted by `(year, id)`. `narrative` is always `None`
/// (Phase 7 fills it in).
pub fn generate_timeline(
    pack: &Pack,
    cells_state: &[i32],
    cells_culture: &[i32],
    cells_religion: &[i32],
    cells_burg: &[i16],
    cells_h: &[u8],
    seed: u64,
    params: &TimelineParams,
) -> Timeline {
    generate_timeline_with_modules(
        pack,
        cells_state,
        cells_culture,
        cells_religion,
        cells_burg,
        cells_h,
        seed,
        params,
        &default_modules(),
    )
}

/// Generate a timeline using a custom set of modules (in the given order).
/// This allows callers to extend or reorder the engine without modifying the
/// core entry point.
pub fn generate_timeline_with_modules(
    pack: &Pack,
    cells_state: &[i32],
    cells_culture: &[i32],
    cells_religion: &[i32],
    cells_burg: &[i16],
    cells_h: &[u8],
    seed: u64,
    params: &TimelineParams,
    modules: &[Box<dyn EventModule>],
) -> Timeline {
    // Derive the engine's RNG seed. If `rng_override` is nonzero, use it;
    // otherwise derive a distinct stream from the timeline seed.
    let rng_seed = if params.rng_override != 0 {
        params.rng_override
    } else {
        seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(0x6A09E667)
    };

    // Normalize cell arrays: i32 (-1 = unassigned) → u32 (0 = unassigned),
    // i16 (0 = none) → u32 (0 = none).
    let n = cells_state.len();
    let cells_state_u32: Vec<u32> =
        cells_state.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();
    let cells_culture_u32: Vec<u32> =
        cells_culture.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();
    let cells_religion_u32: Vec<u32> =
        cells_religion.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();
    let cells_burg_u32: Vec<u32> =
        cells_burg.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();

    let cells_h_vec = if cells_h.len() == n {
        cells_h.to_vec()
    } else {
        vec![0u8; n]
    };

    let mut ctx = GenContext {
        pack: pack.clone(),
        cells_state: cells_state_u32,
        cells_culture: cells_culture_u32,
        cells_religion: cells_religion_u32,
        cells_burg: cells_burg_u32,
        era_start: params.era_start,
        era_end: params.era_end,
        params: params.clone(),
        events: Vec::new(),
        next_id: 1,
        cells_h: cells_h_vec,
    };

    // Iterate years in order. Each module gets a chance to fire per year, in
    // the order registered (found → war → plague → golden_age → schism →
    // migration → succession), matching the plan's dependency order.
    for year in ctx.era_start..ctx.era_end {
        // Re-derive the RNG state for this year from the seed so that the
        // per-year draw is deterministic and independent of the number of
        // events generated in prior years. This ensures that adding or
        // removing an event in year Y does not shift the RNG stream for
        // year Y+1 (a determinism hazard).
        let year_seed = rng_seed.wrapping_add((year as u64).wrapping_mul(0x100000003));
        let mut year_rng = StdRng::seed_from_u64(year_seed);

        for module in modules {
            module.run(&mut ctx, &mut year_rng, year);
        }
    }

    // Sort by (year, id) for the canonical order. Even though we generate
    // in year order, sorting defensively guarantees the contract.
    ctx.events.sort_by(|a, b| {
        a.year.cmp(&b.year).then(a.id.cmp(&b.id))
    });

    ctx.events
}

/// Inner (test-callable) version of `generate_timeline` that takes already-
/// normalized `u32` cell arrays (the WorldAt form). Avoids the serde boundary
/// in native cargo tests.
pub fn generate_timeline_inner(
    pack: &Pack,
    cells_state: &[u32],
    cells_culture: &[u32],
    cells_religion: &[u32],
    cells_burg: &[u32],
    cells_h: &[u8],
    seed: u64,
    params: &TimelineParams,
) -> Timeline {
    generate_timeline_with_modules_inner(
        pack,
        cells_state,
        cells_culture,
        cells_religion,
        cells_burg,
        cells_h,
        seed,
        params,
        &default_modules(),
    )
}

/// Inner version with custom modules (for test-callable use with reordered sets).
pub fn generate_timeline_with_modules_inner(
    pack: &Pack,
    cells_state: &[u32],
    cells_culture: &[u32],
    cells_religion: &[u32],
    cells_burg: &[u32],
    cells_h: &[u8],
    seed: u64,
    params: &TimelineParams,
    modules: &[Box<dyn EventModule>],
) -> Timeline {
    let n = cells_state.len();
    let cells_h_vec = if cells_h.len() == n {
        cells_h.to_vec()
    } else {
        vec![0u8; n]
    };

    let rng_seed = if params.rng_override != 0 {
        params.rng_override
    } else {
        seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(0x6A09E667)
    };

    let mut ctx = GenContext {
        pack: pack.clone(),
        cells_state: cells_state.to_vec(),
        cells_culture: cells_culture.to_vec(),
        cells_religion: cells_religion.to_vec(),
        cells_burg: cells_burg.to_vec(),
        era_start: params.era_start,
        era_end: params.era_end,
        params: params.clone(),
        events: Vec::new(),
        next_id: 1,
        cells_h: cells_h_vec,
    };

    for year in ctx.era_start..ctx.era_end {
        let year_seed = rng_seed.wrapping_add((year as u64).wrapping_mul(0x100000003));
        let mut year_rng = StdRng::seed_from_u64(year_seed);

        for module in modules {
            module.run(&mut ctx, &mut year_rng, year);
        }
    }

    ctx.events.sort_by(|a, b| {
        a.year.cmp(&b.year).then(a.id.cmp(&b.id))
    });

    ctx.events
}

// ---------------------------------------------------------------------------
// Tests — verification gate (plan §Step 4.2)
//
// These tests were extracted and adapted from the monolithic event_engine.rs.
// The full fixtures and gate criteria tests live here to keep them with the
// engine orchestration logic.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::{Burg, Culture, Religion, State};

    /// Build a minimal Pack with N states, for use in unit tests.
    fn make_pack(n_states: usize, n_cultures: usize, n_religions: usize, n_burgs: usize) -> Pack {
        let mut states = Vec::new();
        let mut burgs = Vec::new();
        let mut cultures = Vec::new();
        let mut religions = Vec::new();

        for i in 0..n_states {
            states.push(State {
                id: (i + 1) as u32,
                name: format!("State{}", i + 1),
                color: 0x4a6fa5 + (i as u32),
                capital: (i + 1) as u32,
                center_cell: (i * 100) as u32,
                form: "Monarchy".into(),
                tax_rate: 0.1,
                treasury: 5000.0,
                rural_pop: 10000.0 + (i as f64 * 1000.0),
                urban_pop: 5000.0 + (i as f64 * 500.0),
                military: 20 + (i as u32 * 10),
                founded_year: 0,
                dissolved_year: None,
                culture: 1,
            });
        }

        for i in 0..n_burgs {
            burgs.push(Burg {
                id: (i + 1) as u32,
                name: format!("Burg{}", i + 1),
                cell: (i * 50) as u32,
                state: if i < n_states { (i + 1) as u32 } else { 1 },
                culture: 1,
                religion: 1,
                population: 8.0,
                feature: 1,
                capital: if i < n_states { 1 } else { 0 },
                founded_year: 0,
                dissolved_year: None,
            });
        }

        for i in 0..n_cultures {
            cultures.push(Culture {
                id: (i + 1) as u32,
                name: format!("Culture{}", i + 1),
                color: 0xaa8844 + (i as u32),
                origin: (i * 200) as u32,
                type_code: 0,
                founded_year: 0,
                dissolved_year: None,
                cell_count: 500,
            });
        }

        for i in 0..n_religions {
            religions.push(Religion {
                id: (i + 1) as u32,
                name: format!("Religion{}", i + 1),
                color: 0xddccbb + (i as u32),
                center_cell: (i * 300) as u32,
                parent: None,
                followers: 50000.0 + (i as f64 * 10000.0),
                type_code: 0,
                expansion_mode: "global".into(),
                founded_year: 0,
                dissolved_year: None,
            });
        }

        Pack {
            states,
            provinces: Vec::new(),
            cultures,
            religions,
            burgs,
            armies: Vec::new(),
        }
    }

    /// Build cell arrays for `n` cells with `n_states` states owning them round-robin.
    fn make_cells(n: usize, n_states: usize) -> (Vec<i32>, Vec<i32>, Vec<i32>, Vec<i16>, Vec<u8>) {
        let cells_state: Vec<i32> = (0..n).map(|i| ((i % n_states) + 1) as i32).collect();
        let cells_culture: Vec<i32> = vec![1i32; n];
        let cells_religion: Vec<i32> = vec![1i32; n];
        let cells_burg: Vec<i16> = (0..n).map(|i| if i % 100 == 0 { 1i16 } else { 0i16 }).collect();
        let cells_h: Vec<u8> = vec![50u8; n]; // all land
        (cells_state, cells_culture, cells_religion, cells_burg, cells_h)
    }

    /// Generate a real Pack from a grid for integration tests.
    fn generate_real_pack(seed: u32, n: u32) -> (Pack, Vec<i32>, Vec<i32>, Vec<i32>, Vec<i16>, Vec<u8>) {
        let opts = crate::climate::ClimateOpts::default();
        let grid = crate::generate_world_inner(seed, n, &opts);

        let states_result = crate::gen_states::generate_states(&grid, seed, n.min(20) as u32);
        let suitability = crate::gen_states::compute_suitability(&grid);
        let cultures_result = crate::gen_cultures::generate_cultures_religions(
            &grid, seed, 5, 3, &suitability,
            &states_result.cells_state,
            &states_result.pack.burgs,
        );

        let mut pack = states_result.pack.clone();
        for c in &cultures_result.cultures {
            pack.cultures.push(c.clone());
        }
        for r in &cultures_result.religions {
            pack.religions.push(r.clone());
        }

        (
            pack,
            states_result.cells_state,
            cultures_result.cells_culture,
            cultures_result.cells_religion,
            states_result.cells_burg,
            grid.cells.h.clone(),
        )
    }

    // === M3 gate: determinism ===

    #[test]
    fn timeline_is_deterministic_same_seed() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 50, ..Default::default() };

        let t1 = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);
        let t2 = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        assert_eq!(t1, t2, "same seed must produce identical timeline");
    }

    #[test]
    fn timeline_differs_with_different_seed() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 50, ..Default::default() };

        let t1 = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);
        let t2 = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 43, &params);

        assert_ne!(t1, t2, "different seed should produce different timeline");
    }

    #[test]
    fn timeline_differs_with_different_era_bounds() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let p1 = TimelineParams { era_start: 0, era_end: 50, ..Default::default() };
        let p2 = TimelineParams { era_start: 0, era_end: 100, ..Default::default() };

        let t1 = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &p1);
        let t2 = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &p2);

        assert!(t2.len() >= t1.len(), "longer era should produce >= events");
    }

    // === M3 gate: non-empty ===

    #[test]
    fn timeline_is_non_empty_for_default_world() {
        let pack = make_pack(8, 4, 3, 8);
        let (cs, cc, cr, cb, ch) = make_cells(100, 8);
        let params = TimelineParams { era_start: 0, era_end: 200, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);
        assert!(!timeline.is_empty(), "default world must produce a non-empty timeline");
    }

    // === Event ID uniqueness + sorting ===

    #[test]
    fn all_event_ids_are_unique() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 100, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let mut ids: Vec<u64> = timeline.iter().map(|e| e.id).collect();
        let original = ids.clone();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), original.len(), "event ids must be unique");
    }

    #[test]
    fn timeline_is_sorted_by_year_then_id() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 100, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        for window in timeline.windows(2) {
            let a = &window[0];
            let b = &window[1];
            let ord = a.year.cmp(&b.year).then(a.id.cmp(&b.id));
            assert!(
                ord.is_lt(),
                "timeline not sorted: event {} (year={}, id={}) comes before (year={}, id={})",
                a.id, a.year, a.id, b.year, b.id
            );
        }
    }

    // === Year range ===

    #[test]
    fn all_events_within_era_bounds() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 50, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        for ev in &timeline {
            assert!(
                ev.year >= params.era_start && ev.year < params.era_end,
                "event {} year {} outside era [{}, {})",
                ev.id, ev.year, params.era_start, params.era_end
            );
        }
    }

    // === No missing entity references ===

    #[test]
    fn no_event_references_missing_entity() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 100, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        assert!(!timeline.is_empty(), "need events to check");

        // Project the timeline forward to the end year and verify the
        // projection succeeds (no panics from missing entities).
        let w = crate::timeline::project_world(&pack, &cs, &cc, &cr, &cb, &timeline, params.era_end - 1);

        // Verify projected cells only reference valid state ids (0 = unassigned,
        // or ids that exist in the base pack).
        let max_base_state: u32 = pack.states.iter().map(|s| s.id).max().unwrap_or(0);
        for &s in &w.cells_state {
            assert!(s == 0 || s <= max_base_state + 1000, "projected state id {} out of range", s);
        }
    }

    // === Narrative is always None ===

    #[test]
    fn all_narratives_are_none() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 100, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        for ev in &timeline {
            assert!(ev.narrative.is_none(), "event {} narrative must be None (Phase 7 sets it)", ev.id);
        }
    }

    // === Required event types appear when preconditions exist ===

    #[test]
    fn produces_found_events_when_states_exist() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 200, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let has_found = timeline.iter().any(|e| e.kind == crate::timeline::EventKind::Found);
        assert!(has_found, "should produce Found events when states exist");
    }

    #[test]
    fn produces_war_events_when_multiple_states() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 200, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let has_war = timeline.iter().any(|e| e.kind == crate::timeline::EventKind::War);
        assert!(has_war, "should produce War events when multiple states exist");
    }

    #[test]
    fn produces_schism_events_when_religions_exist() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 200, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let has_schism = timeline.iter().any(|e| e.kind == crate::timeline::EventKind::Schism);
        assert!(has_schism, "should produce Schism events when religions exist");
    }

    #[test]
    fn produces_plague_events_with_sufficient_pop() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 200, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let has_plague = timeline.iter().any(|e| e.kind == crate::timeline::EventKind::Plague);
        assert!(has_plague, "should produce Plague events with sufficient population");
    }

    #[test]
    fn produces_golden_age_events() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 200, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let has_golden_age = timeline.iter().any(|e| e.kind == crate::timeline::EventKind::GoldenAge);
        assert!(has_golden_age, "should produce GoldenAge events");
    }

    #[test]
    fn produces_succession_events_for_aged_states() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 200, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let has_succession = timeline.iter().any(|e| e.kind == crate::timeline::EventKind::Succession);
        assert!(has_succession, "should produce Succession events for aged states");
    }

    // === Event rate bounds ===

    #[test]
    fn event_count_within_bounds_for_small_world() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 100, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        // 5 states * 100 years * 7 modules * ~0.05 avg rate = ~35 max events.
        // Allow generous slack: < 500 events for a small test world.
        assert!(timeline.len() < 500, "event count {} exceeds sanity bound for 100-cell world", timeline.len());
    }

    // === Projection round-trip: events must be projectable ===

    #[test]
    fn generated_timeline_projects_cleanly() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 50, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let w0 = crate::timeline::project_world(&pack, &cs, &cc, &cr, &cb, &timeline, params.era_end - 1);
        assert_eq!(w0.year, params.era_end - 1);
        assert_eq!(w0.cells_state.len(), 100);
    }

    // === Inner (test-callable) version mirrors the public API ===

    #[test]
    fn inner_matches_public_api() {
        let pack = make_pack(3, 2, 1, 3);
        let (cs, cc, cr, cb, ch) = make_cells(50, 3);
        let params = TimelineParams { era_start: 0, era_end: 30, ..Default::default() };

        let cs_u32: Vec<u32> = cs.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();
        let cc_u32: Vec<u32> = cc.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();
        let cr_u32: Vec<u32> = cr.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();
        let cb_u32: Vec<u32> = cb.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();

        let t1 = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);
        let t2 = generate_timeline_inner(&pack, &cs_u32, &cc_u32, &cr_u32, &cb_u32, &ch, 42, &params);

        assert_eq!(t1, t2, "inner must match public API output");
    }

    // === Empty pack produces empty timeline ===

    #[test]
    fn empty_pack_produces_empty_timeline() {
        let pack = Pack::default();
        let cs = vec![-1i32; 10];
        let cc = vec![-1i32; 10];
        let cr = vec![-1i32; 10];
        let cb = vec![0i16; 10];
        let ch = vec![50u8; 10];
        let params = TimelineParams { era_start: 0, era_end: 50, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);
        assert!(timeline.is_empty(), "empty pack should produce no events");
    }

    // === Integration: real generated world ===

    #[test]
    fn timeline_from_real_generated_world() {
        let (pack, cs, cc, cr, cb, ch) = generate_real_pack(42, 500);
        let params = TimelineParams {
            era_start: 0,
            era_end: 100,
            ..Default::default()
        };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);
        assert!(!timeline.is_empty(), "real world should produce events");

        for window in timeline.windows(2) {
            let a = &window[0];
            let b = &window[1];
            let ord = a.year.cmp(&b.year).then(a.id.cmp(&b.id));
            assert!(ord.is_lt(), "timeline not sorted in real-world test");
        }

        for ev in &timeline {
            assert!(ev.year >= 0 && ev.year < 100);
        }
    }

    // === Event rate bounds for real world ===

    #[test]
    fn real_world_event_rate_within_bounds() {
        let (pack, cs, cc, cr, cb, ch) = generate_real_pack(42, 200);
        let params = TimelineParams {
            era_start: 0,
            era_end: 100,
            ..Default::default()
        };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        // At 200 cells with ~10 states over 100 years, we expect maybe
        // 50-150 events. Assert < 1000 as a generous upper bound.
        assert!(timeline.len() < 1000, "event count {} exceeds bound for 200-cell real world", timeline.len());
    }
}

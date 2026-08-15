//! Phase 3 Step 3.1 — Entity data model.
//!
//! Defines the anthropological-layer entity types (`State`, `Province`,
//! `Culture`, `Religion`, `Burg`, `Army`) and the `Pack` holder that
//! aggregates them at the "year 0" anchor (design doc §3.2,
//! tech-requirements §3.2).
//!
//! This module is **pure data + serde**: no generators live here (Step 3.2
//! adds `gen_states.rs`, Step 3.3 adds `gen_cultures.rs`/`gen_religions.rs`),
//! and no rendering or RNG. Each entity carries a `founded_year` and an
//! optional `dissolved_year` so the Phase 4 timeline projector can derive
//! "the world at year Y" from events ≤ Y applied to the base `Pack`
//! (design §3.4). The legacy repair cascade (`crate::lib::repair_entities`,
//! `recompute_dependents`) writes placeholder `Burg@cellN` names because no
//! `pack.burgs` records existed; once Phase 3.2/3.3 populate a real `Pack`,
//! those code paths can read real names from here.
//!
//! Field naming matches `agent/worldgen-technical-requirements.md` §3.2 (the
//! `State`/`Religion` sketches there) and the FMG `pack.states` /
//! `pack.burgs` shapes the design doc cites; TS mirrors in
//! `app/src/state/types.ts` carry identical field names with `number` for the
//! integer types (tech-requirements §3.2). The struct is the wire contract
//! for the worker messages Phase 4 introduces (`projectWorld` /
//! `projectDelta` / `generateTimeline`); adding a Pack-level field requires
//! mirroring it on the TS side. NOTE: `Pack` and its entity structs cross the
//! Phase-4 `projectWorld`/`generateTimeline` boundary — NOT the Phase-2.5
//! `spliceDependentResult` per-cell helper (api.ts), which only ever touches
//! the 11 `cells.*` index arrays + river/lake geometry. Do not add Pack
//! fields to `spliceDependentResult`; that helper never sees `pack.*`.
//!
//! TODO(Route): design §3.2 lists `Route` among the seven base entities; it
//! is intentionally NOT modeled here (MVP out-of-scope per §11 — Routes/
//! markets/military-economy trade sim is stretch post-MVP). Add the struct
//! + TS mirror when the trade-sim stretch goal is taken on.
//!
//! Type-width note (review F1): `Pack.burgs[i].id` is `u32`, but the per-cell
//! burg index `grid.cells.burg` is `Vec<i16>` (grid.rs). The Phase-3.2
//! generator writes `Burg.id` into `cells.burg[cell]`; the Phase-4 timeline
//! projector joins on `cells.burg[cell] == pack.burgs[id-1].id`. `i16` caps
//! ids at 32 767 (safe under FMG town counts at 60k cells, but a narrowing
//! cast). When `gen_states.rs` lands, either widen `CellData.burg` to `i32`
//! (matching `state`/`province`/`culture`/`religion`) so the join is
//! identity, or clamp + `debug_assert!(id <= i16::MAX as u32)` here or at
//! the write site. Do not leave the `u32`→`i16` join undocumented.

// The structs below are deliberately not constructed outside of `#[cfg(test)]`
// yet — Phase 3.2 (`gen_states.rs`) and 3.3 (`gen_cultures.rs` /
// `gen_religions.rs`) are the first production constructors. Silence the
// dead-code warnings for the entity types (mirrors the `biomes.rs`
// `#[allow(dead_code)]` on `BiomeDef`); they will be removed once Phase 3.2
// wires `State`/`Burg` into `lib.rs`.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------//
// Pack — base entities at year 0 (design §3.2)
// ---------------------------------------------------------------------------//

/// The base `Pack` of anthropological entities at the year-0 anchor. Mirrors
/// `agent/worldgen-technical-requirements.md` §3.2.
///
/// All six lists are appended-to as Phase 3 generators run; Phase 4's event
/// engine may add **child** religions via `Schism` events and new states via
/// `Secession`/`Found`, but the engine writes those on a *working copy* of
/// the `Pack` produced by the timeline projector — the base `Pack` is the
/// immutable, generated-once year-0 truth that the `.world` archive
/// serializes (Phase 8).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Pack {
    pub states: Vec<State>,
    pub provinces: Vec<Province>,
    pub cultures: Vec<Culture>,
    pub religions: Vec<Religion>,
    pub burgs: Vec<Burg>,
    pub armies: Vec<Army>,
}

// ---------------------------------------------------------------------------//
// State — sovereign polity (FMG `pack.states[i]`)
// ---------------------------------------------------------------------------//

/// A state. Mirrors FMG `pack.states[i]`: identifier, name, color (Packed RGB
/// `0xRRGGBB`, same encoding the renderer writes into the per-cell data
/// texture), capital `Burg` id, foundational/dissolution spans, and base
/// economic/military attributes the Phase 4 war + golden_age modules read.
///
/// `dissolved_year == None` means "still extant" — the timeline projector
/// flips this to `Some(Y)` on a `Conquer` that strips a state of all cells
/// (a state with no cell-set after projection is treated as dissolved at the
/// year of its last lost cell).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct State {
    /// Stable 1-based id (index in `Pack.states`).
    pub id: u32,
    pub name: String,
    /// Packed RGB color `0xRRGGBB` for the renderer's data-texture fill.
    pub color: u32,
    /// Capital `Burg` id (0 = none assigned).
    pub capital: u32,
    /// The cell id the capital sits on (the seed cell for state expansion).
    pub center_cell: u32,
    /// Government form / type ("Monarchy" / "Republic" / "Theocracy" /
    /// "Union" / "Anarchy" / ... — FMG `defineStateForms`). Drives the
    /// `tax_rate` the Phase 4 economy heir will read; Phase 3.2 assigns it.
    pub form: String,
    /// Per-state tax multiplier (FMG `DEFAULT_TAX_BY_FORM`). Used by the
    /// economy module once Phase 4 wires it; Phase 3.2 pins it from the form.
    pub tax_rate: f64,
    /// Current treasury at year-0 (Phase 4 simulates drift over time).
    pub treasury: f64,
    /// Summed rural population (cells × area) at year-0. FMG
    /// `States.collectStatistics` aggregates this; Phase 3.2 computes it.
    pub rural_pop: f64,
    /// Summed urban population across the state's burgs at year-0.
    pub urban_pop: f64,
    /// Aggregated military strength at year-0 (drives Phase 4 `War` outcomes).
    pub military: u32,
    /// Year the state was founded (0 if it predates the active era). Negative
    /// years are allowed (in-universe years can be negative, design §3.3).
    pub founded_year: i32,
    /// `Some(Y)` the year the state dissolved (lost all cells / was
    /// conquered); `None` means extant.
    pub dissolved_year: Option<i32>,
    /// The originating `Culture` id (FMG `culture` on a state). Used by the
    /// Phase 4 religion + schism spread models and by name generators.
    pub culture: u32,
}

// ---------------------------------------------------------------------------//
// Province — a subdivision of a State (FMG `pack.provinces`)
// ---------------------------------------------------------------------------//

/// A province — a subdivision of a `State`, mirroring FMG `pack.provinces[i]`.
/// Phase 3.2 subdivides each state into provinces after frontier expansion.
/// `dissolved_year == None` means still part of the owning state at the year
/// projector's current step.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Province {
    /// Stable 1-based id (index in `Pack.provinces`).
    pub id: u32,
    /// Owning `State` id; -1 (`0` after unassigned init) if orphaned.
    pub state: u32,
    pub name: String,
    /// Packed RGB color `0xRRGGBB`.
    pub color: u32,
    /// The seed cell the province grew from.
    pub center_cell: u32,
    /// Rural population summed across the province's cells at year-0.
    pub rural_pop: f64,
    /// Urban population summed across the province's burgs at year-0.
    pub urban_pop: f64,
    pub founded_year: i32,
    pub dissolved_year: Option<i32>,
}

// ---------------------------------------------------------------------------//
// Culture — diffusion-from-seed populations (FMG `pack.cultures`)
// ---------------------------------------------------------------------------//

/// A culture. Phase 3.3 seeds cultures from burg density and expands them
/// via cellular-automata-style diffusion over land cells. `cells.culture`
/// on the `Grid` records the per-cell owning culture id; this struct carries
/// the per-culture metadata. `origin` is the originating cell; `type_`
/// mirrors FMG's "navigation culture vs. navigation culture vs. highland
/// vs. river vs. lake" categories used in the name generator and expansion
/// rules (numeric code 0..=4).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Culture {
    /// Stable 1-based id (index in `Pack.cultures`).
    pub id: u32,
    pub name: String,
    /// Packed RGB color `0xRRGGBB` for the renderer's culture-layer fill.
    pub color: u32,
    /// The seed cell the culture originated on.
    pub origin: u32,
    /// FMG culture type code (0 = navigation, 1 = highland, 2 = river,
    /// 3 = lake, 4 = nomadic). Drives the name generator's syllable pool
    /// and the Phase 3.3 expansion bias.
    pub type_code: u8,
    pub founded_year: i32,
    pub dissolved_year: Option<i32>,
    /// Number of cells assigned to this culture at year-0 (populated by
    /// Phase 3.3 expansion).
    pub cell_count: u32,
}

// ---------------------------------------------------------------------------//
// Religion — belief systems spread from converted burgs (FMG `pack.religions`)
// ---------------------------------------------------------------------------//

/// A religion. Phase 3.3 spreads religions from converted burgs (analogous
/// to culture expansion). `parent` makes this the **schism tree** node: a
/// `Schism` event (Phase 4) spawns a *new* `Religion` with `parent = Some(
/// parent_id)` and a seeded `follower_fraction` of the parent's followers
/// reassigned to the child. `followers` is the year-0 count (sum of the
/// religion's burg populations).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Religion {
    /// Stable 1-based id (index in `Pack.religions`).
    pub id: u32,
    pub name: String,
    /// Packed RGB color `0xRRGGBB`.
    pub color: u32,
    /// Originating cell the religion was first preached from.
    pub center_cell: u32,
    /// `None` for an original "root" religion; `Some(parent_id)` for a
    /// denomination split off by a Phase 4 `Schism` event. The renderer
    /// uses this to draw the schism tree (plan Step 6.2).
    pub parent: Option<u32>,
    /// Number of followers at year-0 (sum of the religion's burg pops).
    pub followers: f64,
    /// FMG religion type code (0 = aggregation, matching the FMG origin
    /// categories used in expansion).
    pub type_code: u8,
    pub founded_year: i32,
    pub dissolved_year: Option<i32>,
}

// ---------------------------------------------------------------------------//
// Burg — a settlement (capital / town / city) (FMG `pack.burgs[i]`)
// ---------------------------------------------------------------------------//

/// A burg — a settlement on a single cell. Phase 3.2 seeds burgs (capitals +
/// towns) from population + biome suitability. `cell` is the on-grid cell id
/// the burg sits on; `state` is its owning `State` id; `culture` and
/// `religion` track its belief/ethnic identity (used by Phase 3.3 expansion
/// seeds and Phase 4 `Schism` spread).
///
/// `pop` is in thousands (FMG convention); the renderer scales the burg
/// marker by this. The `repair_entities` cascade (Step 2.5.4) currently emits
/// a placeholder `"Burg@cellN"` string when a land→water flip removes a burg;
/// Phase 3.2 will let it read the real `name` from here.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Burg {
    /// Stable 1-based id (index in `Pack.burgs`).
    pub id: u32,
    pub name: String,
    /// The cell id the burg sits on (a land cell). The repair cascade
    /// removes a burg when this cell flips land→water.
    pub cell: u32,
    /// Owning `State` id (0 = unowned); FMG `state`.
    pub state: u32,
    /// Owning `Culture` id (0 = unassigned); FMG `culture`.
    pub culture: u32,
    /// Owning `Religion` id (0 = unassigned); FMG `religion`.
    pub religion: u32,
    /// Population in *thousands* (FMG convention). Renderer scales the
    /// burg marker; Phase 4 plagues scale it down, golden ages scale it up.
    pub population: f64,
    /// FMG feature flag (0 = off-map pour, nonzero = live burg).
    pub feature: u32,
    /// Capital flag (FMG `capital`); 1 = this burg is its state's capital.
    pub capital: u8,
    pub founded_year: i32,
    pub dissolved_year: Option<i32>,
}

// ---------------------------------------------------------------------------//
// Army — a military unit (FMG `pack.markers`, simplified for MVP)
// ---------------------------------------------------------------------------//

/// An army / military unit. FMG models these in `pack.markers`; the MVP
/// keeps a minimal record of size, owning state, and current cell so the
/// Phase 4 `Raise`/`March`/`Battle`/`Disband` events can move and resize the
/// marker, and the Phase 5 renderer can place them as point sprites. Phase 3
/// does *not* generate armies (they are created at `Raise` events); the
/// year-0 `Pack.armies` is conventionally empty.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Army {
    /// Stable 1-based id (assigned by the Phase 4 event engine when raised).
    pub id: u32,
    /// Owning `State` id.
    pub state: u32,
    /// Current cell the army is deployed on (a land cell).
    pub cell: u32,
    /// Unit size (headcount; FMG uses `i` for the strength). Phase 4 `Battle`
    /// events subtract casualties from here.
    pub size: u32,
    /// Composition tag (FMG "infantry" / "cavalry" / "navy" — Phase 4 use).
    pub kind: String,
    pub founded_year: i32,
    pub dissolved_year: Option<i32>,
}

// ===========================================================================//
// Tests — serde round-trip + founded/dissolved invariants.
// ===========================================================================//

#[cfg(test)]
mod tests {
    #![allow(clippy::needless_range_loop)]
    use super::*;

    /// An empty `Pack` (the year-0 initialization state before any generator
    /// runs) round-trips cleanly through `serde_json`. This is the simplest
    /// contract: PHASE 3.2/3.3 start from a `Pack::default()` and append
    /// entities; the byte-identical reload guarantee (design §7, tech-req §4)
    /// starts here.
    #[test]
    fn pack_empty_round_trips_through_serde_json() {
        let pack = Pack::default();
        let json = serde_json::to_string(&pack).expect("serialize");
        let back: Pack = serde_json::from_str(&json).expect("deserialize");
        assert!(back.states.is_empty());
        assert!(back.provinces.is_empty());
        assert!(back.cultures.is_empty());
        assert!(back.religions.is_empty());
        assert!(back.burgs.is_empty());
        assert!(back.armies.is_empty());
        // Round-trip twice (serialize the deserialized instance) and assert
        // byte-identity — pins the "reload identically" property before any
        // generation logic lands.
        let json2 = serde_json::to_string(&back).expect("serialize2");
        assert_eq!(json, json2, "round-trip not stable");
    }

    /// A populated `Pack` with one of each entity type round-trips through
    /// `serde_json` and preserves every field. Phase 3.2 will essentially
    /// mutate fields on the populated `Pack` the generator builds, so
    /// the round-trip must not silently drop or default-fill any field.
    #[test]
    fn pack_populated_round_trips_preserving_every_field() {
        let pack = sample_pack();
        let json = serde_json::to_string(&pack).expect("serialize");
        let back: Pack = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.states.len(), 1);
        let s = &back.states[0];
        let orig = &pack.states[0];
        assert_eq!(s.id, orig.id);
        assert_eq!(s.name, orig.name);
        assert_eq!(s.color, orig.color);
        assert_eq!(s.capital, orig.capital);
        assert_eq!(s.center_cell, orig.center_cell);
        assert_eq!(s.form, orig.form);
        assert_eq!(s.tax_rate.to_bits(), orig.tax_rate.to_bits());
        assert_eq!(s.treasury.to_bits(), orig.treasury.to_bits());
        assert_eq!(s.rural_pop.to_bits(), orig.rural_pop.to_bits());
        assert_eq!(s.urban_pop.to_bits(), orig.urban_pop.to_bits());
        assert_eq!(s.military, orig.military);
        assert_eq!(s.founded_year, orig.founded_year);
        assert_eq!(s.dissolved_year, orig.dissolved_year);
        assert_eq!(s.culture, orig.culture);

        assert_eq!(back.provinces.len(), 1);
        assert_eq!(back.provinces[0].id, pack.provinces[0].id);
        assert_eq!(back.provinces[0].state, pack.provinces[0].state);
        assert_eq!(back.provinces[0].name, pack.provinces[0].name);
        assert_eq!(back.provinces[0].center_cell, pack.provinces[0].center_cell);

        assert_eq!(back.cultures.len(), 1);
        assert_eq!(back.cultures[0].type_code, pack.cultures[0].type_code);
        assert_eq!(back.cultures[0].origin, pack.cultures[0].origin);
        assert_eq!(back.cultures[0].cell_count, pack.cultures[0].cell_count);

        assert_eq!(back.religions.len(), 1);
        assert_eq!(back.religions[0].parent, pack.religions[0].parent);
        assert_eq!(back.religions[0].followers.to_bits(),
                   pack.religions[0].followers.to_bits());
        assert_eq!(back.religions[0].type_code, pack.religions[0].type_code);

        assert_eq!(back.burgs.len(), 1);
        assert_eq!(back.burgs[0].cell, pack.burgs[0].cell);
        assert_eq!(back.burgs[0].capital, pack.burgs[0].capital);
        assert_eq!(back.burgs[0].feature, pack.burgs[0].feature);
        assert_eq!(back.burgs[0].population.to_bits(),
                   pack.burgs[0].population.to_bits());

        assert_eq!(back.armies.len(), 1);
        assert_eq!(back.armies[0].size, pack.armies[0].size);
        assert_eq!(back.armies[0].cell, pack.armies[0].cell);
        assert_eq!(back.armies[0].kind, pack.armies[0].kind);
    }

    /// The `dissolved_year` span invariant: every entity type carries it,
    /// `None` means "extant" and `Some(y)` means dissolved. The Phase 4
    /// timeline projector will surface entities only when `founded_year
    /// <= year < dissolved_year_or_inf`. Pin all six entity types carry the
    /// `founded_year` / `dissolved_year` span with the documented convention
    /// so a future refactor that drops it breaks the build, not the timeline.
    #[test]
    fn all_entity_types_carry_founded_dissolved_span() {
        let pack = sample_pack();

        // State
        assert_eq!(pack.states[0].founded_year, 0);
        assert!(pack.states[0].dissolved_year.is_none());

        // Province — founded at year 0, still part of its state (None).
        assert_eq!(pack.provinces[0].founded_year, 0);
        assert!(pack.provinces[0].dissolved_year.is_none());

        // Culture
        assert_eq!(pack.cultures[0].founded_year, 0);
        assert!(pack.cultures[0].dissolved_year.is_none());

        // Religion
        assert_eq!(pack.religions[0].founded_year, 0);
        assert!(pack.religions[0].dissolved_year.is_none());

        // Burg
        assert_eq!(pack.burgs[0].founded_year, 12);
        assert!(pack.burgs[0].dissolved_year.is_none());

        // Army — raised (founded) at year 30, still active.
        assert_eq!(pack.armies[0].founded_year, 30);
        assert!(pack.armies[0].dissolved_year.is_none());

        // A dissolved religion (schism child where the parent later merg...)
        // round-trips with the span set.
        let mut dissolved = sample_pack();
        dissolved.religions[0].dissolved_year = Some(732);
        let json = serde_json::to_string(&dissolved).unwrap();
        let back: Pack = serde_json::from_str(&json).unwrap();
        assert_eq!(back.religions[0].dissolved_year, Some(732));
    }

    /// Negative in-universe years survive the round-trip (design §3.3 says
    /// "in-universe year (can be negative)"). Pin it explicitly so a future
    /// change to `u32` or a `#serde` rename doesn't silently break it.
    #[test]
    fn founded_year_can_be_negative_and_round_trips() {
        let mut pack = sample_pack();
        pack.states[0].founded_year = -800;
        pack.cultures[0].founded_year = -1200;
        let json = serde_json::to_string(&pack).unwrap();
        let back: Pack = serde_json::from_str(&json).unwrap();
        assert_eq!(back.states[0].founded_year, -800);
        assert_eq!(back.cultures[0].founded_year, -1200);
    }

    /// The renderer's data-texture fill reads `color` as a packed `0xRRGGBB`
    /// `u32`. Pin that a 24-bit color survives round-trip without sign
    /// extension or truncation (so the renderer's `gl.clearColor` and the
    /// per-cell texel write are byte-faithful to the generator's choice).
    #[test]
    fn color_field_round_trips_as_24bit_packed_rgb() {
        let mut pack = sample_pack();
        pack.states[0].color = 0x123456;
        pack.cultures[0].color = 0xFFFFFF;
        pack.religions[0].color = 0x000001;
        let json = serde_json::to_string(&pack).unwrap();
        let back: Pack = serde_json::from_str(&json).unwrap();
        assert_eq!(back.states[0].color, 0x123456);
        assert_eq!(back.cultures[0].color, 0xFFFFFF);
        assert_eq!(back.religions[0].color, 0x000001);
    }

    /// A `Pack` mirrors a simple aggregator: appending one more `State` does
    /// not disturb the encoding of the others (entities are independent list
    /// rows — Phase 4's timeline projector appends rows, so this is the
    /// invariant the projector's "add child religion" / "secede new state"
    /// relies on).
    #[test]
    fn appending_an_entity_row_is_independent_of_existing_rows() {
        let mut pack = sample_pack();
        let orig_state0 = pack.states[0].clone();
        pack.states.push(State {
            id: 2,
            name: "Easthold".into(),
            color: 0x4488cc,
            capital: 3,
            center_cell: 7777,
            form: "Republic".into(),
            tax_rate: 0.10,
            treasury: 0.0,
            rural_pop: 0.0,
            urban_pop: 0.0,
            military: 40,
            founded_year: 1,
            dissolved_year: None,
            culture: 2,
        });
        let after = serde_json::to_string(&pack).unwrap();
        let back: Pack = serde_json::from_str(&after).unwrap();
        // The first state is unchanged.
        assert_eq!(back.states[0].id, orig_state0.id);
        assert_eq!(back.states[0].name, orig_state0.name);
        assert_eq!(back.states[0].color, orig_state0.color);
        assert_eq!(back.states[0].center_cell, orig_state0.center_cell);
        // The newly-appended second state decodes intact.
        assert_eq!(back.states.len(), 2);
        assert_eq!(back.states[1].id, 2);
        assert_eq!(back.states[1].name, "Easthold");
        assert_eq!(back.states[1].color, 0x4488cc);
        // The standalone JSON serialization of the original first state should
        // appear verbatim as a substring of `after` — the existing row's
        // encoding was NOT perturbed by appending a sibling (serde_json sorts
        // struct fields in declaration order, so a clean substring match
        // pins that the encoder treats each list row independently).
        let orig_state0_json = serde_json::to_string(&orig_state0).unwrap();
        assert!(
            after.contains(&orig_state0_json),
            "appending a row perturbed the existing row's encoding: \
             original = {orig_state0_json}, got = {after}"
        );
    }

    /// Helper: build a small but fully-populated `Pack` with one entity of
    /// each type and a mix of populated/None fields. Used by every test in
    /// this module — keep it in sync with the struct field set so a missing
    /// field on any entity surfaces as a compile error here.
    fn sample_pack() -> Pack {
        Pack {
            states: vec![State {
                id: 1,
                name: "Arvendel".into(),
                color: 0x4a6fa5,
                capital: 1,
                center_cell: 1234,
                form: "Monarchy".into(),
                tax_rate: 0.12,
                treasury: 5000.0,
                rural_pop: 12000.0,
                urban_pop: 8400.0,
                military: 320,
                founded_year: 0,
                dissolved_year: None,
                culture: 1,
            }],
            provinces: vec![Province {
                id: 1,
                state: 1,
                name: "Arvendel Heartland".into(),
                color: 0x5b7bb0,
                center_cell: 1234,
                rural_pop: 4000.0,
                urban_pop: 4200.0,
                founded_year: 0,
                dissolved_year: None,
            }],
            cultures: vec![Culture {
                id: 1,
                name: "Northern Folk".into(),
                color: 0xAA8844,
                origin: 1234,
                type_code: 1, // highland
                founded_year: 0,
                dissolved_year: None,
                cell_count: 612,
            }],
            religions: vec![Religion {
                id: 1,
                name: "Old Faith".into(),
                color: 0xddccbb,
                center_cell: 1234,
                parent: None,
                followers: 4200.0,
                type_code: 0,
                founded_year: 0,
                dissolved_year: None,
            }],
            burgs: vec![Burg {
                id: 1,
                name: "Arvendel City".into(),
                cell: 1234,
                state: 1,
                culture: 1,
                religion: 1,
                population: 4.2, // thousands
                feature: 1,
                capital: 1,
                founded_year: 12,
                dissolved_year: None,
            }],
            armies: vec![Army {
                id: 1,
                state: 1,
                cell: 1300,
                size: 2000,
                kind: "infantry".into(),
                founded_year: 30,
                dissolved_year: None,
            }],
        }
    }
}

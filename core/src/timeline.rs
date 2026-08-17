//! Phase 4 Step 4.1 — Timeline data model + `WorldAt(year)` projector.
//!
//! The event-centric representation that lets the year scrubber derive entity/cell
//! state at any year Y from the year-0 `Pack` plus a chronologically-ordered
//! `Event[]` (design §3.3, §3.4). Projection is O(events ≤ Y) and cheap, so the
//! React scrubber (Phase 5) gets a `WorldAt` every frame at 60fps.
//!
//! See `agent/worldgen-implementation-plan.md` §Step 4.1 for the gate criteria.

use serde::{Deserialize, Serialize};

use crate::entities::{Pack, State, Culture, Religion, Burg, Army};

// ---------------------------------------------------------------------------//
// EntityType — which kind of entity an `Event` targets.
// ---------------------------------------------------------------------------//

/// Which entity type an `Event` addresses (design §3.3 `entity_type`).
/// Mirrors the TS `EntityType` union in `app/src/state/types.ts` (Step 3.1).
/// New variants are forward-compatible: unknown variants are skipped by the
/// projector (never panicked on).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub enum EntityType {
    State,
    Province,
    Culture,
    Religion,
    Burg,
    Army,
    /// Aggregate demographic / population bucket — used by `Plague`,
    /// `Migration`, `GoldenAge` events that scale pops without attaching
    /// to a single burg.
    Pop,
}

// ---------------------------------------------------------------------------//
// EventKind — the taxonomy of historical events (design §3.3 / §4.1 table).
// ---------------------------------------------------------------------------//

/// The kind of historical event (design §3.3 `kind`).
/// Mirrors design §4.1 rule-module table: `succession`, `war`, `plague`,
/// `golden_age`, `schism`, `found_expand`, `migration`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum EventKind {
    // --- found / expand family (design §4.1 `found_expand`) ---
    Found,
    Conquer,
    Secession,

    // --- succession family (design §4.1 `succession`) ---
    Succession,
    CivilWar,

    // --- war family (design §4.1 `war`) ---
    War,
    Battle,
    Treaty,

    // --- demography ---
    Plague,
    GoldenAge,
    Migrate,

    // --- religion ---
    Schism,

    // --- military ---
    Raise,
    March,
    Disband,

    // --- settlement lifecycle ---
    Raze,

    // --- dissolutions ---
    Dissolve,
}

// ---------------------------------------------------------------------------//
// EventPayload — structured, type-specific data for an `Event`.
// ---------------------------------------------------------------------------//

/// Outcome payload for a `War` event (design §4.1 `war` module).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct WarOutcome {
    /// `0` = attacker wins, `1` = defender wins, `2` = stalemate (treaty).
    pub result: u8,
    /// Casualties applied to the loser's army / pops (fractional 0..1).
    pub attrition: f64,
    /// The conquered `cell` ids (attacker-win path). Empty for other outcomes.
    pub conquered_cells: Vec<u32>,
}

/// Payload for a `Schism` event: how many of the parent `Religion`'s followers
/// jump to the newly-spawned child denomination.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SchismPayload {
    /// Follower fraction reassigned to the child religion at split time (0..1).
    pub follower_fraction: f64,
    /// The new `Religion` id spawned by this schism.
    pub child_religion_id: u32,
}

/// Structured payload for a `Conquer` event: the cells reassigned from the
/// loser to the winner.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ConquerPayload {
    /// Cell ids that flip ownership to the winning state.
    pub cells: Vec<u32>,
}

/// Payload for a `Migrate` event: a culture/religion spreads across cells.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct MigratePayload {
    /// Cell ids that switch to the migrant culture / religion.
    pub cells: Vec<u32>,
    /// Target culture_id (for `Migrate` with `entity_type == Culture`) or
    /// religion_id (for `entity_type == Religion`).
    pub target_id: u32,
}

/// Type-specific structured data carried by an `Event` (design §3.3 `payload`).
/// The projector interprets the variant that matches `EventKind`. Unknown
/// payloads are ignored gracefully (`Unknown` fallback).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", content = "data")]
pub enum EventPayload {
    None,
    /// `Found`/`Raze`: the cell id.
    Found { cell: u32 },
    /// `Succession`/`CivilWar`: the heir's name (optional narrative seed).
    Succession { heir_name: Option<String> },
    /// `War` / `Treaty`: war outcome.
    War {
        opponent_state_id: u32,
        outcome: WarOutcome,
    },
    /// `Conquer`: cells reassigned from loser → winner.
    Conquer { payload: ConquerPayload },
    /// `Schism`: child religion spawn.
    Schism { payload: SchismPayload },
    /// `Plague` / `GoldenAge` / `Migrate`: population scalar (0..1) applied to
    /// the entity's `rural_pop` + `urban_pop`.
    PopScalar { factor: f64 },
    /// `Migrate`: cells + target entity for culture/religion spread.
    Migrate { payload: MigratePayload },
    /// `Raise`: army size + deployment cell.
    Raise { army_size: u32, cell: u32 },
    /// `March`: destination cell.
    March { cell: u32 },
    /// `Disband`: no extra data.
    Disband,
    /// `Raze`: the cell a burg is destroyed on.
    Raze { cell: u32 },
    /// `Dissolve`: no extra data — entity is removed at/after this year.
    Dissolve,
    /// Fallback for payloads the projector doesn't know yet (forward compat).
    Unknown,
}

impl Default for EventPayload {
    fn default() -> Self {
        EventPayload::None
    }
}

// ---------------------------------------------------------------------------//
// Event — a single historical event (design §3.3).
// ---------------------------------------------------------------------------//

/// A single historical event (design §3.3 `Event`). `id` is a globally-unique
/// monotonic id assigned by the engine; `year` is the in-universe year (can be
/// negative before the era anchor). `narrative` is optional LLM-polished prose
/// (design §4.2) — always `None` from the engine, filled by the client later.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Event {
    pub id: u64,
    /// In-universe year (can be negative — design §3.3).
    pub year: i32,
    /// The entity this event targets.
    pub entity_id: u32,
    pub entity_type: EntityType,
    pub kind: EventKind,
    pub payload: EventPayload,
    /// Optional LLM-polished Markdown narrative (design §4.2). `None` while
    /// offline; client fills it in on "Polish with LLM".
    pub narrative: Option<String>,
}

/// A chronologically-sorted `Event[]` (design §3.3 `timeline`).
pub type Timeline = Vec<Event>;

// ---------------------------------------------------------------------------//
// WorldAt — the projected world state at a given year.
// ---------------------------------------------------------------------------//

/// The projected world state at year `Y` (design §3.4 `WorldAt(Y)`).
/// Derived by `project_world` / `project_delta` from the year-0 `Pack` +
/// events with `year <= Y`.
///
/// The per-cell arrays use `u32` (the JS TypedArray-friendly form) where `0`
/// means "unassigned" — the projector converts the year-0 `i32` base arrays
/// (`-1` = unassigned) into this `u32` form so the Phase 5 renderer can upload
/// them directly as data-texture texels.
///
/// Entity snapshots: the year-0 `Pack` entities plus year-Y overrides on
/// pop scalars (`Plague`/`GoldenAge`) and dissolved flags (`Dissolve`).
/// Armies are appended here as `Raise`/`Disband` fire.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct WorldAt {
    /// The year this snapshot is projected for.
    pub year: i32,
    /// Per-cell owning state id at year Y. `0` = water/unassigned.
    /// Derived from the year-0 `cells.state` + `Found`/`Conquer`/`Secession`.
    pub cells_state: Vec<u32>,
    /// Per-cell culture id at year Y. `0` = unassigned.
    pub cells_culture: Vec<u32>,
    /// Per-cell religion id at year Y. `0` = unassigned.
    pub cells_religion: Vec<u32>,
    /// Per-cell burg id at year Y (`0` = none).
    pub cells_burg: Vec<u32>,
    /// Entity snapshots at year Y — same `Pack` shape, with pop scalars and
    /// dissolved flags applied. Armies are appended here as `Raise`/`Disband`
    /// fire.
    pub pack: Pack,
}

impl WorldAt {
    /// Number of cells this snapshot covers.
    pub fn cell_count(&self) -> usize {
        self.cells_state.len()
    }

    /// Re-derive the per-cell state/culture/religion/burg arrays from the
    /// `Pack`'s entity membership (used when only a `Pack` is available, no
    /// base cell arrays). Cells not covered by any entity get `0`.
    #[allow(dead_code)]
    pub fn cells_from_pack(pack: &Pack, cell_count: usize) -> (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>) {
        let mut cells_state = vec![0u32; cell_count];
        let mut cells_culture = vec![0u32; cell_count];
        let mut cells_religion = vec![0u32; cell_count];
        let mut cells_burg = vec![0u32; cell_count];

        // Provinces carry state ownership; cells_province is passed separately
        // in project_world, so this helper can't reconstruct it from Pack alone.
        // We rely on burg→state linkage below for the state fill.

        for burg in &pack.burgs {
            let idx = burg.cell as usize;
            if idx < cell_count {
                cells_burg[idx] = burg.id;
                if burg.state > 0 {
                    cells_state[idx] = burg.state;
                }
                if burg.culture > 0 {
                    cells_culture[idx] = burg.culture;
                }
                if burg.religion > 0 {
                    cells_religion[idx] = burg.religion;
                }
            }
        }
        (cells_state, cells_culture, cells_religion, cells_burg)
    }
}

// ---------------------------------------------------------------------------//
// Conversion: year-0 i32 base arrays (CellData / StatesResult / CulturesResult
// convention, -1 = unassigned) → u32 WorldAt arrays (0 = unassigned).
// ---------------------------------------------------------------------------//

#[inline]
fn i32_to_u32_cell(v: i32) -> u32 {
    if v < 0 { 0 } else { v as u32 }
}

/// Convert the four year-0 `i32` cell arrays from `StatesResult` / `CulturesResult`
/// into the `u32` form used by `WorldAt`.
pub fn convert_base_cells(
    cells_state: &[i32],
    cells_culture: &[i32],
    cells_religion: &[i32],
    cells_burg: &[i16],
) -> (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>) {
    let n = cells_state.len();
    let mut cs = Vec::with_capacity(n);
    let mut cc = Vec::with_capacity(n);
    let mut cr = Vec::with_capacity(n);
    let mut cb = Vec::with_capacity(cells_burg.len());
    for &v in cells_state {
        cs.push(i32_to_u32_cell(v));
    }
    for &v in cells_culture {
        cc.push(i32_to_u32_cell(v));
    }
    for &v in cells_religion {
        cr.push(i32_to_u32_cell(v));
    }
    for &v in cells_burg {
        // i16: 0 = none, positive = burg id (1-based)
        cb.push(if v < 0 { 0 } else { v as u32 });
    }
    (cs, cc, cr, cb)
}

// ---------------------------------------------------------------------------//
// projectors
// ---------------------------------------------------------------------------//

/// Entry point exposed to JS (Phase 5 scrubber): project `WorldAt(target_year)`
/// from a base `Pack` + year-0 cell arrays + `timeline`.
///
/// The four `*_cells_*` slices must come from the Phase 3 generators' results
/// (`StatesResult.cells_state`, etc.) — they use the `i32` (`-1` = unassigned)
/// and `i16` (`0` = none) conventions; this fn normalizes them to the `u32`
/// (`0` = unassigned) form `WorldAt` returns to JS.
///
/// This is O(events ≤ Y) and allocates a fresh `WorldAt`. For incremental
/// scrubbing, the JS side calls `project_delta_h` (held-grid variant) instead.
pub fn project_world(
    pack: &Pack,
    cells_state: &[i32],
    cells_culture: &[i32],
    cells_religion: &[i32],
    cells_burg: &[i16],
    timeline: &Timeline,
    target_year: i32,
) -> WorldAt {
    let (cs, cc, cr, cb) = convert_base_cells(cells_state, cells_culture, cells_religion, cells_burg);
    project_world_u32(pack, &cs, &cc, &cr, &cb, timeline, target_year)
}

/// Project using already-`u32` cell arrays (the normalized form). Internal — use
/// `project_world` for the public API or `project_delta` for incremental scrubbing.
pub fn project_world_u32(
    pack: &Pack,
    cells_state: &[u32],
    cells_culture: &[u32],
    cells_religion: &[u32],
    cells_burg: &[u32],
    timeline: &Timeline,
    target_year: i32,
) -> WorldAt {
    let mut world = WorldAt {
        year: i32::MIN, // sentinel: apply ALL events ≤ target_year
        cells_state: cells_state.to_vec(),
        cells_culture: cells_culture.to_vec(),
        cells_religion: cells_religion.to_vec(),
        cells_burg: cells_burg.to_vec(),
        pack: pack.clone(),
    };

    apply_delta(&mut world, timeline, i32::MIN, target_year);
    world.year = target_year;
    world
}

/// Apply only the events in `(prev_year, target_year]` to an existing `WorldAt`,
/// mutating it in place (design §3.4 "incremental step"). This is the fast
/// path for scrubbing one year at a time — avoids re-applying the full history.
///
/// **Backward jumps** (`target_year <= prev_year`) can't be done incrementally
/// (events aren't reversible); the caller must call `project_world` for those.
/// This fn treats a backward target as a no-op on cell arrays and only bumps
/// `world.year` (the caller is expected to have re-projected from base).
pub fn project_delta(world: &mut WorldAt, timeline: &Timeline, prev_year: i32, target_year: i32) {
    if target_year <= prev_year {
        world.year = target_year;
        return;
    }
    apply_delta(world, timeline, prev_year, target_year);
    world.year = target_year;
}

/// Internal: apply every event with `prev_year < year <= target_year`, in
/// chronological order (design §4.2 "chronological order, evolving WorldAt").
fn apply_delta(world: &mut WorldAt, timeline: &Timeline, prev_year: i32, target_year: i32) {
    let events: Vec<&Event> = timeline
        .iter()
        .filter(|e| e.year > prev_year && e.year <= target_year)
        .collect();
    // Timeline is expected to be sorted, but we sort defensively to honor the
    // "chronological order" invariant without caller burden.
    let mut events = events;
    events.sort_by_key(|e| e.year);

    for ev in events {
        apply_event(world, ev);
    }
}

/// Apply a single event to the working `WorldAt`. Unknown payload kinds are
/// skipped (forward compatibility — never panic).
fn apply_event(world: &mut WorldAt, ev: &Event) {
    match ev.kind {
        EventKind::Found => {
            // A state/burg is founded at entity_id; mark the cell owned.
            if let EventPayload::Found { cell } = &ev.payload {
                match ev.entity_type {
                    EntityType::State => set_cell(&mut world.cells_state, *cell, ev.entity_id),
                    EntityType::Burg => set_cell(&mut world.cells_burg, *cell, ev.entity_id),
                    _ => {}
                }
            }
        }
        EventKind::Conquer => {
            // Winner claims the listed cells.
            if let EventPayload::Conquer { payload } = &ev.payload {
                for &cell in &payload.cells {
                    set_cell(&mut world.cells_state, cell, ev.entity_id);
                }
            }
        }
        EventKind::Secession => {
            // The seceding state claims the listed cells (reusing ConquerPayload).
            if let EventPayload::Conquer { payload } = &ev.payload {
                for &cell in &payload.cells {
                    set_cell(&mut world.cells_state, cell, ev.entity_id);
                }
            }
        }
        EventKind::Raze => {
            if let EventPayload::Raze { cell } = &ev.payload {
                set_cell(&mut world.cells_burg, *cell, 0);
                if let Some(b) = find_burg_mut(&mut world.pack, ev.entity_id) {
                    b.dissolved_year = Some(ev.year);
                }
            }
        }
        EventKind::Schism => {
            // Spawn a child Religion with a seeded follower fraction.
            if let EventPayload::Schism { payload } = &ev.payload {
                if let Some(rel) = find_religion(&world.pack, ev.entity_id) {
                    let child = Religion {
                        id: payload.child_religion_id,
                        name: format!("{}_schism", rel.name),
                        color: rel.color, // parent color; renderer tints children
                        center_cell: rel.center_cell,
                        parent: Some(rel.id),
                        followers: rel.followers * payload.follower_fraction,
                        type_code: rel.type_code,
                        expansion_mode: rel.expansion_mode.clone(),
                        founded_year: ev.year,
                        dissolved_year: None,
                    };
                    world.pack.religions.push(child);
                    // Reassign the child's followers from the parent.
                    if let Some(p) = find_religion_mut(&mut world.pack, ev.entity_id) {
                        p.followers *= 1.0 - payload.follower_fraction;
                    }
                }
            }
        }
        EventKind::Plague | EventKind::GoldenAge => {
            // Scale pops for the target entity.
            let factor = match &ev.payload {
                EventPayload::PopScalar { factor } => *factor,
                _ => 1.0,
            };
            match ev.entity_type {
                EntityType::State => {
                    if let Some(s) = find_state_mut(&mut world.pack, ev.entity_id) {
                        s.rural_pop *= factor;
                        s.urban_pop *= factor;
                    }
                }
                EntityType::Burg => {
                    if let Some(b) = find_burg_mut(&mut world.pack, ev.entity_id) {
                        b.population *= factor;
                    }
                }
                EntityType::Pop => {
                    // Aggregate: scale every burg + state pop.
                    for s in &mut world.pack.states {
                        s.rural_pop *= factor;
                        s.urban_pop *= factor;
                    }
                    for b in &mut world.pack.burgs {
                        b.population *= factor;
                    }
                }
                _ => {}
            }
        }
        EventKind::Migrate => {
            if let EventPayload::Migrate { payload } = &ev.payload {
                for &cell in &payload.cells {
                    match ev.entity_type {
                        EntityType::Culture => set_cell(&mut world.cells_culture, cell, payload.target_id),
                        EntityType::Religion => set_cell(&mut world.cells_religion, cell, payload.target_id),
                        _ => {}
                    }
                }
            }
        }
        EventKind::Raise => {
            // Create a new army.
            if let EventPayload::Raise { army_size, cell } = &ev.payload {
                let new_id = world.pack.armies.last().map_or(1, |a: &Army| a.id + 1);
                world.pack.armies.push(Army {
                    id: new_id,
                    state: ev.entity_id,
                    cell: *cell,
                    size: *army_size,
                    kind: "infantry".to_string(),
                    founded_year: ev.year,
                    dissolved_year: None,
                });
            }
        }
        EventKind::March => {
            if let EventPayload::March { cell } = &ev.payload {
                for a in &mut world.pack.armies {
                    if a.id == ev.entity_id {
                        a.cell = *cell;
                    }
                }
            }
        }
        EventKind::Disband => {
            if let Some(a) = world.pack.armies.iter_mut().find(|a| a.id == ev.entity_id) {
                a.dissolved_year = Some(ev.year);
            }
        }
        EventKind::Dissolve => {
            match ev.entity_type {
                EntityType::State => if let Some(s) = find_state_mut(&mut world.pack, ev.entity_id) { s.dissolved_year = Some(ev.year); },
                EntityType::Culture => if let Some(c) = find_culture_mut(&mut world.pack, ev.entity_id) { c.dissolved_year = Some(ev.year); },
                EntityType::Religion => if let Some(r) = find_religion_mut(&mut world.pack, ev.entity_id) { r.dissolved_year = Some(ev.year); },
                EntityType::Burg => if let Some(b) = find_burg_mut(&mut world.pack, ev.entity_id) { b.dissolved_year = Some(ev.year); },
                _ => {}
            }
        }
        EventKind::Succession | EventKind::CivilWar | EventKind::Treaty |
        EventKind::Battle => {
            // Succession: heir inherits (no cell change — the state persists).
            // CivilWar / Treaty / Battle: modelled via Conquer events
            // that carry actual cell transfers, so this is a data-model no-op.
        }
        EventKind::War => {
            // War: the attacker wins and claims `conquered_cells` from the
            // outcome payload. Apply each cell flip to cells_state.
            if let EventPayload::War { outcome, .. } = &ev.payload {
                for &cell in &outcome.conquered_cells {
                    set_cell(&mut world.cells_state, cell, ev.entity_id);
                }
            }
        }
    }
}
//----------------------------------------------------------------------------//
// small helpers
// ---------------------------------------------------------------------------//

#[inline]
fn set_cell(arr: &mut [u32], cell: u32, value: u32) {
    let idx = cell as usize;
    if idx < arr.len() {
        arr[idx] = value;
    }
}

fn find_state_mut<'p>(pack: &'p mut Pack, id: u32) -> Option<&'p mut State> {
    pack.states.iter_mut().find(|s| s.id == id)
}

fn find_culture_mut<'p>(pack: &'p mut Pack, id: u32) -> Option<&'p mut Culture> {
    pack.cultures.iter_mut().find(|c| c.id == id)
}

fn find_religion<'p>(pack: &'p Pack, id: u32) -> Option<&'p Religion> {
    pack.religions.iter().find(|r| r.id == id)
}

fn find_religion_mut<'p>(pack: &'p mut Pack, id: u32) -> Option<&'p mut Religion> {
    pack.religions.iter_mut().find(|r| r.id == id)
}

fn find_burg_mut<'p>(pack: &'p mut Pack, id: u32) -> Option<&'p mut Burg> {
    pack.burgs.iter_mut().find(|b| b.id == id)
}

// ===========================================================================//
// Tests — verification gate (plan §Step 4.1):
//   - full projection vs hand-built timeline with Found + Conquer
//   - delta projection equals full projection for the same Y
//   - determinism: identical timeline + base → identical WorldAt
// ===========================================================================//

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal Pack with one state (id=1) owning cells [0..3], one
    /// culture, one religion, one burg (id=1) on cell 0.
    fn sample_pack_one_state() -> Pack {
        Pack {
            states: vec![State {
                id: 1, name: "Aeloria".into(), color: 0x4a6fa5, capital: 1,
                center_cell: 0, form: "Monarchy".into(), tax_rate: 0.12,
                treasury: 5000.0, rural_pop: 12000.0, urban_pop: 8000.0,
                military: 40, founded_year: 0, dissolved_year: None, culture: 1,
            }],
            provinces: vec![],
            cultures: vec![Culture {
                id: 1, name: "Northland".into(), color: 0xaa8844, origin: 0,
                type_code: 0, founded_year: 0, dissolved_year: None, cell_count: 3,
            }],
            religions: vec![Religion {
                id: 1, name: "Old Faith".into(), color: 0xddccbb, center_cell: 0,
                parent: None, followers: 20000.0, type_code: 0,
                expansion_mode: "global".into(), founded_year: 0, dissolved_year: None,
            }],
            burgs: vec![Burg {
                id: 1, name: "Aeloria City".into(), cell: 0, state: 1,
                culture: 1, religion: 1, population: 8.0, feature: 1,
                capital: 1, founded_year: 0, dissolved_year: None,
            }],
            armies: vec![],
        }
    }

    /// Year-0 per-cell arrays matching the `StatesResult`/`CulturesResult`
    /// i32/i16 convention (-1 = unassigned, 0 = no burg).
    fn base_cells(n: usize) -> (Vec<i32>, Vec<i32>, Vec<i32>, Vec<i16>) {
        (
            vec![1i32; n],
            vec![1i32; n],
            vec![1i32; n],
            { let mut b = vec![0i16; n]; if n > 0 { b[0] = 1; } b },
        )
    }

    /// Feed a hand-built timeline: Found (burg on cell 2 at year 10) then
    /// Conquer (state 99 takes cells [3,4] at year 20). project_world at
    /// year 25 should show the burg founded and the cells conquered.
    #[test]
    fn project_world_applies_found_then_conquer() {
        let pack = sample_pack_one_state();
        let (cs, cc, cr, cb) = base_cells(5);
        let timeline: Timeline = vec![
            Event {
                id: 1, year: 10, entity_id: 99,
                entity_type: EntityType::Burg, kind: EventKind::Found,
                payload: EventPayload::Found { cell: 2 }, narrative: None,
            },
            Event {
                id: 2, year: 20, entity_id: 99,
                entity_type: EntityType::State, kind: EventKind::Conquer,
                payload: EventPayload::Conquer {
                    payload: ConquerPayload { cells: vec![3, 4] },
                },
                narrative: None,
            },
        ];

        // Before year 10: burg 1 only on cell 0, cells 0..3 owned by state 1.
        let w0 = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 5);
        assert_eq!(w0.cells_burg[0], 1);
        assert_eq!(w0.cells_burg[2], 0);

        // After Found at year 10: burg 99 on cell 2.
        let w1 = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 10);
        assert_eq!(w1.cells_burg[2], 99);

        // After Conquer at year 20: cells 3,4 owned by state 99; 0,1,2 untouched.
        let w2 = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 25);
        assert_eq!(w2.cells_state[0], 1);
        assert_eq!(w2.cells_state[1], 1);
        assert_eq!(w2.cells_state[2], 1);
        assert_eq!(w2.cells_state[3], 99);
        assert_eq!(w2.cells_state[4], 99);
        assert_eq!(w2.cells_burg[2], 99);
    }

    /// Delta projection over (10, 25] from a year-10 snapshot must equal
    /// the full projection at year 25.
    #[test]
    fn delta_projection_equals_full_projection() {
        let pack = sample_pack_one_state();
        let (cs, cc, cr, cb) = base_cells(5);
        let timeline: Timeline = {
            let mut evs = Vec::new();
            for y in (10..=30).step_by(5) {
                evs.push(Event {
                    id: y as u64,
                    year: y,
                    entity_id: 1,
                    entity_type: if y % 2 == 0 { EntityType::State } else { EntityType::Burg },
                    kind: if y % 2 == 0 { EventKind::Conquer } else { EventKind::Found },
                    payload: if y % 2 == 0 {
                        EventPayload::Conquer { payload: ConquerPayload { cells: vec![(y % 5) as u32] } }
                    } else {
                        EventPayload::Found { cell: (y % 5) as u32 }
                    },
                    narrative: None,
                });
            }
            evs
        };

        let full = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 25);

        let mut delta = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 10);
        project_delta(&mut delta, &timeline, 10, 15);
        project_delta(&mut delta, &timeline, 15, 20);
        project_delta(&mut delta, &timeline, 20, 25);

        assert_eq!(delta.cells_state, full.cells_state, "state cells must match full projection");
        assert_eq!(delta.cells_culture, full.cells_culture);
        assert_eq!(delta.cells_burg, full.cells_burg);
        assert_eq!(delta.pack.states.len(), full.pack.states.len());
    }

    /// Determinism: identical timeline + base → identical WorldAt.
    #[test]
    fn project_world_is_deterministic() {
        let pack = sample_pack_one_state();
        let (cs, cc, cr, cb) = base_cells(5);
        let timeline: Timeline = vec![
            Event {
                id: 1, year: 5, entity_id: 1,
                entity_type: EntityType::State, kind: EventKind::Conquer,
                payload: EventPayload::Conquer { payload: ConquerPayload { cells: vec![2, 3] } },
                narrative: None,
            },
            Event {
                id: 2, year: 15, entity_id: 1,
                entity_type: EntityType::State, kind: EventKind::Plague,
                payload: EventPayload::PopScalar { factor: 0.5 },
                narrative: None,
            },
        ];

        let a = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 20);
        let b = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 20);

        assert_eq!(a, b, "WorldAt must be deterministic for identical inputs");
        // Pop scalar applied: state 1 rural_pop halved from 12000 → 6000.
        assert_eq!(a.pack.states[0].rural_pop.to_bits(), (12000.0f64 * 0.5).to_bits());
        // Conquer applied.
        assert_eq!(a.cells_state[2], 1);
        assert_eq!(a.cells_state[3], 1);
    }

    /// A Schism event spawns a child Religion with `parent = Some(parent)`
    /// and transfers a follower fraction.
    #[test]
    fn schism_spawns_child_religion() {
        let pack = sample_pack_one_state();
        let (cs, cc, cr, cb) = base_cells(4);
        let timeline: Timeline = vec![Event {
            id: 1, year: 100, entity_id: 1,
            entity_type: EntityType::Religion, kind: EventKind::Schism,
            payload: EventPayload::Schism {
                payload: SchismPayload { follower_fraction: 0.3, child_religion_id: 2 },
            },
            narrative: None,
        }];

        let w = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 100);
        // Parent followers reduced to 70%.
        assert_eq!(w.pack.religions[0].followers.to_bits(), (20000.0f64 * 0.7).to_bits());
        // One child religion spawned.
        assert_eq!(w.pack.religions.len(), 2);
        let child = &w.pack.religions[1];
        assert_eq!(child.id, 2);
        assert_eq!(child.parent, Some(1));
        assert_eq!(child.followers.to_bits(), (20000.0f64 * 0.3).to_bits());
        assert_eq!(child.founded_year, 100);
    }

    /// A `Dissolve` event marks an entity dissolved at the event's year.
    #[test]
    fn dissolve_marks_entity_dissolved() {
        let pack = sample_pack_one_state();
        let (cs, cc, cr, cb) = base_cells(3);
        let timeline: Timeline = vec![Event {
            id: 1, year: 50, entity_id: 1,
            entity_type: EntityType::State, kind: EventKind::Dissolve,
            payload: EventPayload::Dissolve, narrative: None,
        }];

        let w = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 50);
        assert_eq!(w.pack.states[0].dissolved_year, Some(50));
    }

    /// Serde round-trip: an `Event` with a `Schism` payload survives a
    /// serde_json round-trip byte-identically (design §7 reload invariant).
    #[test]
    fn event_serde_round_trips() {
        let ev = Event {
            id: 42, year: 800, entity_id: 7,
            entity_type: EntityType::Religion, kind: EventKind::Schism,
            payload: EventPayload::Schism {
                payload: SchismPayload { follower_fraction: 0.25, child_religion_id: 8 },
            },
            narrative: Some("The faith split.".into()),
        };
        let json = serde_json::to_string(&ev).expect("serialize");
        let back: Event = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(ev, back);

        let mut ev2 = ev.clone();
        ev2.narrative = None;
        let json2 = serde_json::to_string(&ev2).expect("serialize2");
        let back2: Event = serde_json::from_str(&json2).expect("deserialize2");
        assert_eq!(ev2, back2);
    }

    /// Backward delta (target ≤ prev) is a safe no-op on cell arrays.
    #[test]
    fn delta_backward_no_op_does_not_corrupt() {
        let pack = sample_pack_one_state();
        let (cs, cc, cr, cb) = base_cells(3);
        let timeline: Timeline = vec![];
        let mut w = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 10);
        let state_before = w.cells_state.clone();
        project_delta(&mut w, &timeline, 10, 5);
        assert_eq!(w.cells_state, state_before);
        assert_eq!(w.year, 5);
    }

    /// Unknown payload kind is skipped gracefully (forward compatibility).
    #[test]
    fn unknown_payload_does_not_panic() {
        let pack = sample_pack_one_state();
        let (cs, cc, cr, cb) = base_cells(3);
        let timeline: Timeline = vec![Event {
            id: 1, year: 10, entity_id: 1,
            entity_type: EntityType::State, kind: EventKind::Found,
            payload: EventPayload::Unknown, narrative: None,
        }];
        let w = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 10);
        let (exp_cs, exp_cc, _, _) = convert_base_cells(&cs, &cc, &vec![-1i32; 3], &cb);
        assert_eq!(w.cells_state, exp_cs);
    }

    /// `convert_base_cells` normalizes -1 → 0 and preserves positive ids.
    #[test]
    fn convert_base_cells_normalizes_unassigned() {
        let cells_state = vec![-1i32, 1, -1, 3, 2];
        let cells_culture = vec![1i32, -1, 2, -1, 1];
        let cells_religion = vec![-1i32; 5];
        let cells_burg = vec![0i16, 1, 0, 2, 0];
        let (cs, cc, cr, cb) = convert_base_cells(&cells_state, &cells_culture, &cells_religion, &cells_burg);
        assert_eq!(cs, vec![0u32, 1, 0, 3, 2]);
        assert_eq!(cc, vec![1u32, 0, 2, 0, 1]);
        assert_eq!(cr, vec![0u32; 5]);
        assert_eq!(cb, vec![0u32, 1, 0, 2, 0]);
    }

    /// `Raise` event spawns a new army; `Disband` marks it dissolved.
    #[test]
    fn raise_then_disband_army_lifecycle() {
        let pack = sample_pack_one_state();
        let (cs, cc, cr, cb) = base_cells(5);
        let timeline: Timeline = vec![
            Event {
                id: 1, year: 30, entity_id: 1,
                entity_type: EntityType::State, kind: EventKind::Raise,
                payload: EventPayload::Raise { army_size: 5000, cell: 2 },
                narrative: None,
            },
            Event {
                id: 2, year: 40, entity_id: 1,
                entity_type: EntityType::Army, kind: EventKind::Disband,
                payload: EventPayload::Disband, narrative: None,
            },
        ];

        let w_at_raise = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 30);
        assert_eq!(w_at_raise.pack.armies.len(), 1);
        assert_eq!(w_at_raise.pack.armies[0].size, 5000);
        assert_eq!(w_at_raise.pack.armies[0].cell, 2);
        assert_eq!(w_at_raise.pack.armies[0].state, 1);
        assert_eq!(w_at_raise.pack.armies[0].dissolved_year, None);

        let w_at_disband = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 50);
        assert_eq!(w_at_disband.pack.armies.len(), 1);
        assert_eq!(w_at_disband.pack.armies[0].dissolved_year, Some(40));
    }

    /// A `War` event with `conquered_cells` in the outcome must flip those
    /// cells to the attacker's ownership in the projected WorldAt.
    #[test]
    fn war_event_conquers_cells_in_projection() {
        let pack = sample_pack_one_state();
        // cell 3 and 4 start owned by state 2 (opponent), rest by state 1.
        let cs = vec![1i32, 1, 1, 2, 2];
        let cc = vec![1i32, 1, 1, 1, 1];
        let cr = vec![1i32; 5];
        let cb = vec![0i16; 5];

        // War event: state 1 attacks state 2 and conquers cells 3,4.
        let timeline: Timeline = vec![Event {
            id: 1, year: 10, entity_id: 1,
            entity_type: EntityType::State, kind: EventKind::War,
            payload: EventPayload::War {
                opponent_state_id: 2,
                outcome: WarOutcome {
                    result: 0,
                    attrition: 0.3,
                    conquered_cells: vec![3, 4],
                },
            },
            narrative: None,
        }];

        // Before the war: cells 3,4 are owned by state 2.
        let w0 = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 5);
        assert_eq!(w0.cells_state[3], 2, "cell 3 should be state 2 before war");
        assert_eq!(w0.cells_state[4], 2, "cell 4 should be state 2 before war");

        // After the war: cells 3,4 flip to state 1 (the attacker).
        let w1 = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 15);
        assert_eq!(w1.cells_state[3], 1, "cell 3 should flip to state 1 after war");
        assert_eq!(w1.cells_state[4], 1, "cell 4 should flip to state 1 after war");
        // Cells 0,1,2 stay with state 1.
        assert_eq!(w1.cells_state[0], 1);
        assert_eq!(w1.cells_state[1], 1);
        assert_eq!(w1.cells_state[2], 1);

        // Delta projection must match full projection.
        let mut w_delta = project_world(&pack, &cs, &cc, &cr, &cb, &timeline, 10);
        project_delta(&mut w_delta, &timeline, 10, 15);
        assert_eq!(w_delta.cells_state, w1.cells_state, "delta projection must match full");
    }
}

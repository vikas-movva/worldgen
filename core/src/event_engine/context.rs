//! Phase 4 Step 4.2 — Event engine mutable working context.
//!
//! Extracted from the monolithic `event_engine.rs`. The refactor splits the
//! former monolithic `GenContext` into three focused state components:
//!
//! - [`GenWorld`] — simulation/world state: the working `Pack` plus the
//!   per-cell ownership/culture/religion/burg/province arrays.
//! - [`GenMap`] — map state: the per-cell heightmap and the Voronoi/Delaunay
//!   cell-adjacency topology. The topology is a concrete `Cells` (never an
//!   `Option`); production and tests always supply a valid topology.
//! - [`GenTimeline`] — timeline/event state: the era bounds, the tunable
//!   parameters, the emitted `Event` sink (`events`), and the monotonic
//!   event-id counter (`next_id`). `events` and `next_id` are private and are
//!   only mutated through [`GenTimeline::emit`] / [`GenTimeline::push_event`].
//!
//! [`GenContext`] remains the orchestration object passed between event
//! modules. It owns one of each component and exposes coordinated access to
//! them, delegating entity lookups / cell helpers to the right component.
//!
//! Modules access this context via `&mut GenContext` through the `EventModule`
//! trait (see `mod.rs`).

use super::params::TimelineParams;
use crate::entities::{Burg, Culture, Pack, Religion, State};
use crate::mesh::Cells;
use crate::timeline::{EntityType, Event, EventKind, EventPayload};
use rand::rngs::StdRng;

/// Simulation/world state: the working `Pack` plus the per-cell entity index
/// arrays.
///
/// Per-cell conventions (all `u32`, normalized from the `i32`/`i16` season-0
/// convention at the engine boundary):
///
/// ```text
/// cells_state    0 = unassigned
/// cells_culture  0 = unassigned
/// cells_religion 0 = unassigned
/// cells_burg     0 = none
/// cells_province 0 = unassigned
/// ```
pub struct GenWorld {
    /// Working pack (cloned from the year-0 base). Modules mutate entity
    /// fields like `dissolved_year`, `followers`, `population`, etc.
    pub pack: Pack,
    /// Per-cell owning state id (`0` = unassigned in u32 form).
    pub cells_state: Vec<u32>,
    /// Per-cell culture id (`0` = unassigned).
    pub cells_culture: Vec<u32>,
    /// Per-cell religion id (`0` = unassigned).
    pub cells_religion: Vec<u32>,
    /// Per-cell burg id (`0` = none).
    pub cells_burg: Vec<u32>,
    /// Per-cell province id (`0` = unassigned). Used by the war module to
    /// identify which province a disputed cell belongs to, so that conquering
    /// a cell transfers the entire province.
    pub cells_province: Vec<u32>,
}

impl GenWorld {
    /// Number of cells.
    pub fn cell_count(&self) -> usize {
        self.cells_state.len()
    }

    /// The owning state id at `cell`, if assigned (non-zero).
    /// Behavior-oriented accessor; kept as public API even though no current
    /// module reads state via it directly (see `GenContext::world`).
    #[allow(dead_code)]
    pub fn state_at(&self, cell: usize) -> Option<u32> {
        self.cells_state.get(cell).copied().filter(|&v| v != 0)
    }

    /// The province id at `cell`, if assigned (non-zero).
    /// Behavior-oriented accessor; kept as public API even though no current
    /// module reads province via it directly (see `GenContext::world`).
    #[allow(dead_code)]
    pub fn province_at(&self, cell: usize) -> Option<u32> {
        self.cells_province.get(cell).copied().filter(|&v| v != 0)
    }

    /// Find a state by id.
    pub fn find_state(&self, id: u32) -> Option<&State> {
        self.pack.states.iter().find(|s| s.id == id)
    }

    /// Find a state by id (mutable).
    pub fn find_state_mut(&mut self, id: u32) -> Option<&mut State> {
        self.pack.states.iter_mut().find(|s| s.id == id)
    }

    /// Find a religion by id.
    pub fn find_religion(&self, id: u32) -> Option<&Religion> {
        self.pack.religions.iter().find(|r| r.id == id)
    }

    /// Find a religion by id (mutable).
    pub fn find_religion_mut(&mut self, id: u32) -> Option<&mut Religion> {
        self.pack.religions.iter_mut().find(|r| r.id == id)
    }

    /// Find a burg by id (mutable).
    pub fn find_burg_mut(&mut self, id: u32) -> Option<&mut Burg> {
        self.pack.burgs.iter_mut().find(|b| b.id == id)
    }

    /// Find a culture by id.
    pub fn find_culture(&self, id: u32) -> Option<&Culture> {
        self.pack.cultures.iter().find(|c| c.id == id)
    }

    /// Find a culture by id (mutable).
    pub fn find_culture_mut(&mut self, id: u32) -> Option<&mut Culture> {
        self.pack.cultures.iter_mut().find(|c| c.id == id)
    }

    /// Find cells owned by `state_id`.
    pub fn cells_of_state(&self, state_id: u32) -> Vec<u32> {
        self.cells_state
            .iter()
            .enumerate()
            .filter_map(|(i, &s)| if s == state_id { Some(i as u32) } else { None })
            .collect()
    }

    /// Next free burg id.
    pub fn next_burg_id(&self) -> u32 {
        self.pack.burgs.last().map_or(1, |b| b.id + 1)
    }

    /// Next free army id.
    pub fn next_army_id(&self) -> u32 {
        self.pack.armies.last().map_or(1, |a| a.id + 1)
    }

    /// Next free state id (for secessions).
    pub fn next_state_id(&self) -> u32 {
        self.pack.states.last().map_or(1, |s| s.id + 1)
    }

    /// Next free religion id (for schisms).
    pub fn next_religion_id(&self) -> u32 {
        self.pack.religions.last().map_or(1, |r| r.id + 1)
    }
}

/// Map state: the per-cell heightmap and the Voronoi/Delaunay cell-adjacency
/// topology.
///
/// The topology is a concrete [`Cells`] (never an `Option`). Production always
/// supplies a real mesh. When a caller (e.g. a synthetic test or a WASM call
/// with no mesh geometry) has no mesh, the engine resolves it to a minimal
/// square-grid topology that reproduces the legacy adjacency, so the refactor
/// stays behavior-preserving.
pub struct GenMap {
    /// Per-cell heightmap (`< 20` == water, FMG sea level).
    pub heights: Vec<u8>,
    /// Voronoi/Delaunay cell-adjacency topology (CSR `c`/`i` arrays from
    /// [`Cells`]).
    pub topology: Cells,
}

impl GenMap {
    /// Get the edge-sharing neighbors of `cell` from the Delaunay topology.
    pub fn neighbors_of_cell(&self, cell: usize) -> &[u32] {
        self.topology.neighbors_of_cell(cell)
    }

    /// Is cell `c` land? (height >= 20, FMG sea level.)
    pub fn is_land(&self, cell: usize) -> bool {
        self.heights
            .get(cell)
            .is_some_and(|&h| h >= crate::heightmap::SEA_LEVEL)
    }
}

/// Timeline/event state: era bounds, tunable parameters, the emitted event
/// sink, and the monotonic event-id counter.
pub struct GenTimeline {
    /// The era bounds.
    pub era_start: i32,
    pub era_end: i32,
    /// The parameters.
    pub params: TimelineParams,
    /// Shared event sink. Modules append events here via [`GenTimeline::emit`]
    /// or [`GenTimeline::push_event`].
    events: Vec<Event>,
    /// Monotonic event-id counter (seeds from the timeline seed).
    next_id: u64,
}

impl GenTimeline {
    /// Create an empty timeline with the given era bounds, parameters, and
    /// starting event-id counter.
    pub fn new(era_start: i32, era_end: i32, params: TimelineParams, next_id: u64) -> Self {
        GenTimeline {
            era_start,
            era_end,
            params,
            events: Vec::new(),
            next_id,
        }
    }

    /// Assign the next deterministic event id.
    fn next_event_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Emit a fully-formed event into the sink. Assigns a deterministic id
    /// from the monotonic counter unless the event already carries one, then
    /// pushes it. Returns the (assigned) id of the emitted event.
    pub fn emit(&mut self, mut event: Event) -> u64 {
        if event.id == 0 {
            event.id = self.next_event_id();
        }
        self.events.push(event);
        // The id assigned/what the caller should observe is the event's id.
        self.events.last().map(|e| e.id).unwrap_or(0)
    }

    /// Push an event into the sink. The caller provides the year; `id` is
    /// assigned from the monotonic counter. Returns the assigned id.
    pub fn push_event(
        &mut self,
        year: i32,
        entity_id: u32,
        entity_type: EntityType,
        kind: EventKind,
        payload: EventPayload,
    ) -> u64 {
        self.emit(Event {
            id: 0,
            year,
            entity_id,
            entity_type,
            kind,
            payload,
            narrative: None,
        })
    }

    /// Immutable access to the emitted events.
    /// Part of the emission API; the engine itself consumes via
    /// [`GenTimeline::into_events`].
    #[allow(dead_code)]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Consume the timeline, returning the emitted events.
    pub fn into_events(self) -> Vec<Event> {
        self.events
    }

    /// Borrow the RNG as `&mut StdRng` — the type that modules receive.
    /// Modules do NOT hold an RNG reference in the trait; they receive it as
    /// a parameter to `run`.
    #[allow(dead_code)]
    pub fn _rng_placeholder(&self) -> Option<&StdRng> {
        None
    }
}

/// Mutable working state for the event generator. The engine clones the input
/// `Pack` + cell arrays into this struct (via its [`GenWorld`]) and builds the
/// map + timeline components, then each module reads and mutates them.
///
/// `GenContext` is an orchestration object: it owns the three focused
/// components and delegates access to them, but holds no unrelated fields of
/// its own.
pub struct GenContext {
    /// World state (Pack + per-cell ownership/culture/religion/burg/province).
    pub world: GenWorld,
    /// Map state (heightmap + topology).
    pub map: GenMap,
    /// Timeline/event state (era, params, event sink, id counter).
    pub timeline: GenTimeline,
}

impl GenContext {
    /// Number of cells.
    pub fn cell_count(&self) -> usize {
        self.world.cell_count()
    }

    /// Is cell `c` land? (height >= 20, FMG sea level.)
    pub fn is_land(&self, c: u32) -> bool {
        let idx = c as usize;
        self.map.is_land(idx)
    }

    /// Get cell neighbors of `cell` from the Voronoi/Delaunay topology.
    pub(crate) fn neighbors_of_cell(&self, cell: u32) -> Vec<u32> {
        self.map.neighbors_of_cell(cell as usize).to_vec()
    }

    /// Find a state by id.
    pub fn find_state(&self, id: u32) -> Option<&State> {
        self.world.find_state(id)
    }

    /// Find a state by id (mutable).
    pub fn find_state_mut(&mut self, id: u32) -> Option<&mut State> {
        self.world.find_state_mut(id)
    }

    /// Find a religion by id.
    pub fn find_religion(&self, id: u32) -> Option<&Religion> {
        self.world.find_religion(id)
    }

    /// Find a religion by id (mutable).
    pub fn find_religion_mut(&mut self, id: u32) -> Option<&mut Religion> {
        self.world.find_religion_mut(id)
    }

    /// Find a burg by id (mutable).
    pub fn find_burg_mut(&mut self, id: u32) -> Option<&mut Burg> {
        self.world.find_burg_mut(id)
    }

    /// Find a culture by id.
    pub fn find_culture(&self, id: u32) -> Option<&Culture> {
        self.world.find_culture(id)
    }

    /// Find a culture by id (mutable).
    pub fn find_culture_mut(&mut self, id: u32) -> Option<&mut Culture> {
        self.world.find_culture_mut(id)
    }

    /// Find cells owned by `state_id`.
    pub fn cells_of_state(&self, state_id: u32) -> Vec<u32> {
        self.world.cells_of_state(state_id)
    }

    /// Next free burg id.
    pub fn next_burg_id(&self) -> u32 {
        self.world.next_burg_id()
    }

    /// Next free army id.
    pub fn next_army_id(&self) -> u32 {
        self.world.next_army_id()
    }

    /// Next free state id (for secessions).
    pub fn next_state_id(&self) -> u32 {
        self.world.next_state_id()
    }

    /// Next free religion id (for schisms).
    pub fn next_religion_id(&self) -> u32 {
        self.world.next_religion_id()
    }

    /// Push an event into the sink. The caller provides the year; `id` is
    /// assigned from the monotonic counter on the timeline.
    pub fn push_event(
        &mut self,
        year: i32,
        entity_id: u32,
        entity_type: EntityType,
        kind: EventKind,
        payload: EventPayload,
    ) {
        self.timeline
            .push_event(year, entity_id, entity_type, kind, payload);
    }

    /// Borrow the RNG as `&mut StdRng` — the type that modules receive.
    /// Modules do NOT hold an RNG reference in the trait; they receive it as
    /// a parameter to `run`.
    #[allow(dead_code)]
    pub fn _rng_placeholder(&self) -> Option<&StdRng> {
        None
    }
}

/// Apply a multiplicative scalar to a state's `rural_pop` / `urban_pop`,
/// clamped to the shared [`crate::timeline::POP_FLOOR`] so repeated events can
/// never drive a population to zero, negative, or `NaN`. Shared by the
/// `plague` and `golden_age` modules; the projector applies the same clamp so
/// the projected world matches the generator's working context exactly.
pub(crate) fn scale_state_pops(ctx: &mut GenContext, state_id: u32, mult: f64) {
    if let Some(s) = ctx.find_state_mut(state_id) {
        s.rural_pop = (s.rural_pop * mult).max(crate::timeline::POP_FLOOR);
        s.urban_pop = (s.urban_pop * mult).max(crate::timeline::POP_FLOOR);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_engine::TimelineParams;

    /// Build a REAL Voronoi `Cells` topology from a deterministic mesh, returning
    /// the CSR topology and its exact cell count (`mesh.points.len()`, which can
    /// be less than the requested count due to Poisson shortfall + top-up).
    fn voronoi_topology(cell_count: u32, seed: u32) -> (Cells, usize) {
        let mesh = crate::mesh::build(cell_count, seed);
        (mesh.cells, mesh.points.len())
    }

    /// Build a synthetic test context backed by a real Voronoi topology from a
    /// deterministic mesh. Returns the context and the exact cell count so
    /// callers reference `neighbors_of_cell` against in-range ids.
    fn make_ctx(cell_count: u32, seed: u32) -> (GenContext, usize) {
        let (cells, n) = voronoi_topology(cell_count, seed);
        let world = GenWorld {
            pack: Pack::default(),
            cells_state: vec![1u32; n],
            cells_culture: vec![0u32; n],
            cells_religion: vec![0u32; n],
            cells_burg: vec![0u32; n],
            cells_province: vec![0u32; n],
        };
        let map = GenMap {
            heights: vec![50u8; n],
            topology: cells,
        };
        let timeline = GenTimeline::new(0, 1, TimelineParams::default(), 1);
        (
            GenContext {
                world,
                map,
                timeline,
            },
            n,
        )
    }

    #[test]
    fn out_of_bounds_returns_empty() {
        let (ctx, n) = make_ctx(100, 42);
        assert!(ctx.neighbors_of_cell(n as u32).is_empty());
    }

    /// Build a `GenContext` backed by a real Voronoi mesh so the Delaunay
    /// neighbor path is exercised. Returns the context plus the cell count; the
    /// mesh is generated deterministically from `seed`.
    fn make_ctx_with_mesh(n: u32, seed: u32) -> (GenContext, usize) {
        make_ctx(n, seed)
    }

    /// On a real Voronoi mesh, `neighbors_of_cell` must return exactly the
    /// cells in the CSR `c` slice — i.e. the Delaunay edge-sharing neighbors.
    #[test]
    fn delaunay_neighbors_match_csr_slice() {
        let (ctx, n) = make_ctx_with_mesh(500, 42);
        let cells = &ctx.map.topology;
        for cell in 0..n {
            let expected: Vec<u32> = cells.neighbors_of_cell(cell).to_vec();
            let got = ctx.neighbors_of_cell(cell as u32);
            assert_eq!(got, expected, "cell {cell}: Delaunay neighbors mismatch");
        }
    }

    /// Delaunay adjacency must be symmetric: if `b` is a neighbor of `a`,
    /// then `a` must be a neighbor of `b`.
    #[test]
    fn delaunay_adjacency_is_symmetric() {
        let (ctx, n) = make_ctx_with_mesh(1000, 7);
        for a in 0..n {
            let neighbors = ctx.neighbors_of_cell(a as u32);
            for &b in &neighbors {
                let b_neighbors = ctx.neighbors_of_cell(b);
                assert!(
                    b_neighbors.contains(&(a as u32)),
                    "asymmetric: {a}->{b} set but {b}->{a} missing"
                );
            }
        }
    }

    /// No cell is its own neighbor (no self-loops in the Voronoi topology).
    #[test]
    fn delaunay_no_self_neighbors() {
        let (ctx, n) = make_ctx_with_mesh(800, 123);
        for cell in 0..n {
            let neighbors = ctx.neighbors_of_cell(cell as u32);
            assert!(
                !neighbors.contains(&(cell as u32)),
                "cell {cell} is its own neighbor"
            );
        }
    }

    /// Every reported neighbor is a valid in-range cell id.
    #[test]
    fn delaunay_neighbors_in_bounds() {
        let (ctx, n) = make_ctx_with_mesh(1000, 42);
        for cell in 0..n {
            for &nb in &ctx.neighbors_of_cell(cell as u32) {
                assert!(
                    (nb as usize) < n,
                    "neighbor {nb} of cell {cell} out of bounds (n={n})"
                );
            }
        }
    }

    /// Every cell on a real Voronoi mesh has at least 3 neighbors (triangle
    /// dual invariant), so the Delaunay path must never return < 3 except for
    /// the out-of-bounds case.
    #[test]
    fn delaunay_every_cell_has_at_least_three_neighbors() {
        let (ctx, n) = make_ctx_with_mesh(1000, 42);
        for cell in 0..n {
            let neighbors = ctx.neighbors_of_cell(cell as u32);
            assert!(
                neighbors.len() >= 3,
                "cell {cell} has only {len} neighbors (need ≥3)",
                len = neighbors.len()
            );
        }
    }
}

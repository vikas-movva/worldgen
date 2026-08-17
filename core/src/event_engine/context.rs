//! Phase 4 Step 4.2 — Event engine mutable working context.
//!
//! Extracted from the monolithic `event_engine.rs`. The `GenContext` struct holds
//! the working `Pack` + cell arrays that each `EventModule` reads and mutates,
//! along with the shared event sink and id counter.
//!
//! Modules access this context via `&mut GenContext` through the `EventModule`
//! trait (see `mod.rs`). The context provides convenience helpers for entity
//! lookups and cell assignment (refactor §P4.2-modular).

use crate::entities::{Burg, Culture, Pack, Religion, State};
use crate::mesh::Cells;
use crate::timeline::{EntityType, Event, EventKind, EventPayload};
use rand::rngs::StdRng;

/// Mutable working state for the event generator. The engine clones the input
/// `Pack` + cell arrays into this struct, then each module reads and mutates
/// it. Events are emitted into the shared `events` vec.
pub struct GenContext {
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
    /// The era bounds.
    pub era_start: i32,
    pub era_end: i32,
    /// The parameters.
    pub params: super::params::TimelineParams,
    /// Shared event sink. Modules append events here.
    pub events: Vec<Event>,
    /// Monotonic event-id counter (seeds from the timeline seed).
    pub next_id: u64,
    /// The grid's per-cell heightmap, for land/water checks.
    pub cells_h: Vec<u8>,
    /// The Voronoi/Delaunay cell-adjacency topology (CSR `c`/`i` arrays from
    /// `mesh::Cells`). `Some` when the caller supplies a real mesh so that
    /// `neighbors_of_cell` returns true edge-sharing neighbors instead of the
    /// legacy square-grid approximation. `None` only in synthetic unit tests
    /// that build `GenContext` directly without a mesh; those tests must not
    /// depend on geometric neighbor counts.
    pub cells: Option<Cells>,
}

impl GenContext {
    /// Number of cells.
    pub fn cell_count(&self) -> usize {
        self.cells_state.len()
    }

    /// Is cell `c` land? (height >= 20, FMG sea level.)
    pub fn is_land(&self, c: u32) -> bool {
        let idx = c as usize;
        idx < self.cells_h.len() && self.cells_h[idx] >= 20
    }

    /// Assign a deterministic event id and return it.
    pub fn next_event_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Push an event into the sink. The caller provides the year; `id` is
    /// assigned from the monotonic counter.
    pub fn push_event(
        &mut self,
        year: i32,
        entity_id: u32,
        entity_type: EntityType,
        kind: EventKind,
        payload: EventPayload,
    ) {
        let id = self.next_event_id();
        self.events.push(Event {
            id,
            year,
            entity_id,
            entity_type,
            kind,
            payload,
            narrative: None,
        });
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

    /// Get cell neighbors of `cell`.
    ///
    /// When the context holds a real Voronoi/Delaunay topology (`self.cells`
    /// is `Some`), this returns the true edge-sharing neighbors by indexing the
    /// CSR adjacency arrays (`cells.c` / `cells.i`) — the same topology the
    /// mesh generator produces and the war module's `find_border_cells` walks.
    ///
    /// When no topology is available (only synthetic unit tests that build a
    /// `GenContext` without a mesh), it falls back to the 4-directional square
    /// grid adjacency inferred from `cell_count` (side = `sqrt(N)`). This
    /// fallback is deprecated; production callers always supply a mesh.
    pub(crate) fn neighbors_of_cell(&self, cell: u32) -> Vec<u32> {
        let idx = cell as usize;
        if idx >= self.cell_count() {
            return Vec::new();
        }
        if let Some(cells) = &self.cells {
            // Real Voronoi/Delaunay topology — true edge-sharing neighbors.
            return cells.neighbors_of_cell(idx).to_vec();
        }
        // Legacy fallback: 4-directional square grid adjacency. Deprecated.
        self.neighbors_of_cell_grid(cell)
    }

    /// 4-directional square-grid adjacency (legacy fallback only).
    /// Kept for synthetic tests that build `GenContext` without a mesh.
    fn neighbors_of_cell_grid(&self, cell: u32) -> Vec<u32> {
        let n = self.cell_count();
        let idx = cell as usize;
        if idx >= n {
            return Vec::new();
        }
        let side = (n as f64).sqrt() as u32;
        if side == 0 {
            return Vec::new();
        }
        let r = cell / side;
        let c = cell % side;
        let mut neighbors = Vec::with_capacity(4);
        // North (r-1, c)
        if r > 0 {
            neighbors.push((r - 1) * side + c);
        }
        // South (r+1, c)
        if r + 1 < side && (r + 1) * side + c < n as u32 {
            neighbors.push((r + 1) * side + c);
        }
        // West (r, c-1)
        if c > 0 {
            neighbors.push(r * side + (c - 1));
        }
        // East (r, c+1)
        if c + 1 < side && r * side + (c + 1) < n as u32 {
            neighbors.push(r * side + (c + 1));
        }
        neighbors
    }
    

    /// Borrow the RNG as `&mut StdRng` — the type that modules receive.
    /// Modules do NOT hold an RNG reference in the trait; they receive it as
    /// a parameter to `run`.
    #[allow(dead_code)]
    pub fn _rng_placeholder(&self) -> Option<&StdRng> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_engine::TimelineParams;

    fn make_ctx(n: usize) -> GenContext {
        GenContext {
            pack: Pack::default(),
            cells_state: vec![1u32; n],
            cells_culture: vec![0u32; n],
            cells_religion: vec![0u32; n],
            cells_burg: vec![0u32; n],
            era_start: 0,
            era_end: 1,
            params: TimelineParams::default(),
            events: Vec::new(),
            next_id: 1,
            cells_h: vec![50u8; n],
            // No mesh topology — `make_ctx` tests exercise the grid fallback.
            cells: None,
        }
    }

    #[test]
    fn corner_cell_has_two_neighbors() {
        let side = 10u32;
        let n = (side * side) as usize;
        let ctx = make_ctx(n);
        let neighbors = ctx.neighbors_of_cell(0);
        assert_eq!(neighbors.len(), 2);
        assert!(neighbors.contains(&1));           // east
        assert!(neighbors.contains(&(side)));      // south
    }

    #[test]
    fn interior_cell_has_four_neighbors() {
        let side = 10u32;
        let n = (side * side) as usize;
        let ctx = make_ctx(n);
        let neighbors = ctx.neighbors_of_cell(55); // interior
        assert_eq!(neighbors.len(), 4);
    }

    #[test]
    fn edge_cell_has_three_neighbors() {
        let side = 10u32;
        let n = (side * side) as usize;
        let ctx = make_ctx(n);
        let neighbors = ctx.neighbors_of_cell(5); // top edge, not corner
        assert_eq!(neighbors.len(), 3);
    }

    #[test]
    fn out_of_bounds_returns_empty() {
        let side = 10u32;
        let n = (side * side) as usize;
        let ctx = make_ctx(n);
        assert!(ctx.neighbors_of_cell(n as u32).is_empty());
    }

    #[test]
    fn all_grid_neighbors_are_valid_indices() {
        let side = 10u32;
        let n = (side * side) as usize;
        let ctx = make_ctx(n);
        for cell in 0..n {
            for &nb in &ctx.neighbors_of_cell(cell as u32) {
                assert!((nb as usize) < n, "neighbor {} of cell {} out of bounds", nb, cell);
            }
        }
    }

    /// Build a `GenContext` backed by a real Voronoi mesh so the Delaunay
    /// neighbor path is exercised (not the grid fallback). Returns the context
    /// plus the cell count; the mesh is generated deterministically from `seed`.
    fn make_ctx_with_mesh(n: u32, seed: u32) -> (GenContext, usize) {
        let mesh = crate::mesh::build(n, seed);
        let n_cells = mesh.points.len();
        let mut ctx = make_ctx(n_cells);
        ctx.cells = Some(mesh.cells.clone());
        (ctx, n_cells)
    }

    /// On a real Voronoi mesh, `neighbors_of_cell` must return exactly the
    /// cells in the CSR `c` slice — i.e. the Delaunay edge-sharing neighbors.
    #[test]
    fn delaunay_neighbors_match_csr_slice() {
        let (ctx, n) = make_ctx_with_mesh(500, 42);
        let cells = ctx.cells.as_ref().expect("mesh should be set");
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
                assert!((nb as usize) < n, "neighbor {nb} of cell {cell} out of bounds (n={n})");
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

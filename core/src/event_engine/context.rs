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

    /// Borrow the RNG as `&mut StdRng` — the type that modules receive.
    /// Modules do NOT hold an RNG reference in the trait; they receive it as
    /// a parameter to `run`.
    #[allow(dead_code)]
    pub fn _rng_placeholder(&self) -> Option<&StdRng> {
        None
    }
}

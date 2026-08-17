//! Phase 4 Step 4.2 — `FoundExpand` event module.
//!
//! Each state may found a new burg in an unoccupied land cell within its
//! territory. The probability is `ctx.timeline.params.found_rate` per state per
//! year.
//! The new burg gets a `Found` event and is placed on a random unoccupied
//! land cell owned by the state.
//!
//! Extracted from the monolithic `event_engine.rs` (refactor §P4.2-modular).

use crate::entities::Burg;
use crate::event_engine::context::GenContext;
use crate::event_engine::EventModule;
use crate::timeline::{EntityType, EventKind, EventPayload};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::Rng;
pub struct FoundExpandModule;

impl EventModule for FoundExpandModule {
    fn name(&self) -> &'static str {
        "found_expand"
    }

    fn run(&self, ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
        if ctx.world.pack.states.is_empty() {
            return;
        }

        // Collect eligible state ids (owned data, no borrow on ctx).
        let eligible: Vec<(u32, i32)> = ctx
            .world
            .pack
            .states
            .iter()
            .filter(|s| {
                s.dissolved_year.is_none()
                    && (s.rural_pop + s.urban_pop) >= ctx.timeline.params.min_state_pop
            })
            .map(|s| (s.id, s.founded_year))
            .collect();

        for (state_id, _) in eligible {
            if !rng.gen_bool(ctx.timeline.params.found_rate) {
                continue;
            }

            if let Some(burg) = try_found_burg(ctx, state_id, rng, year) {
                let cell = burg.cell;
                let burg_id = burg.id;
                let population = burg.population;
                ctx.world.cells_burg[cell as usize] = burg_id;
                ctx.world.pack.burgs.push(burg);
                ctx.push_event(
                    year,
                    burg_id,
                    EntityType::Burg,
                    EventKind::Found,
                    EventPayload::Found { cell, population },
                );
            }
        }
    }
}

/// Try to found a new burg for `state_id` on an unoccupied land cell within
/// the state's territory. Returns the Burg if placement succeeds.
fn try_found_burg(ctx: &GenContext, state_id: u32, rng: &mut StdRng, year: i32) -> Option<Burg> {
    let spacing = ctx.timeline.params.min_burg_spacing;

    // Collect unoccupied land cells owned by this state that satisfy the
    // per-state burg spacing constraint (when `min_burg_spacing > 0`).
    let candidates: Vec<u32> = ctx
        .world
        .cells_state
        .iter()
        .enumerate()
        .filter_map(|(i, &s)| {
            let cell = i as u32;
            if s != state_id || !ctx.is_land(cell) || ctx.world.cells_burg[i] != 0 {
                return None;
            }
            if spacing > 0 && is_within_spacing(ctx, cell, state_id, spacing) {
                return None;
            }
            Some(cell)
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }

    let &cell = candidates.choose(rng)?;
    let id = ctx.next_burg_id();

    let culture = ctx.find_state(state_id).map(|s| s.culture).unwrap_or(0);

    // The first (undissolved) burg a state gains is its capital. Matching rule
    // in the projector keeps the reconstructed burg identical.
    let first_burg = !ctx
        .world
        .pack
        .burgs
        .iter()
        .any(|b| b.state == state_id && b.dissolved_year.is_none());

    Some(Burg {
        id,
        name: format!("Burg{}", id),
        cell,
        state: state_id,
        culture,
        religion: 0,
        population: ctx.timeline.params.founding_population,
        feature: 1,
        capital: if first_burg { 1 } else { 0 },
        founded_year: year,
        dissolved_year: None,
    })
}

/// Whether `cell` is within `spacing` graph-hops of an existing (undissolved)
/// burg owned by `state_id`. BFS over the mesh topology. Returns `false`
/// immediately when `spacing == 0` (constraint disabled).
fn is_within_spacing(ctx: &GenContext, cell: u32, state_id: u32, spacing: u32) -> bool {
    let n = ctx.cell_count();
    let burg_cells: Vec<u32> = ctx
        .world
        .pack
        .burgs
        .iter()
        .filter(|b| b.state == state_id && b.dissolved_year.is_none())
        .map(|b| b.cell)
        .collect();

    let mut visited = vec![false; n];
    for &b in &burg_cells {
        if let Some(v) = visited.get_mut(b as usize) {
            *v = true;
        }
    }
    let mut queue = burg_cells;

    let mut hops = 0;
    while !queue.is_empty() && hops < spacing {
        let mut next: Vec<u32> = Vec::new();
        for &c in &queue {
            for nb in ctx.neighbors_of_cell(c) {
                let ni = nb as usize;
                if ni < n && !visited[ni] {
                    if nb == cell {
                        return true;
                    }
                    visited[ni] = true;
                    next.push(nb);
                }
            }
        }
        queue = next;
        hops += 1;
    }
    false
}

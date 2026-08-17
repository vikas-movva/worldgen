//! Phase 4 Step 4.2 — `FoundExpand` event module.
//!
//! Each state may found a new burg in an unoccupied land cell within its
//! territory. The probability is `ctx.params.found_rate` per state per year.
//! The new burg gets a `Found` event and is placed on a random unoccupied
//! land cell owned by the state.
//!
//! Extracted from the monolithic `event_engine.rs` (refactor §P4.2-modular).

use crate::event_engine::context::GenContext;
use crate::event_engine::EventModule;
use crate::entities::Burg;
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
        if ctx.pack.states.is_empty() {
            return;
        }

        // Collect eligible state ids (owned data, no borrow on ctx).
        let eligible: Vec<(u32, i32)> = ctx
            .pack
            .states
            .iter()
            .filter(|s| {
                s.dissolved_year.is_none()
                    && (s.rural_pop + s.urban_pop) >= ctx.params.min_state_pop
            })
            .map(|s| (s.id, s.founded_year))
            .collect();

        for (state_id, _) in eligible {
            if !rng.gen_bool(ctx.params.found_rate) {
                continue;
            }

            if let Some(burg) = try_found_burg(ctx, state_id, rng, year) {
                let cell = burg.cell;
                let burg_id = burg.id;
                ctx.cells_burg[cell as usize] = burg_id;
                ctx.pack.burgs.push(burg);
                ctx.push_event(
                    year,
                    burg_id,
                    EntityType::Burg,
                    EventKind::Found,
                    EventPayload::Found { cell },
                );
            }
        }
    }
}

/// Try to found a new burg for `state_id` on an unoccupied land cell within
/// the state's territory. Returns the Burg if placement succeeds.
fn try_found_burg(ctx: &GenContext, state_id: u32, rng: &mut StdRng, year: i32) -> Option<Burg> {
    // Collect unoccupied land cells owned by this state.
    let candidates: Vec<u32> = ctx
        .cells_state
        .iter()
        .enumerate()
        .filter_map(|(i, &s)| {
            let cell = i as u32;
            if s == state_id && ctx.is_land(cell) && ctx.cells_burg[i] == 0 {
                Some(cell)
            } else {
                None
            }
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }

    let &cell = candidates.choose(rng)?;
    let id = ctx.next_burg_id();

    let culture = ctx
        .find_state(state_id)
        .map(|s| s.culture)
        .unwrap_or(0);

    Some(Burg {
        id,
        name: format!("Burg{}", id),
        cell,
        state: state_id,
        culture,
        religion: 0,
        population: ctx.params.founding_population,
        feature: 1,
        capital: 0,
        founded_year: year,
        dissolved_year: None,
    })
}

//! Phase 4 Step 4.2 — `Plague` event module.
//!
//! A plague reduces population in a state. Probability is
//! `ctx.params.plague_prob` per state per year.
//!
//! Extracted from the monolithic `event_engine.rs` (refactor §P4.2-modular).

use crate::event_engine::context::GenContext;
use crate::event_engine::EventModule;
use crate::timeline::{EntityType, EventKind, EventPayload};
use rand::rngs::StdRng;
use rand::Rng;

/// The plague event module.
pub struct PlagueModule;

impl EventModule for PlagueModule {
    fn name(&self) -> &'static str {
        "plague"
    }

    fn run(&self, ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
        // Collect eligible state ids (owned data).
        let eligible: Vec<u32> = ctx
            .pack
            .states
            .iter()
            .filter(|s| s.dissolved_year.is_none())
            .map(|s| s.id)
            .collect();

        for state_id in eligible {
            if rng.gen_bool(ctx.params.plague_prob) {
                let factor = 1.0 - (ctx.params.plague_mortality * rng.gen_range(0.5..=1.0));
                if let Some(s) = ctx.find_state_mut(state_id) {
                    s.rural_pop *= factor;
                    s.urban_pop *= factor;
                }
                ctx.push_event(
                    year,
                    state_id,
                    EntityType::State,
                    EventKind::Plague,
                    EventPayload::PopScalar { factor },
                );
            }
        }
    }
}

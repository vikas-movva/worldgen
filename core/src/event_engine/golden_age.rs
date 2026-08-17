//! Phase 4 Step 4.2 — `GoldenAge` event module.
//!
//! A golden age increases population growth in a state.
//! Probability is `ctx.params.golden_age_prob` per state per year.
//!
//! Extracted from the monolithic `event_engine.rs` (refactor §P4.2-modular).

use crate::event_engine::context::GenContext;
use crate::event_engine::EventModule;
use crate::timeline::{EntityType, EventKind, EventPayload};
use rand::rngs::StdRng;
use rand::Rng;

/// The golden age event module.
pub struct GoldenAgeModule;

impl EventModule for GoldenAgeModule {
    fn name(&self) -> &'static str {
        "golden_age"
    }

    fn run(&self, ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
        let eligible: Vec<u32> = ctx
            .pack
            .states
            .iter()
            .filter(|s| s.dissolved_year.is_none())
            .map(|s| s.id)
            .collect();

        for state_id in eligible {
            if rng.gen_bool(ctx.params.golden_age_prob) {
                let mult = ctx.params.golden_age_growth * rng.gen_range(0.8..=1.2);
                if let Some(s) = ctx.find_state_mut(state_id) {
                    s.rural_pop *= mult;
                    s.urban_pop *= mult;
                }
                ctx.push_event(
                    year,
                    state_id,
                    EntityType::State,
                    EventKind::GoldenAge,
                    EventPayload::PopScalar { factor: mult },
                );
            }
        }
    }
}

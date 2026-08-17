//! Phase 4 Step 4.2 — `GoldenAge` event module.
//!
//! A golden age increases population growth in a state.
//! Probability is `ctx.timeline.params.golden_age_prob` per state per year.
//!
//! Extracted from the monolithic `event_engine.rs` (refactor §P4.2-modular).

use crate::event_engine::context::{scale_state_pops, GenContext};
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
            .world
            .pack
            .states
            .iter()
            .filter(|s| s.dissolved_year.is_none())
            .map(|s| s.id)
            .collect();

        for state_id in eligible {
            if rng.gen_bool(ctx.timeline.params.golden_age_prob) {
                let mult = ctx.timeline.params.golden_age_growth * rng.gen_range(0.8..=1.2);
                scale_state_pops(ctx, state_id, mult);
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

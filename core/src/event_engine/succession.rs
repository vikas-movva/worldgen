//! Phase 4 Step 4.2 — `Succession` event module.
//!
//! A state gets a new ruler. This is primarily a narrative event (the state
//! persists, no cell change). Probability scales with the state's age.
//!
//! Extracted from the monolithic `event_engine.rs` (refactor §P4.2-modular).

use crate::event_engine::context::GenContext;
use crate::event_engine::EventModule;
use crate::timeline::{EntityType, EventKind, EventPayload};
use rand::rngs::StdRng;
use rand::Rng;

/// Succession age threshold in years. A state founded more than
/// `SUCCESSION_AGE` years ago may roll for succession each year.
const SUCCESSION_AGE: i32 = 50;

/// The succession event module.
pub struct SuccessionModule;

impl EventModule for SuccessionModule {
    fn name(&self) -> &'static str {
        "succession"
    }

    fn run(&self, ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
        // Collect eligible state ids + founded_year (owned data).
        let eligible: Vec<(u32, i32)> = ctx
            .pack
            .states
            .iter()
            .filter(|s| {
                s.dissolved_year.is_none()
                    && s.founded_year < year
                    && year - s.founded_year > 50
            })
            .map(|s| (s.id, s.founded_year))
            .collect();

        for (state_id, founded_year) in &eligible {
            let age = year - founded_year;
            let prob = (0.02 * (age as f64 / 100.0).min(5.0)).min(0.15);

            if rng.gen_bool(prob) {
                let disputed = rng.gen_bool(0.1);
                if disputed {
                    ctx.push_event(
                        year,
                        *state_id,
                        EntityType::State,
                        EventKind::Succession,
                        EventPayload::Succession {
                            heir_name: Some(format!("Heir{}", state_id)),
                        },
                    );
                } else {
                    ctx.push_event(
                        year,
                        *state_id,
                        EntityType::State,
                        EventKind::Succession,
                        EventPayload::Succession { heir_name: None },
                    );
                }
            }
        }

        // CivilWar: disputed successions sometimes trigger civil wars.
        for (state_id, _) in &eligible {
            if rng.gen_bool(0.03) {
                ctx.push_event(
                    year,
                    *state_id,
                    EntityType::State,
                    EventKind::CivilWar,
                    EventPayload::None,
                );
            }
        }
    }
}

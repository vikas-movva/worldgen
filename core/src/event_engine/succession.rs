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

/// Deterministic heir-name pool for disputed successions. The same state
/// always draws the same heir name (indexed by `state_id`), so a given seed
/// produces a stable narrative.
const HEIR_NAMES: [&str; 12] = [
    "Aldric",
    "Bertha",
    "Cedric",
    "Dorian",
    "Edelweiss",
    "Florian",
    "Giselda",
    "Hakon",
    "Isolde",
    "Jorund",
    "Kyrie",
    "Lothar",
];

/// A deterministic heir name for `state_id`, from a small fixed pool.
fn heir_name(state_id: u32) -> String {
    let idx = (state_id as usize) % HEIR_NAMES.len();
    HEIR_NAMES[idx].to_string()
}

/// The succession event module.
pub struct SuccessionModule;

impl EventModule for SuccessionModule {
    fn name(&self) -> &'static str {
        "succession"
    }

    fn run(&self, ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
        // Collect eligible state ids + founded_year (owned data).
        let eligible: Vec<(u32, i32)> = ctx
            .world
            .pack
            .states
            .iter()
            .filter(|s| {
                s.dissolved_year.is_none()
                    && s.founded_year < year
                    && year - s.founded_year > SUCCESSION_AGE
            })
            .map(|s| (s.id, s.founded_year))
            .collect();

        // Track which states had a *disputed* succession this year; a disputed
        // succession is the trigger for a civil war, not an independent roll.
        let mut disputed: Vec<u32> = Vec::new();

        for (state_id, founded_year) in &eligible {
            let age = year - founded_year;
            let prob = (0.02 * (age as f64 / 100.0).min(5.0)).min(0.15);

            if rng.gen_bool(prob) {
                let is_disputed = rng.gen_bool(0.1);
                if is_disputed {
                    disputed.push(*state_id);
                }
                ctx.push_event(
                    year,
                    *state_id,
                    EntityType::State,
                    EventKind::Succession,
                    EventPayload::Succession {
                        heir_name: if is_disputed {
                            Some(heir_name(*state_id))
                        } else {
                            None
                        },
                    },
                );
            }
        }

        // CivilWar: disputed successions sometimes degenerate into civil war.
        for state_id in disputed {
            if rng.gen_bool(0.03) {
                ctx.push_event(
                    year,
                    state_id,
                    EntityType::State,
                    EventKind::CivilWar,
                    EventPayload::None,
                );
            }
        }
    }
}

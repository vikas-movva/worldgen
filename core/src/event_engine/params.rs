//! Phase 4 Step 4.2 — Event engine parameters.
//!
//! Tunable parameters for the event generator. All fields have deterministic
//! defaults so that omitting `opts` in a `generate_timeline` call produces a
//! reproducible world.
//!
//! This was extracted from the monolithic `event_engine.rs` into its own module
//! so that parameter definitions live independently of the module implementations
//! and the engine loop (refactor §P4.2-modular).

use serde::{Deserialize, Serialize};

/// Tunable parameters for the event generator. All fields have deterministic
/// defaults so that omitting `opts` in a `generate_timeline` call produces a
/// reproducible world.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TimelineParams {
    /// Era year range `[era_start, era_end)`. Events are generated only within
    /// this interval. Default: 0..1000.
    pub era_start: i32,
    pub era_end: i32,
    /// Probability per eligible state per year of founding a new burg
    /// (`Found` event). 0.0 = never, 1.0 = every year. Default 0.05.
    pub found_rate: f64,
    /// Probability per eligible (at-war-eligible) state per year of war
    /// initiation. Default 0.08.
    pub war_rate: f64,
    /// Probability per land state per year of a plague outbreak. Default 0.02.
    pub plague_prob: f64,
    /// Probability per land state per year of a golden age. Default 0.05.
    pub golden_age_prob: f64,
    /// Probability per existing (parent) religion per eligible year of a
    /// schism. Default 0.015.
    pub schism_prob: f64,
    /// Probability per (culture, neighbor-culture) pair per year of migration
    /// pressure. Default 0.03.
    pub migration_prob: f64,
    /// Expected burg population (in thousands) at founding. Default 5.0.
    pub founding_population: f64,
    /// Expected plague mortality fraction (0..1). Default 0.25.
    pub plague_mortality: f64,
    /// Expected golden-age growth multiplier. Default 1.15.
    pub golden_age_growth: f64,
    /// Expected schism follower fraction (0..1). Default 0.3.
    pub schism_fraction: f64,
    /// Expected migration fraction (0..1). Default 0.1.
    pub migration_fraction: f64,
    /// Minimum world population (in thousands) for an event to trigger.
    /// Guards tiny test packs from over-firing. Default 1.0.
    pub min_state_pop: f64,
    /// Minimum graph distance (in cells) a newly-founded burg must keep from
    /// any existing burg owned by the same state. `0` disables the spacing
    /// constraint (spatial de-duplication only). Default 0.
    pub min_burg_spacing: u32,
    /// Random number generator seed override. If 0, the engine derives a
    /// sub-stream from the timeline seed. Default: 0 (derive).
    pub rng_override: u64,
}

impl Default for TimelineParams {
    fn default() -> Self {
        TimelineParams {
            era_start: 0,
            era_end: 1000,
            found_rate: 0.05,
            war_rate: 0.08,
            plague_prob: 0.02,
            golden_age_prob: 0.05,
            schism_prob: 0.015,
            migration_prob: 0.03,
            founding_population: 5.0,
            plague_mortality: 0.25,
            golden_age_growth: 1.15,
            schism_fraction: 0.3,
            migration_fraction: 0.1,
            min_state_pop: 1.0,
            min_burg_spacing: 0,
            rng_override: 0,
        }
    }
}

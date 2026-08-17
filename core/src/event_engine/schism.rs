//! Phase 4 Step 4.2 — `Schism` event module.
//!
//! A parent religion splits into a child denomination. Probability is
//! `ctx.timeline.params.schism_prob` per parent religion per year.
//!
//! Extracted from the monolithic `event_engine.rs` (refactor §P4.2-modular).

use crate::entities::Religion;
use crate::event_engine::context::GenContext;
use crate::event_engine::EventModule;
use crate::timeline::{EntityType, EventKind, EventPayload, SchismPayload};
use rand::rngs::StdRng;
use rand::Rng;

/// The schism event module.
pub struct SchismModule;

impl EventModule for SchismModule {
    fn name(&self) -> &'static str {
        "schism"
    }

    fn run(&self, ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
        // Collect parent religion ids + their data (owned, no borrow on ctx).
        let parents: Vec<(u32, String, u32, u8, String, f64)> = ctx
            .world
            .pack
            .religions
            .iter()
            .filter(|r| r.parent.is_none() && r.followers > 1000.0)
            .map(|r| {
                (
                    r.id,
                    r.name.clone(),
                    r.center_cell,
                    r.type_code,
                    r.expansion_mode.clone(),
                    r.followers,
                )
            })
            .collect();

        for (
            parent_id,
            parent_name,
            parent_center,
            parent_type_code,
            parent_mode,
            parent_followers,
        ) in parents
        {
            if !rng.gen_bool(ctx.timeline.params.schism_prob) {
                continue;
            }

            let fraction = ctx.timeline.params.schism_fraction * rng.gen_range(0.5..=1.0);
            let child_id = ctx.next_religion_id();

            // Reassign followers from parent to child.
            if let Some(p) = ctx.find_religion_mut(parent_id) {
                p.followers = parent_followers * (1.0 - fraction);
            }

            let child = Religion {
                id: child_id,
                name: format!("{}-ism", parent_name),
                color: ctx.find_religion(parent_id).map(|r| r.color).unwrap_or(0),
                center_cell: parent_center,
                parent: Some(parent_id),
                followers: parent_followers * fraction,
                type_code: parent_type_code,
                expansion_mode: parent_mode,
                founded_year: year,
                dissolved_year: None,
            };

            ctx.world.pack.religions.push(child);

            ctx.push_event(
                year,
                parent_id,
                EntityType::Religion,
                EventKind::Schism,
                EventPayload::Schism {
                    payload: SchismPayload {
                        follower_fraction: fraction,
                        child_religion_id: child_id,
                    },
                },
            );
        }
    }
}

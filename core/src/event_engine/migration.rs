//! Phase 4 Step 4.2 — `Migrate` event module.
//!
//! A culture spreads to adjacent cells. Probability is
//! `ctx.timeline.params.migration_prob` per culture per year.
//!
//! Extracted from the monolithic `event_engine.rs` (refactor §P4.2-modular).

use crate::event_engine::context::GenContext;
use crate::event_engine::EventModule;
use crate::timeline::{EntityType, EventKind, EventPayload, MigratePayload};
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::Rng;

/// The migration event module.
pub struct MigrationModule;

impl EventModule for MigrationModule {
    fn name(&self) -> &'static str {
        "migration"
    }

    fn run(&self, ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
        if ctx.world.pack.cultures.is_empty() {
            return;
        }

        // Collect cultures with sufficient cell_count (owned data).
        let eligible: Vec<u32> = ctx
            .world
            .pack
            .cultures
            .iter()
            .filter(|c| c.cell_count > 10)
            .map(|c| c.id)
            .collect();

        for culture_id in eligible {
            if !rng.gen_bool(ctx.timeline.params.migration_prob) {
                continue;
            }

            // Find border cells of this culture: land cells owned by this
            // culture that are directly adjacent (mesh topology) to a land
            // cell owned by a different culture.
            let border_cells: Vec<u32> = ctx
                .world
                .cells_culture
                .iter()
                .enumerate()
                .filter(|(i, &c)| c as u32 == culture_id && ctx.is_land(*i as u32))
                .filter_map(|(i, _)| {
                    let cell = i as u32;
                    let has_foreign = ctx.neighbors_of_cell(cell).iter().any(|&nb| {
                        let ni = nb as usize;
                        ni < ctx.cell_count()
                            && ctx.is_land(nb)
                            && ctx.world.cells_culture[ni] != 0
                            && ctx.world.cells_culture[ni] != culture_id
                    });
                    if has_foreign {
                        Some(cell)
                    } else {
                        None
                    }
                })
                .collect();

            if border_cells.is_empty() {
                continue;
            }

            let &target_cell = border_cells.choose(rng).unwrap();

            // Neighboring cultures of the chosen border cell (the possible
            // migration targets), via the mesh topology.
            let mut target_ids: Vec<u32> = Vec::new();
            for nb in ctx.neighbors_of_cell(target_cell) {
                let ni = nb as usize;
                if ni < ctx.cell_count() && ctx.is_land(nb) {
                    let nc = ctx.world.cells_culture[ni];
                    if nc != 0 && nc != culture_id && !target_ids.contains(&nc) {
                        target_ids.push(nc);
                    }
                }
            }

            if target_ids.is_empty() {
                continue;
            }

            let target_id = *target_ids.choose(rng).unwrap();
            let fraction = ctx.timeline.params.migration_fraction * rng.gen_range(0.5..=1.0);

            // Transfer up to `fraction` of the culture's distinct border cells
            // to the target culture. Pick with replacement, then de-duplicate
            // so the event lists and the `cell_count` accounting reflect the
            // actual distinct cells moved.
            let n_requested =
                ((ctx.cell_count() as f64 * fraction) as usize).min(border_cells.len());
            let mut cells_to_transfer: Vec<u32> = (0..n_requested)
                .filter_map(|_| border_cells.choose(rng).copied())
                .collect();
            cells_to_transfer.sort_unstable();
            cells_to_transfer.dedup();
            let moved = cells_to_transfer.len() as u32;

            if moved == 0 {
                continue;
            }

            for &cell in &cells_to_transfer {
                ctx.world.cells_culture[cell as usize] = target_id;
            }

            // Update the cultures' cell_count to match the distinct cells moved.
            if let Some(src) = ctx.find_culture_mut(culture_id) {
                src.cell_count = src.cell_count.saturating_sub(moved);
            }
            if let Some(dst) = ctx.find_culture_mut(target_id) {
                dst.cell_count = dst.cell_count.saturating_add(moved);
            }

            ctx.push_event(
                year,
                culture_id,
                EntityType::Culture,
                EventKind::Migrate,
                EventPayload::Migrate {
                    payload: MigratePayload {
                        cells: cells_to_transfer,
                        target_id,
                    },
                },
            );
        }
    }
}

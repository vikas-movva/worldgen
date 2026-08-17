//! Phase 4 Step 4.2 — `Migrate` event module.
//!
//! A culture spreads to adjacent cells. Probability is
//! `ctx.params.migration_prob` per culture per year.
//!
//! Extracted from the monolithic `event_engine.rs` (refactor §P4.2-modular).

use crate::event_engine::context::GenContext;
use crate::event_engine::EventModule;
use crate::timeline::{EntityType, EventKind, EventPayload, MigratePayload};
use rand::rngs::StdRng;
use rand::Rng;
use rand::seq::SliceRandom;

/// The migration event module.
pub struct MigrationModule;

impl EventModule for MigrationModule {
    fn name(&self) -> &'static str {
        "migration"
    }

    fn run(&self, ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
        if ctx.pack.cultures.is_empty() {
            return;
        }

        // Collect cultures with sufficient cell_count (owned data).
        let eligible: Vec<(u32, u32)> = ctx
            .pack
            .cultures
            .iter()
            .filter(|c| c.cell_count > 10)
            .map(|c| (c.id, c.cell_count))
            .collect();

        for (culture_id, _cell_count) in eligible {
            if !rng.gen_bool(ctx.params.migration_prob) {
                continue;
            }

            // Find border cells of this culture (cells owned by this culture
            // that are adjacent to cells with a different owner).
            let border_cells: Vec<u32> = ctx
                .cells_culture
                .iter()
                .enumerate()
                .filter_map(|(i, &c)| {
                    if c as u32 != culture_id {
                        return None;
                    }
                    let cell = i as u32;
                    if !ctx.is_land(cell) {
                        return None;
                    }
                    let w = 20;
                    let start = (i as usize).saturating_sub(w / 2);
                    let end = ((i as usize) + w / 2 + 1).min(ctx.cell_count());
                    for j in start..end {
                        if j != i && ctx.cells_culture[j] != c && ctx.is_land(j as u32) {
                            return Some(cell);
                        }
                    }
                    None
                })
                .collect();

            if border_cells.is_empty() {
                continue;
            }

            let &target_cell = border_cells.choose(rng).unwrap();
            let current_culture = ctx.cells_culture[target_cell as usize];

            // Find adjacent cells with a different culture.
            let w = 20;
            let start = (target_cell as usize).saturating_sub(w / 2);
            let end = ((target_cell as usize) + w / 2 + 1).min(ctx.cell_count());
            let mut target_ids: Vec<u32> = Vec::new();
            for j in start..end {
                if j != target_cell as usize && ctx.is_land(j as u32) {
                    let nc = ctx.cells_culture[j];
                    if nc != current_culture && nc != 0 {
                        if !target_ids.contains(&nc) {
                            target_ids.push(nc);
                        }
                    }
                }
            }

            if target_ids.is_empty() {
                continue;
            }

            let target_id = *target_ids.choose(rng).unwrap();
            let fraction = ctx.params.migration_fraction * rng.gen_range(0.5..=1.0);

            // Transfer some cells to the target culture.
            let n_transfer = ((ctx.cell_count() as f64 * fraction) as usize).min(border_cells.len());
            let cells_to_transfer: Vec<u32> = (0..n_transfer)
                .filter_map(|_| border_cells.choose(rng).copied())
                .collect();

            for &cell in &cells_to_transfer {
                ctx.cells_culture[cell as usize] = target_id;
            }

            // Update the cultures' cell_count.
            if let Some(src) = ctx.find_culture_mut(culture_id) {
                let lost = cells_to_transfer.len() as u32;
                src.cell_count = src.cell_count.saturating_sub(lost);
            }
            if let Some(dst) = ctx.find_culture_mut(target_id) {
                let gained = cells_to_transfer.len() as u32;
                dst.cell_count += gained;
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

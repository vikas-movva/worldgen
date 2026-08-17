//! Phase 4 Step 4.2 — `War` event module.
//!
//! Two land states that are neighbors (share at least one bordering cell)
//! may go to war. With probability `ctx.params.war_rate` per eligible state
//! per year, the attacker invades a bordering cell owned by a target state.
//! On success, the target cell flips ownership to the attacker and a `War`
//! event is emitted.
//!
//! Extracted from the monolithic `event_engine.rs` (refactor §P4.2-modular).

use crate::event_engine::context::GenContext;
use crate::event_engine::EventModule;
use crate::timeline::{EntityType, EventKind, EventPayload, WarOutcome};
use rand::rngs::StdRng;
use rand::Rng;

/// The war event module.
pub struct WarModule;

impl EventModule for WarModule {
    fn name(&self) -> &'static str {
        "war"
    }

    fn run(&self, ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
        // Collect active state ids + their owned land cells, snapshotting
        // so we can iterate without borrow conflicts.
        let states: Vec<(u32, Vec<u32>)> = ctx
            .pack
            .states
            .iter()
            .filter(|s| s.dissolved_year.is_none())
            .map(|s| {
                let cells = ctx.cells_of_state(s.id);
                (s.id, cells)
            })
            .collect();

        if states.len() < 2 {
            return;
        }

        for (attacker_id, attacker_cells) in &states {
            // Skip if this state lacks the population threshold for war.
            let att_pop = ctx
                .find_state(*attacker_id)
                .map(|s| s.rural_pop + s.urban_pop)
                .unwrap_or(0.0);

            if att_pop < ctx.params.min_state_pop {
                continue;
            }

            if !rng.gen_bool(ctx.params.war_rate) {
                continue;
            }

            // Find bordering cells of the attacker (cells adjacent to a cell
            // not owned by the attacker but owned by another state).
            let border_cells = find_border_cells(ctx, attacker_cells);

            if border_cells.is_empty() {
                continue;
            }

            // Pick a random border cell and attempt to conquer it.
            let border_cell = border_cells[rng.gen_range(0..border_cells.len())];
            let target_state_id = ctx.cells_state[border_cell as usize];

            // Don't attack ourselves or nobody.
            if target_state_id == 0 || target_state_id == *attacker_id {
                continue;
            }

            // Check the target is still active (not dissolved).
            let target_active = ctx
                .pack
                .states
                .iter()
                .any(|s| s.id == target_state_id && s.dissolved_year.is_none());

            if !target_active {
                continue;
            }

            // Conquer: flip ownership.
            ctx.cells_state[border_cell as usize] = *attacker_id;

            // Build the war outcome payload — the attacker wins.
            let outcome = WarOutcome {
                result: 0, // attacker wins
                attrition: 0.3,
                conquered_cells: vec![border_cell],
            };

            ctx.push_event(
                year,
                *attacker_id,
                EntityType::State,
                EventKind::War,
                EventPayload::War {
                    opponent_state_id: target_state_id,
                    outcome,
                },
            );
        }
    }
}

/// Find cells adjacent to the attacker that are owned by a different state —
/// these are the conquest targets (cells the attacker can flip).
fn find_border_cells(ctx: &GenContext, owned_cells: &[u32]) -> Vec<u32> {
    let n = ctx.cell_count();
    let mut border = Vec::new();

    for &cell in owned_cells {
        let idx = cell as usize;
        if idx >= n {
            continue;
        }
        let owner = ctx.cells_state[idx];
        if owner == 0 {
            continue;
        }

        // Check 4-neighbors (grid adjacency). The grid is treated as a 2D
        // layout with `side` computed from the cell count. We use a
        // rectangular adjacency; if a neighbor wraps around the grid edge
        // we skip it.
        let side = (n as f64).sqrt() as u32;
        if side == 0 {
            continue;
        }

        let r = idx as u32 / side;
        let c = idx as u32 % side;

        let neighbors = [(r.wrapping_sub(1), c), (r, c.wrapping_sub(1)), (r, c + 1), (r + 1, c)];

        for (nr, nc) in neighbors {
            if nr >= side || nc >= side {
                continue;
            }
            let nidx = (nr * side + nc) as usize;
            if nidx >= n {
                continue;
            }
            let n_owner = ctx.cells_state[nidx];
            // A border target is a neighbor cell owned by a different,
            // non-zero state. We want to conquer these cells.
            if n_owner != 0 && n_owner != owner {
                border.push(nidx as u32);
                break;
            }
        }
    }

    border
}

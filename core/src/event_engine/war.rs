//! Phase 4 Step 4.2 — `War` event module.
//!
//! Active land states may initiate wars against neighboring states.
//! With probability `ctx.timeline.params.war_rate` per eligible state per year,
//! an attacker selects one bordering enemy cell and resolves a battle.
//!
//! A successful invasion transfers the entire province of the disputed cell to
//! the attacker and emits a `War` event.
//!
//! Important invariants:
//! - Cell adjacency comes from the world's actual topology, not a square grid.
//! - Owned cells are recomputed after every mutation.
//! - A battle may fail.
//! - `war_rate` is validated before use.
//!
//! Extracted from `event_engine.rs` (refactor §P4.2-modular).

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
        // A probability outside [0, 1] is invalid for rand::gen_bool().
        let war_rate = ctx.timeline.params.war_rate.clamp(0.0, 1.0);

        if war_rate <= 0.0 {
            return;
        }

        // Snapshot only the state IDs. Their territory must be looked up
        // again for every iteration because wars mutate ownership.
        let state_ids: Vec<u32> = ctx
            .world
            .pack
            .states
            .iter()
            .filter(|state| state.dissolved_year.is_none())
            .map(|state| state.id)
            .collect();

        if state_ids.len() < 2 {
            return;
        }

        for attacker_id in state_ids {
            // The state may have been dissolved by another event later in
            // the pipeline, so always re-check current state existence.
            let attacker_pop = match ctx.find_state(attacker_id) {
                Some(state) if state.dissolved_year.is_none() => state.rural_pop + state.urban_pop,
                _ => continue,
            };

            if attacker_pop < ctx.timeline.params.min_state_pop {
                continue;
            }

            // war_rate determines whether this state attempts a war this year.
            if !rng.gen_bool(war_rate) {
                continue;
            }

            // IMPORTANT:
            // Recompute ownership here instead of using a snapshot created
            // before previous wars mutated ctx.world.cells_state.
            let attacker_cells = ctx.cells_of_state(attacker_id);

            let border_cells = find_border_cells(ctx, attacker_id, &attacker_cells);

            if border_cells.is_empty() {
                continue;
            }

            let target_cell = border_cells[rng.gen_range(0..border_cells.len())] as usize;

            if target_cell >= ctx.world.cells_state.len() {
                continue;
            }

            let target_state_id = ctx.world.cells_state[target_cell];

            // The ownership may theoretically have changed between finding
            // the border and selecting it. Re-validate before attacking.
            if target_state_id == 0 || target_state_id == attacker_id {
                continue;
            }

            let target_active = ctx
                .world
                .pack
                .states
                .iter()
                .any(|state| state.id == target_state_id && state.dissolved_year.is_none());

            if !target_active {
                continue;
            }

            // Resolve the actual battle.
            let outcome = resolve_battle(ctx, attacker_id, target_state_id, rng);

            let conquered = outcome.result == 0;

            if conquered {
                // On victory, the entire province of the disputed cell is
                // ceded to the attacker — not just the border cell that was
                // contested. We look up the province id of `target_cell` in
                // `world.cells_province`, then transfer every cell whose
                // `world.cells_province` matches.
                let target_province = ctx.world.cells_province[target_cell];

                // Guard: province `0` is the "unassigned" sentinel. Treating
                // it as a real province would cede *every* unassigned-province
                // cell the defender owns — a pathological multi-cell conquest.
                // When the disputed cell has no province, fall back to ceding
                // just the single contested cell.
                let province_cells: Vec<u32> = if target_province == 0 {
                    vec![target_cell as u32]
                } else {
                    (0..ctx.cell_count())
                        .filter(|&c| {
                            let idx = c as usize;
                            ctx.world.cells_province[idx] == target_province
                                && ctx.world.cells_state[idx] == target_state_id
                        })
                        .map(|c| c as u32)
                        .collect()
                };

                // Transfer all province cells to the attacker.
                for &cell in &province_cells {
                    ctx.world.cells_state[cell as usize] = attacker_id;
                }

                // A defender that loses every cell is dissolved. This mirrors
                // the projector's post-cession rule, so the working context
                // and the projected world agree.
                if ctx.cells_of_state(target_state_id).is_empty() {
                    if let Some(def) = ctx.find_state_mut(target_state_id) {
                        def.dissolved_year = def.dissolved_year.or(Some(year));
                    }
                }

                ctx.push_event(
                    year,
                    attacker_id,
                    EntityType::State,
                    EventKind::War,
                    EventPayload::War {
                        opponent_state_id: target_state_id,
                        outcome: WarOutcome {
                            result: outcome.result,
                            attrition: outcome.attrition,
                            conquered_cells: province_cells,
                        },
                    },
                );
            } else {
                ctx.push_event(
                    year,
                    attacker_id,
                    EntityType::State,
                    EventKind::War,
                    EventPayload::War {
                        opponent_state_id: target_state_id,
                        outcome: WarOutcome {
                            result: outcome.result,
                            attrition: outcome.attrition,
                            conquered_cells: Vec::new(),
                        },
                    },
                );
            }
        }
    }
}

/// Find enemy cells adjacent to cells currently owned by `attacker_id`.
///
/// This function deliberately does NOT assume the world is a square grid.
/// `cell_neighbors()` must use the actual Voronoi/Delaunay topology.
fn find_border_cells(ctx: &GenContext, attacker_id: u32, owned_cells: &[u32]) -> Vec<u32> {
    let cell_count = ctx.cell_count();
    let mut border = Vec::new();

    for &cell in owned_cells {
        let idx = cell as usize;

        if idx >= cell_count || idx >= ctx.world.cells_state.len() {
            continue;
        }

        // The ownership snapshot may be stale if another war occurred earlier.
        // Never use a cell that no longer belongs to this attacker.
        if ctx.world.cells_state[idx] != attacker_id {
            continue;
        }

        for neighbor in cell_neighbors(ctx, cell) {
            let neighbor_idx = neighbor as usize;

            if neighbor_idx >= cell_count || neighbor_idx >= ctx.world.cells_state.len() {
                continue;
            }

            let neighbor_owner = ctx.world.cells_state[neighbor_idx];

            if neighbor_owner != 0 && neighbor_owner != attacker_id {
                border.push(neighbor);
            }
        }
    }

    border.sort_unstable();
    border.dedup();
    border
}

/// Return the actual neighboring cells of a cell.
///
/// Replace the body of this function with the topology accessor already
/// exposed by your Voronoi/world representation.
///
/// For a Voronoi mesh this should return cells sharing an edge with `cell`.
fn cell_neighbors(ctx: &GenContext, cell: u32) -> Vec<u32> {
    ctx.neighbors_of_cell(cell)
}

/// Resolve a battle between two states.
///
/// `war_rate` controls whether a war starts. It does NOT determine whether
/// the attacker wins.
///
/// The current MVP resolution uses relative population strength plus a small
/// random factor. This keeps the model deterministic given the RNG while
/// avoiding guaranteed victories.
fn resolve_battle(
    ctx: &GenContext,
    attacker_id: u32,
    defender_id: u32,
    rng: &mut StdRng,
) -> BattleResult {
    let attacker_power = state_power(ctx, attacker_id);
    let defender_power = state_power(ctx, defender_id);

    if attacker_power <= 0.0 {
        return BattleResult {
            result: 1,
            attrition: 0.3,
        };
    }

    if defender_power <= 0.0 {
        return BattleResult {
            result: 0,
            attrition: 0.3,
        };
    }

    // Population ratio determines the baseline probability of victory.
    //
    // attacker_power / (attacker_power + defender_power)
    // naturally falls into [0, 1].
    let base_win_probability = attacker_power / (attacker_power + defender_power);

    // Small random variation prevents identical states from always producing
    // the same result while keeping population as the dominant factor.
    let random_factor: f64 = rng.gen_range(0.90..=1.10);

    let win_probability = (base_win_probability * random_factor).clamp(0.05, 0.95);

    let attacker_wins = rng.gen_bool(win_probability);

    BattleResult {
        result: if attacker_wins { 0 } else { 1 },
        attrition: if attacker_wins { 0.30 } else { 0.15 },
    }
}

/// Compute a state's military/economic power.
///
/// For now this uses population as the only strength signal.
/// This is intentionally isolated so military strength can later incorporate
/// wealth, technology, geography, infrastructure, etc.
fn state_power(ctx: &GenContext, state_id: u32) -> f64 {
    ctx.find_state(state_id)
        .filter(|state| state.dissolved_year.is_none())
        .map(|state| (state.rural_pop + state.urban_pop).max(0.0))
        .unwrap_or(0.0)
}

struct BattleResult {
    /// 0 = attacker wins, 1 = attacker loses.
    result: u8,
    attrition: f64,
}

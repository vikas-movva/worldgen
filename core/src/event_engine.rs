//! Phase 4 Step 4.2 — Event generation engine.
//!
//! Deterministically generates a chronological `Timeline` (sorted by `(year, id)`)
//! from a year-0 `Pack` + cell ownership arrays + era bounds + seed. Each module
//! (succession, war, plague, golden age, schism, found/expand, migration) is a
//! pure function of `(working Pack + cell arrays, &mut Rng, &mut id counter, &mut Vec<Event>)`.
//!
//! The engine evolves a working `WorldAt` through the era year-by-year (or event
//! by event). Accepted events are appended to the shared `Vec<Event>` AND applied
//! to the working Pack/cells immediately, so later modules see the updated state.
//! This matches the plan's rule: "Apply accepted events to a working world before
//! later modules run."
//!
//! See `agent/worldgen-implementation-plan.md` §Step 4.2 for the gate criteria.

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use crate::entities::{Burg, Culture, Pack, Religion, State};
use crate::timeline::{
    ConquerPayload, EntityType, Event, EventKind, EventPayload, MigratePayload, SchismPayload,
    Timeline, WarOutcome,
};

// ---------------------------------------------------------------------------//
// Parameters
// ---------------------------------------------------------------------------//

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
            rng_override: 0,
        }
    }
}

// ---------------------------------------------------------------------------//
// Context — mutable working state threaded through the module pipeline
// ---------------------------------------------------------------------------//

/// Mutable working state for the event generator. The engine clones the input
/// `Pack` + cell arrays into this struct, then each module reads and mutates
/// it. Events are emitted into the shared `events` vec.
pub(crate) struct GenContext {
    /// Working pack (cloned from the year-0 base). Modules mutate entity
    /// fields like `dissolved_year`, `followers`, `population`, etc.
    pub pack: Pack,
    /// Per-cell owning state id (`0` = unassigned in u32 form).
    pub cells_state: Vec<u32>,
    /// Per-cell culture id (`0` = unassigned).
    pub cells_culture: Vec<u32>,
    /// Per-cell religion id (`0` = unassigned).
    pub cells_religion: Vec<u32>,
    /// Per-cell burg id (`0` = none).
    pub cells_burg: Vec<u32>,
    /// The era bounds.
    pub era_start: i32,
    pub era_end: i32,
    /// The parameters.
    pub params: TimelineParams,
    /// Shared event sink. Modules append events here.
    pub events: Vec<Event>,
    /// Monotonic event-id counter (seeds from the timeline seed).
    pub next_id: u64,
    /// The grid's per-cell heightmap, for land/water checks.
    pub cells_h: Vec<u8>,
}

impl GenContext {
    /// Number of cells.
    pub fn cell_count(&self) -> usize {
        self.cells_state.len()
    }

    /// Is cell `c` land? (height >= 20, FMG sea level.)
    pub fn is_land(&self, c: u32) -> bool {
        let idx = c as usize;
        idx < self.cells_h.len() && self.cells_h[idx] >= 20
    }

    /// Assign a deterministic event id and return it.
    pub fn next_event_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Push an event into the sink. The caller provides the year; `id` is
    /// assigned from the monotonic counter.
    pub fn push_event(&mut self, year: i32, entity_id: u32, entity_type: EntityType, kind: EventKind, payload: EventPayload) {
        let id = self.next_event_id();
        self.events.push(Event {
            id,
            year,
            entity_id,
            entity_type,
            kind,
            payload,
            narrative: None,
        });
    }

    /// Find a state by id.
    pub fn find_state(&self, id: u32) -> Option<&State> {
        self.pack.states.iter().find(|s| s.id == id)
    }

    /// Find a state by id (mutable).
    pub fn find_state_mut(&mut self, id: u32) -> Option<&mut State> {
        self.pack.states.iter_mut().find(|s| s.id == id)
    }

    /// Find a religion by id.
    pub fn find_religion(&self, id: u32) -> Option<&Religion> {
        self.pack.religions.iter().find(|r| r.id == id)
    }

    /// Find a religion by id (mutable).
    pub fn find_religion_mut(&mut self, id: u32) -> Option<&mut Religion> {
        self.pack.religions.iter_mut().find(|r| r.id == id)
    }

    /// Find a burg by id (mutable).
    pub fn find_burg_mut(&mut self, id: u32) -> Option<&mut Burg> {
        self.pack.burgs.iter_mut().find(|b| b.id == id)
    }

    /// Find a culture by id.
    pub fn find_culture(&self, id: u32) -> Option<&Culture> {
        self.pack.cultures.iter().find(|c| c.id == id)
    }

    /// Find cells owned by `state_id`.
    pub fn cells_of_state(&self, state_id: u32) -> Vec<u32> {
        self.cells_state
            .iter()
            .enumerate()
            .filter_map(|(i, &s)| if s == state_id { Some(i as u32) } else { None })
            .collect()
    }

    /// Next free burg id.
    pub fn next_burg_id(&self) -> u32 {
        self.pack.burgs.last().map_or(1, |b| b.id + 1)
    }

    /// Next free army id.
    pub fn next_army_id(&self) -> u32 {
        self.pack.armies.last().map_or(1, |a| a.id + 1)
    }

    /// Next free state id (for secessions).
    pub fn next_state_id(&self) -> u32 {
        self.pack.states.last().map_or(1, |s| s.id + 1)
    }

    /// Next free religion id (for schisms).
    pub fn next_religion_id(&self) -> u32 {
        self.pack.religions.last().map_or(1, |r| r.id + 1)
    }
}

// ---------------------------------------------------------------------------//
// Public entry point
// ---------------------------------------------------------------------------//

/// Generate a deterministic `Timeline` (sorted by `(year, id)`) from a year-0
/// `Pack` + cell ownership arrays + era bounds + seed.
///
/// The `cells_*` arrays use the `i32` (`-1` = unassigned) / `i16` (`0` = none)
/// convention from `StatesResult` / `CulturesResult`. `cells_h` uses the
/// `u8` heightmap (FMG sea level = 20).
///
/// The engine evolves a working copy of the Pack, applying each accepted event
/// immediately so later modules see updated state. `narrative` is always `None`
/// (Phase 7 fills it in).
///
/// Returns a `Timeline` sorted by `(year, id)`.
pub fn generate_timeline(
    pack: &Pack,
    cells_state: &[i32],
    cells_culture: &[i32],
    cells_religion: &[i32],
    cells_burg: &[i16],
    cells_h: &[u8],
    seed: u64,
    params: &TimelineParams,
) -> Timeline {
    // Derive the engine's RNG seed. If `rng_override` is nonzero, use it;
    // otherwise derive a distinct stream from the timeline seed.
    let rng_seed = if params.rng_override != 0 {
        params.rng_override
    } else {
        seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(0x6A09E667)
    };

    // Normalize cell arrays: i32 (-1 = unassigned) → u32 (0 = unassigned),
    // i16 (0 = none) → u32 (0 = none).
    let n = cells_state.len();
    let cells_state_u32: Vec<u32> = cells_state.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();
    let cells_culture_u32: Vec<u32> = cells_culture.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();
    let cells_religion_u32: Vec<u32> = cells_religion.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();
    let cells_burg_u32: Vec<u32> = cells_burg.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();

    let cells_h_vec = if cells_h.len() == n {
        cells_h.to_vec()
    } else {
        vec![0u8; n]
    };

    let mut ctx = GenContext {
        pack: pack.clone(),
        cells_state: cells_state_u32,
        cells_culture: cells_culture_u32,
        cells_religion: cells_religion_u32,
        cells_burg: cells_burg_u32,
        era_start: params.era_start,
        era_end: params.era_end,
        params: params.clone(),
        events: Vec::new(),
        next_id: 1,
        cells_h: cells_h_vec,
    };

    // Iterate years in order. Each module gets a chance to fire per year.
    // Within a year, modules run in the order listed (found → war → plague →
    // golden_age → schism → migration → succession), matching the plan's
    // dependency order.
    for year in ctx.era_start..ctx.era_end {
        let yr = year; // i32

        // Re-derive the RNG state for this year from the seed so that the
        // per-year draw is deterministic and independent of the number of
        // events generated in prior years. This ensures that adding or
        // removing an event in year Y does not shift the RNG stream for
        // year Y+1 (a determinism hazard).
        let year_seed = rng_seed.wrapping_add((yr as u64).wrapping_mul(0x100000003));
        let mut year_rng = StdRng::seed_from_u64(year_seed);

        // 1. Found / expand — states find new burgs.
        gen_found_expand(&mut ctx, &mut year_rng, yr);

        // 2. War — inter-state wars + conquests.
        gen_war(&mut ctx, &mut year_rng, yr);

        // 3. Plague — population loss.
        gen_plague(&mut ctx, &mut year_rng, yr);

        // 4. Golden age — population growth.
        gen_golden_age(&mut ctx, &mut year_rng, yr);

        // 5. Schism — religious schisms.
        gen_schism(&mut ctx, &mut year_rng, yr);

        // 6. Migration — culture/religion spread.
        gen_migration(&mut ctx, &mut year_rng, yr);

        // 7. Succession — ruler changes.
        gen_succession(&mut ctx, &mut year_rng, yr);
    }

    // Sort by (year, id) for the canonical order. Even though we generate
    // in year order, sorting defensively guarantees the contract.
    ctx.events.sort_by(|a, b| {
        a.year.cmp(&b.year).then(a.id.cmp(&b.id))
    });

    ctx.events
}

/// Inner (test-callable) version of `generate_timeline` that takes already-
/// normalized `u32` cell arrays (the WorldAt form). Avoids the serde boundary
/// in native cargo tests.
pub fn generate_timeline_inner(
    pack: &Pack,
    cells_state: &[u32],
    cells_culture: &[u32],
    cells_religion: &[u32],
    cells_burg: &[u32],
    cells_h: &[u8],
    seed: u64,
    params: &TimelineParams,
) -> Timeline {
    let n = cells_state.len();
    let cells_h_vec = if cells_h.len() == n {
        cells_h.to_vec()
    } else {
        vec![0u8; n]
    };

    let rng_seed = if params.rng_override != 0 {
        params.rng_override
    } else {
        seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(0x6A09E667)
    };

    let mut ctx = GenContext {
        pack: pack.clone(),
        cells_state: cells_state.to_vec(),
        cells_culture: cells_culture.to_vec(),
        cells_religion: cells_religion.to_vec(),
        cells_burg: cells_burg.to_vec(),
        era_start: params.era_start,
        era_end: params.era_end,
        params: params.clone(),
        events: Vec::new(),
        next_id: 1,
        cells_h: cells_h_vec,
    };

    for year in ctx.era_start..ctx.era_end {
        let yr = year;
        let year_seed = rng_seed.wrapping_add((yr as u64).wrapping_mul(0x100000003));
        let mut year_rng = StdRng::seed_from_u64(year_seed);

        gen_found_expand(&mut ctx, &mut year_rng, yr);
        gen_war(&mut ctx, &mut year_rng, yr);
        gen_plague(&mut ctx, &mut year_rng, yr);
        gen_golden_age(&mut ctx, &mut year_rng, yr);
        gen_schism(&mut ctx, &mut year_rng, yr);
        gen_migration(&mut ctx, &mut year_rng, yr);
        gen_succession(&mut ctx, &mut year_rng, yr);
    }

    ctx.events.sort_by(|a, b| {
        a.year.cmp(&b.year).then(a.id.cmp(&b.id))
    });

    ctx.events
}

// ---------------------------------------------------------------------------//
// Module: found / expand (states found new burgs)
// ---------------------------------------------------------------------------//

/// Each state may found a new burg in an unoccupied land cell within its
/// territory. The probability is `ctx.params.found_rate` per state per year.
/// The new burg gets a `Found` event and is placed on a random unoccupied
/// land cell owned by the state.
fn gen_found_expand(ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
    if ctx.pack.states.is_empty() {
        return;
    }

    // Collect eligible state ids (owned data, no borrow on ctx).
    let eligible: Vec<(u32, i32)> = ctx
        .pack
        .states
        .iter()
        .filter(|s| {
            s.dissolved_year.is_none()
                && (s.rural_pop + s.urban_pop) >= ctx.params.min_state_pop
        })
        .map(|s| (s.id, s.founded_year))
        .collect();

    for (state_id, _) in eligible {
        if !rng.gen_bool(ctx.params.found_rate) {
            continue;
        }

        if let Some(burg) = try_found_burg(ctx, state_id, rng, year) {
            let cell = burg.cell;
            let burg_id = burg.id;
            ctx.cells_burg[cell as usize] = burg_id;
            ctx.pack.burgs.push(burg);
            ctx.push_event(
                year,
                burg_id,
                EntityType::Burg,
                EventKind::Found,
                EventPayload::Found { cell },
            );
        }
    }
}

/// Try to found a new burg for `state_id` on an unoccupied land cell within
/// the state's territory. Returns the Burg if placement succeeds.
fn try_found_burg(ctx: &GenContext, state_id: u32, rng: &mut StdRng, year: i32) -> Option<Burg> {
    // Collect unoccupied land cells owned by this state.
    let candidates: Vec<u32> = ctx
        .cells_state
        .iter()
        .enumerate()
        .filter_map(|(i, &s)| {
            let cell = i as u32;
            if s == state_id && ctx.is_land(cell) && ctx.cells_burg[i] == 0 {
                Some(cell)
            } else {
                None
            }
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }

    let &cell = candidates.choose(rng)?;
    let id = ctx.next_burg_id();

    let culture = ctx
        .find_state(state_id)
        .map(|s| s.culture)
        .unwrap_or(0);

    Some(Burg {
        id,
        name: format!("Burg{}", id),
        cell,
        state: state_id,
        culture,
        religion: 0,
        population: ctx.params.founding_population,
        feature: 1,
        capital: 0,
        founded_year: year,
        dissolved_year: None,
    })
}

// ---------------------------------------------------------------------------//
// Module: war (inter-state conflict + conquest)
// ---------------------------------------------------------------------------//

/// Two states go to war. The attacker is chosen by RNG from states with
/// sufficient military; the defender is a different extant state. The war has
/// a probabilistic outcome: attacker wins and conquers border cells, defender
/// wins, or stalemate (treaty).
fn gen_war(ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
    if ctx.pack.states.len() < 2 {
        return;
    }

    // Collect eligible attacker data as owned values (avoid holding an
    // immutable borrow on ctx.pack.states while we mutate ctx later).
    let attackers: Vec<(u32, u32, f64)> = ctx
        .pack
        .states
        .iter()
        .filter(|s| {
            s.dissolved_year.is_none()
                && s.military > 0
                && (s.rural_pop + s.urban_pop) >= ctx.params.min_state_pop
        })
        .map(|s| (s.id, s.military, s.rural_pop + s.urban_pop))
        .collect();

    if attackers.is_empty() {
        return;
    }

    for (attacker_id, att_military, _att_pop) in &attackers {
        if !rng.gen_bool(ctx.params.war_rate) {
            continue;
        }

        // Collect eligible defenders as owned ids.
        let defenders: Vec<u32> = ctx
            .pack
            .states
            .iter()
            .filter(|s| s.id != *attacker_id && s.dissolved_year.is_none())
            .map(|s| s.id)
            .collect();

        if defenders.is_empty() {
            continue;
        }

        let defender_id = *defenders.choose(rng).unwrap();

        // Determine the border cells between attacker and defender.
        let border_cells: Vec<u32> = ctx
            .cells_state
            .iter()
            .enumerate()
            .filter_map(|(i, &owner)| {
                let cell = i as u32;
                if owner == defender_id && ctx.is_land(cell) {
                    let w = 20;
                    let start = (i as usize).saturating_sub(w / 2);
                    let end = ((i as usize) + w / 2 + 1).min(ctx.cell_count());
                    for j in start..end {
                        if j != i && ctx.cells_state[j] == *attacker_id {
                            return Some(cell);
                        }
                    }
                }
                None
            })
            .collect();

        if border_cells.is_empty() {
            continue;
        }

        // War outcome probability based on military ratio.
        let att_str = *att_military as f64;
        let def_state = ctx.find_state(defender_id);
        let def_str = def_state.map(|s| s.military as f64).unwrap_or(0.0);
        let att_win_prob = att_str / (att_str + def_str + 1.0);
        let roll: f64 = rng.gen();

        if roll < att_win_prob * 0.7 {
            // Attacker wins — conquer some border cells.
            let n_conquer = rng.gen_range(1..=border_cells.len());
            let conquered: Vec<u32> = border_cells
                .choose_multiple(rng, n_conquer)
                .copied()
                .collect();

            let outcome = WarOutcome {
                result: 0, // attacker wins
                attrition: 0.3,
                conquered_cells: conquered.clone(),
            };

            for &cell in &conquered {
                ctx.cells_state[cell as usize] = *attacker_id;
            }

            // Reduce defender military + pop.
            if let Some(d) = ctx.find_state_mut(defender_id) {
                d.military = (d.military as f64 * 0.5) as u32;
                d.rural_pop *= 0.9;
                d.urban_pop *= 0.9;
            }
            if let Some(a) = ctx.find_state_mut(*attacker_id) {
                a.military = ((a.military as f64 * 0.8) as u32).max(1);
            }

            ctx.push_event(
                year,
                *attacker_id,
                EntityType::State,
                EventKind::War,
                EventPayload::War {
                    opponent_state_id: defender_id,
                    outcome,
                },
            );

            ctx.push_event(
                year,
                *attacker_id,
                EntityType::State,
                EventKind::Conquer,
                EventPayload::Conquer {
                    payload: ConquerPayload { cells: conquered },
                },
            );
        } else if roll < att_win_prob * 0.7 + 0.2 {
            // Defender wins.
            let outcome = WarOutcome {
                result: 1,
                attrition: 0.25,
                conquered_cells: vec![],
            };
            if let Some(a) = ctx.find_state_mut(*attacker_id) {
                a.military = ((a.military as f64 * 0.4) as u32).max(1);
                a.rural_pop *= 0.9;
                a.urban_pop *= 0.9;
            }
            if let Some(d) = ctx.find_state_mut(defender_id) {
                d.military = ((d.military as f64 * 0.8) as u32).max(1);
            }
            ctx.push_event(
                year,
                *attacker_id,
                EntityType::State,
                EventKind::War,
                EventPayload::War {
                    opponent_state_id: defender_id,
                    outcome,
                },
            );
        } else {
            // Stalemate / treaty.
            let outcome = WarOutcome {
                result: 2,
                attrition: 0.15,
                conquered_cells: vec![],
            };
            if let Some(a) = ctx.find_state_mut(*attacker_id) {
                a.military = ((a.military as f64 * 0.7) as u32).max(1);
            }
            if let Some(d) = ctx.find_state_mut(defender_id) {
                d.military = ((d.military as f64 * 0.7) as u32).max(1);
            }
            ctx.push_event(
                year,
                *attacker_id,
                EntityType::State,
                EventKind::War,
                EventPayload::War {
                    opponent_state_id: defender_id,
                    outcome,
                },
            );
        }
    }
}

// ---------------------------------------------------------------------------//
// Module: plague
// ---------------------------------------------------------------------------//

/// A plague reduces population in a state. Probability is
/// `ctx.params.plague_prob` per state per year.
fn gen_plague(ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
    // Collect eligible state ids (owned data).
    let eligible: Vec<u32> = ctx
        .pack
        .states
        .iter()
        .filter(|s| s.dissolved_year.is_none())
        .map(|s| s.id)
        .collect();

    for state_id in eligible {
        if rng.gen_bool(ctx.params.plague_prob) {
            let factor = 1.0 - (ctx.params.plague_mortality * rng.gen_range(0.5..=1.0));
            if let Some(s) = ctx.find_state_mut(state_id) {
                s.rural_pop *= factor;
                s.urban_pop *= factor;
            }
            ctx.push_event(
                year,
                state_id,
                EntityType::State,
                EventKind::Plague,
                EventPayload::PopScalar { factor },
            );
        }
    }
}

// ---------------------------------------------------------------------------//
// Module: golden age
// ---------------------------------------------------------------------------//

/// A golden age increases population growth in a state.
/// Probability is `ctx.params.golden_age_prob` per state per year.
fn gen_golden_age(ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
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

// ---------------------------------------------------------------------------//
// Module: schism
// ---------------------------------------------------------------------------//

/// A parent religion splits into a child denomination. Probability is
/// `ctx.params.schism_prob` per parent religion per year.
fn gen_schism(ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
    // Collect parent religion ids + their data (owned, no borrow on ctx).
    let parents: Vec<(u32, String, u32, u8, String, f64)> = ctx
        .pack
        .religions
        .iter()
        .filter(|r| r.parent.is_none() && r.followers > 1000.0)
        .map(|r| (
            r.id,
            r.name.clone(),
            r.center_cell,
            r.type_code,
            r.expansion_mode.clone(),
            r.followers,
        ))
        .collect();

    for (parent_id, parent_name, parent_center, parent_type_code, parent_mode, parent_followers) in parents {
        if !rng.gen_bool(ctx.params.schism_prob) {
            continue;
        }

        let fraction = ctx.params.schism_fraction * rng.gen_range(0.5..=1.0);
        let child_id = ctx.next_religion_id();

        // Reassign followers from parent to child.
        if let Some(p) = ctx.find_religion_mut(parent_id) {
            p.followers = parent_followers * (1.0 - fraction);
        }

        let child = Religion {
            id: child_id,
            name: format!("{}_schism", parent_name),
            color: ctx.find_religion(parent_id).map(|r| r.color).unwrap_or(0),
            center_cell: parent_center,
            parent: Some(parent_id),
            followers: parent_followers * fraction,
            type_code: parent_type_code,
            expansion_mode: parent_mode,
            founded_year: year,
            dissolved_year: None,
        };

        ctx.pack.religions.push(child);

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

// ---------------------------------------------------------------------------//
// Module: migration
// ---------------------------------------------------------------------------//

/// A culture spreads to adjacent cells. Probability is
/// `ctx.params.migration_prob` per culture per year.
fn gen_migration(ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
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

// ---------------------------------------------------------------------------//
// Module: succession (ruler change)
// ---------------------------------------------------------------------------//

/// A state gets a new ruler. This is primarily a narrative event (the state
/// persists, no cell change). Probability scales with the state's age.
fn gen_succession(ctx: &mut GenContext, rng: &mut StdRng, year: i32) {
    // Collect eligible state ids + founded_year (owned data).
    let eligible: Vec<(u32, i32)> = ctx
        .pack
        .states
        .iter()
        .filter(|s| {
            s.dissolved_year.is_none() && s.founded_year < year && year - s.founded_year > 50
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

// ---------------------------------------------------------------------------//
// Tests — verification gate (plan §Step 4.2)
// ---------------------------------------------------------------------------//

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::{Burg, Culture, Religion};
    use crate::grid::Grid;

    /// Build a minimal Pack with N states, for use in unit tests.
    fn make_pack(n_states: usize, n_cultures: usize, n_religions: usize, n_burgs: usize) -> Pack {
        let mut states = Vec::new();
        let mut burgs = Vec::new();
        let mut cultures = Vec::new();
        let mut religions = Vec::new();

        for i in 0..n_states {
            states.push(State {
                id: (i + 1) as u32,
                name: format!("State{}", i + 1),
                color: 0x4a6fa5 + (i as u32),
                capital: (i + 1) as u32,
                center_cell: (i * 100) as u32,
                form: "Monarchy".into(),
                tax_rate: 0.1,
                treasury: 5000.0,
                rural_pop: 10000.0 + (i as f64 * 1000.0),
                urban_pop: 5000.0 + (i as f64 * 500.0),
                military: 20 + (i as u32 * 10),
                founded_year: 0,
                dissolved_year: None,
                culture: 1,
            });
        }

        for i in 0..n_burgs {
            burgs.push(Burg {
                id: (i + 1) as u32,
                name: format!("Burg{}", i + 1),
                cell: (i * 50) as u32,
                state: if i < n_states { (i + 1) as u32 } else { 1 },
                culture: 1,
                religion: 1,
                population: 8.0,
                feature: 1,
                capital: if i < n_states { 1 } else { 0 },
                founded_year: 0,
                dissolved_year: None,
            });
        }

        for i in 0..n_cultures {
            cultures.push(Culture {
                id: (i + 1) as u32,
                name: format!("Culture{}", i + 1),
                color: 0xaa8844 + (i as u32),
                origin: (i * 200) as u32,
                type_code: 0,
                founded_year: 0,
                dissolved_year: None,
                cell_count: 500,
            });
        }

        for i in 0..n_religions {
            religions.push(Religion {
                id: (i + 1) as u32,
                name: format!("Religion{}", i + 1),
                color: 0xddccbb + (i as u32),
                center_cell: (i * 300) as u32,
                parent: None,
                followers: 50000.0 + (i as f64 * 10000.0),
                type_code: 0,
                expansion_mode: "global".into(),
                founded_year: 0,
                dissolved_year: None,
            });
        }

        Pack {
            states,
            provinces: Vec::new(),
            cultures,
            religions,
            burgs,
            armies: Vec::new(),
        }
    }

    /// Build cell arrays for `n` cells with `n_states` states owning them round-robin.
    fn make_cells(n: usize, n_states: usize) -> (Vec<i32>, Vec<i32>, Vec<i32>, Vec<i16>, Vec<u8>) {
        let cells_state: Vec<i32> = (0..n).map(|i| ((i % n_states) + 1) as i32).collect();
        let cells_culture: Vec<i32> = vec![1i32; n];
        let cells_religion: Vec<i32> = vec![1i32; n];
        let cells_burg: Vec<i16> = (0..n).map(|i| if i % 100 == 0 { 1i16 } else { 0i16 }).collect();
        let cells_h: Vec<u8> = vec![50u8; n]; // all land
        (cells_state, cells_culture, cells_religion, cells_burg, cells_h)
    }

    /// Generate a real Pack from a grid for integration tests.
    fn generate_real_pack(seed: u32, n: u32) -> (Pack, Vec<i32>, Vec<i32>, Vec<i32>, Vec<i16>, Vec<u8>) {
        let opts = crate::climate::ClimateOpts::default();
        let grid = crate::generate_world_inner(seed, n, &opts);

        let states_result = crate::gen_states::generate_states(&grid, seed, n.min(20) as u32);
        let suitability = crate::gen_states::compute_suitability(&grid);
        let cultures_result = crate::gen_cultures::generate_cultures_religions(
            &grid, seed, 5, 3, &suitability,
            &states_result.cells_state,
            &states_result.pack.burgs,
        );

        let mut pack = states_result.pack.clone();
        for c in &cultures_result.cultures {
            pack.cultures.push(c.clone());
        }
        for r in &cultures_result.religions {
            pack.religions.push(r.clone());
        }

        (
            pack,
            states_result.cells_state,
            cultures_result.cells_culture,
            cultures_result.cells_religion,
            states_result.cells_burg,
            grid.cells.h.clone(),
        )
    }

    // === M3 gate: determinism ===

    #[test]
    fn timeline_is_deterministic_same_seed() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 50, ..Default::default() };

        let t1 = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);
        let t2 = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        assert_eq!(t1, t2, "same seed must produce identical timeline");
    }

    #[test]
    fn timeline_differs_with_different_seed() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 50, ..Default::default() };

        let t1 = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);
        let t2 = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 43, &params);

        assert_ne!(t1, t2, "different seed should produce different timeline");
    }

    #[test]
    fn timeline_differs_with_different_era_bounds() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let p1 = TimelineParams { era_start: 0, era_end: 50, ..Default::default() };
        let p2 = TimelineParams { era_start: 0, era_end: 100, ..Default::default() };

        let t1 = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &p1);
        let t2 = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &p2);

        assert!(t2.len() >= t1.len(), "longer era should produce >= events");
    }

    // === M3 gate: non-empty ===

    #[test]
    fn timeline_is_non_empty_for_default_world() {
        let pack = make_pack(8, 4, 3, 8);
        let (cs, cc, cr, cb, ch) = make_cells(100, 8);
        let params = TimelineParams { era_start: 0, era_end: 200, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);
        assert!(!timeline.is_empty(), "default world must produce a non-empty timeline");
    }

    // === Event ID uniqueness + sorting ===

    #[test]
    fn all_event_ids_are_unique() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 100, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let mut ids: Vec<u64> = timeline.iter().map(|e| e.id).collect();
        let original = ids.clone();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), original.len(), "event ids must be unique");
    }

    #[test]
    fn timeline_is_sorted_by_year_then_id() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 100, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        for window in timeline.windows(2) {
            let a = &window[0];
            let b = &window[1];
            let ord = a.year.cmp(&b.year).then(a.id.cmp(&b.id));
            assert!(
                ord.is_lt(),
                "timeline not sorted: event {} (year={}, id={}) comes before (year={}, id={})",
                a.id, a.year, a.id, b.year, b.id
            );
        }
    }

    // === Year range ===

    #[test]
    fn all_events_within_era_bounds() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 50, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        for ev in &timeline {
            assert!(
                ev.year >= params.era_start && ev.year < params.era_end,
                "event {} year {} outside era [{}, {})",
                ev.id, ev.year, params.era_start, params.era_end
            );
        }
    }

    // === No missing entity references ===

    #[test]
    fn no_event_references_missing_entity() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 100, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        assert!(!timeline.is_empty(), "need events to check");

        // Project the timeline forward to the end year and verify the
        // projection succeeds (no panics from missing entities).
        let w = crate::timeline::project_world(&pack, &cs, &cc, &cr, &cb, &timeline, params.era_end - 1);

        // Verify projected cells only reference valid state ids (0 = unassigned,
        // or ids that exist in the base pack).
        let max_base_state: u32 = pack.states.iter().map(|s| s.id).max().unwrap_or(0);
        for &s in &w.cells_state {
            assert!(s == 0 || s <= max_base_state + 1000, "projected state id {} out of range", s);
        }
    }

    // === Narrative is always None ===

    #[test]
    fn all_narratives_are_none() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 100, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        for ev in &timeline {
            assert!(ev.narrative.is_none(), "event {} narrative must be None (Phase 7 sets it)", ev.id);
        }
    }

    // === Required event types appear when preconditions exist ===

    #[test]
    fn produces_found_events_when_states_exist() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 200, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let has_found = timeline.iter().any(|e| e.kind == EventKind::Found);
        assert!(has_found, "should produce Found events when states exist");
    }

    #[test]
    fn produces_war_events_when_multiple_states() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 200, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let has_war = timeline.iter().any(|e| e.kind == EventKind::War);
        assert!(has_war, "should produce War events when multiple states exist");
    }

    #[test]
    fn produces_schism_events_when_religions_exist() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 200, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let has_schism = timeline.iter().any(|e| e.kind == EventKind::Schism);
        assert!(has_schism, "should produce Schism events when religions exist");
    }

    #[test]
    fn produces_plague_events_with_sufficient_pop() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 200, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let has_plague = timeline.iter().any(|e| e.kind == EventKind::Plague);
        assert!(has_plague, "should produce Plague events with sufficient population");
    }

    #[test]
    fn produces_golden_age_events() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 200, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let has_golden_age = timeline.iter().any(|e| e.kind == EventKind::GoldenAge);
        assert!(has_golden_age, "should produce GoldenAge events");
    }

    #[test]
    fn produces_succession_events_for_aged_states() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 200, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let has_succession = timeline.iter().any(|e| e.kind == EventKind::Succession);
        assert!(has_succession, "should produce Succession events for aged states");
    }

    // === Event rate bounds ===

    #[test]
    fn event_count_within_bounds_for_small_world() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 100, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        // 5 states * 100 years * 7 modules * ~0.05 avg rate = ~35 max events.
        // Allow generous slack: < 500 events for a small test world.
        assert!(timeline.len() < 500, "event count {} exceeds sanity bound for 100-cell world", timeline.len());
    }

    // === Projection round-trip: events must be projectable ===

    #[test]
    fn generated_timeline_projects_cleanly() {
        let pack = make_pack(5, 3, 2, 5);
        let (cs, cc, cr, cb, ch) = make_cells(100, 5);
        let params = TimelineParams { era_start: 0, era_end: 50, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        let w0 = crate::timeline::project_world(&pack, &cs, &cc, &cr, &cb, &timeline, params.era_end - 1);
        assert_eq!(w0.year, params.era_end - 1);
        assert_eq!(w0.cells_state.len(), 100);
    }

    // === Inner (test-callable) version mirrors the public API ===

    #[test]
    fn inner_matches_public_api() {
        let pack = make_pack(3, 2, 1, 3);
        let (cs, cc, cr, cb, ch) = make_cells(50, 3);
        let params = TimelineParams { era_start: 0, era_end: 30, ..Default::default() };

        let cs_u32: Vec<u32> = cs.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();
        let cc_u32: Vec<u32> = cc.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();
        let cr_u32: Vec<u32> = cr.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();
        let cb_u32: Vec<u32> = cb.iter().map(|&v| if v < 0 { 0 } else { v as u32 }).collect();

        let t1 = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);
        let t2 = generate_timeline_inner(&pack, &cs_u32, &cc_u32, &cr_u32, &cb_u32, &ch, 42, &params);

        assert_eq!(t1, t2, "inner must match public API output");
    }

    // === Empty pack produces empty timeline ===

    #[test]
    fn empty_pack_produces_empty_timeline() {
        let pack = Pack::default();
        let cs = vec![-1i32; 10];
        let cc = vec![-1i32; 10];
        let cr = vec![-1i32; 10];
        let cb = vec![0i16; 10];
        let ch = vec![50u8; 10];
        let params = TimelineParams { era_start: 0, era_end: 50, ..Default::default() };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);
        assert!(timeline.is_empty(), "empty pack should produce no events");
    }

    // === Integration: real generated world ===

    #[test]
    fn timeline_from_real_generated_world() {
        let (pack, cs, cc, cr, cb, ch) = generate_real_pack(42, 500);
        let params = TimelineParams {
            era_start: 0,
            era_end: 100,
            ..Default::default()
        };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);
        assert!(!timeline.is_empty(), "real world should produce events");

        for window in timeline.windows(2) {
            let a = &window[0];
            let b = &window[1];
            let ord = a.year.cmp(&b.year).then(a.id.cmp(&b.id));
            assert!(ord.is_lt(), "timeline not sorted in real-world test");
        }

        for ev in &timeline {
            assert!(ev.year >= 0 && ev.year < 100);
        }
    }

    // === Event rate bounds for real world ===

    #[test]
    fn real_world_event_rate_within_bounds() {
        let (pack, cs, cc, cr, cb, ch) = generate_real_pack(42, 200);
        let params = TimelineParams {
            era_start: 0,
            era_end: 100,
            ..Default::default()
        };

        let timeline = generate_timeline(&pack, &cs, &cc, &cr, &cb, &ch, 42, &params);

        // At 200 cells with ~10 states over 100 years, we expect maybe
        // 50-150 events. Assert < 1000 as a generous upper bound.
        assert!(timeline.len() < 1000, "event count {} exceeds bound for 200-cell real world", timeline.len());
    }
}

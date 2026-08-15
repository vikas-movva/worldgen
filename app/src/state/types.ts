// Phase 3 Step 3.1 — Entity data model (TypeScript mirror).
//
// Mirrors `core/src/entities.rs` field-for-field (per
// `agent/worldgen-technical-requirements.md` §3.2), with `number` for the
// Rust `u32`/`i32`/`u8`/`u16`/`f64` types. This is the wire contract that
// Phase 4 will surface through the worker (`projectWorld` /
// `projectDelta` / `generateTimeline`); for now it is the typed shape a
// Phase 3.2/3.3 generator's unit tests will construct fixtures against.
//
// KEEP IN SYNC with `core/src/entities.rs`: every field added on the Rust
// side must land here with the same name (serde-wasm-bindgen maps snake_case
// Rust fields to snake_case JS keys — no rename). NOTE: `Pack` and these
// entity structs cross the Phase-4 `projectWorld`/`generateTimeline` worker
// boundary — NOT the Phase-2.5 `spliceDependentResult` per-cell helper
// (api.ts), which only touches the 11 `cells.*` index arrays + river/lake
// geometry and never sees `pack.*`. Do not add Pack fields to that helper.
//
// TODO(Route): design §3.2 lists `Route` among the seven base entities; it is
// intentionally NOT modeled here (MVP out-of-scope per §11 — Routes/markets/
// military-economy trade sim is stretch post-MVP). Add the Rust struct +
// TS mirror when that stretch goal is taken on.
//
// Type-width note (review F1): `Burg.id` is `u32` on the Rust side but the
// per-cell burg index `cells.burg` is `i16`. Both surface as `number` here so
// the mismatch is Rust-side only (invisible to the TS key-set tests). The
// Phase-4 timeline projector joins on `cells.burg[cell] == pack.burgs[id-1].id`;
// `gen_states.rs` must widen `CellData.burg` to `i32` or clamp, see
// `entities.rs` module doc.

/**
 * Base anthropological-layer entities at the year-0 anchor (design §3.2).
 * Mirrors `Pack` in `core/src/entities.rs`.
 */
export type Pack = {
	states: State[];
	provinces: Province[];
	cultures: Culture[];
	religions: Religion[];
	burgs: Burg[];
	armies: Army[];
};

/**
 * A sovereign polity (FMG `pack.states[i]`).
 * `dissolved_year == null` means still extant.
 * `color` is a packed `0xRRGGBB` RGB value written into the per-cell data
 * texture by the renderer.
 */
export type State = {
	id: number;
	name: string;
	color: number;
	/** Capital `Burg` id (0 = none assigned). */
	capital: number;
	/** The cell id the capital sits on (state expansion seed). */
	center_cell: number;
	/** Government form ("Monarchy"/"Republic"/"Theocracy"/...). */
	form: string;
	/** Per-state tax multiplier (FMG `DEFAULT_TAX_BY_FORM`). */
	tax_rate: number;
	/** Treasury at year-0. */
	treasury: number;
	/** Summed rural population across the state's cells. */
	rural_pop: number;
	/** Summed urban population across the state's burgs. */
	urban_pop: number;
	/** Aggregated military strength (drives Phase 4 `War` outcomes). */
	military: number;
	/** Year the state was founded (0 predates the era; negative allowed). */
	founded_year: number;
	/** `Some(Y)` if dissolved; `null` if extant. */
	dissolved_year: number | null;
	/** Originating `Culture` id. */
	culture: number;
};

/**
 * A subdivision of a `State` (FMG `pack.provinces`).
 * `dissolved_year == null` means still part of the owning state.
 */
export type Province = {
	id: number;
	/** Owning `State` id. */
	state: number;
	name: string;
	color: number;
	/** Seed cell the province grew from. */
	center_cell: number;
	rural_pop: number;
	urban_pop: number;
	founded_year: number;
	dissolved_year: number | null;
};

/**
 * A culture seeded from burg density and expanded by diffusion (FMG
 * `pack.cultures`). `type_code` mirrors the FMG culture category (0 nav,
 * 1 highland, 2 river, 3 lake, 4 nomadic).
 */
export type Culture = {
	id: number;
	name: string;
	color: number;
	/** The seed cell the culture originated on. */
	origin: number;
	/** FMG culture type code 0..=4. */
	type_code: number;
	founded_year: number;
	dissolved_year: number | null;
	/** Number of cells assigned to this culture at year-0. */
	cell_count: number;
};

/**
 * A religion (FMG `pack.religions`). `parent` is the schism-tree link: a
 * Phase 4 `Schism` event spawns a new `Religion` with
 * `parent = parent_id` and a seeded `follower_fraction` of the parent's
 * followers reassigned to the child. `followers` is the year-0 count.
 */
export type Religion = {
	id: number;
	name: string;
	color: number;
	/** Originating cell the religion was first preached from. */
	center_cell: number;
	/** `null` for an original "root" religion; `parent_id` for a schism child. */
	parent: number | null;
	/** Year-0 follower count (sum of the religion's burg populations). */
	followers: number;
	/** FMG religion type code. */
	type_code: number;
	founded_year: number;
	dissolved_year: number | null;
};

/**
 * A settlement (capital / town / city) on a single cell (FMG `pack.burgs`).
 * `population` is in thousands (FMG convention). The Step 2.5.4 entity
 * repair cascade emits placeholder `"Burg@cellN"` strings on land→water
 * flips; once Phase 3.2 populates a real `Pack`, it can read real `name`s
 * from here.
 */
export type Burg = {
	id: number;
	name: string;
	/** The cell id the burg sits on (a land cell). */
	cell: number;
	/** Owning `State` id (0 = unowned). */
	state: number;
	/** Owning `Culture` id (0 = unassigned). */
	culture: number;
	/** Owning `Religion` id (0 = unassigned). */
	religion: number;
	/** Population in thousands. Renderer scales the burg marker by this. */
	population: number;
	/** FMG feature flag (0 = off-map, nonzero = live burg). */
	feature: number;
	/** 1 if this burg is its state's capital, else 0. */
	capital: number;
	founded_year: number;
	dissolved_year: number | null;
};

/**
 * A military unit (FMG `pack.markers`, minimal MVP). Phase 3 does NOT
 * generate armies (they are raised by Phase 4 events); the year-0
 * `Pack.armies` is conventionally empty.
 */
export type Army = {
	id: number;
	/** Owning `State` id. */
	state: number;
	/** Current cell the army is deployed on (a land cell). */
	cell: number;
	/** Unit size (headcount). Phase 4 `Battle` subtracts casualties. */
	size: number;
	/** Composition tag ("infantry" / "cavalry" / "navy"). */
	kind: string;
	founded_year: number;
	dissolved_year: number | null;
};

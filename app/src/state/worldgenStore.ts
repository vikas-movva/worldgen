// Zustand store for worldgen state.
// Step 2.2: holds the current Grid for the renderer.
// Step 2.3: adds per-layer visibility (terrain/biome) so the UI can toggle
// without forcing a geometry rebuild.

import { create } from "zustand";
import type {
	Climate,
	CulturesResult,
	Grid,
	LakeGeo,
	Mesh,
	RiverGeo,
	StatesResult,
	Timeline,
	WorldAt,
} from "../core/api";
import type { EntityKind, LayerName } from "../render/layers";

export type LayerState = Record<LayerName, boolean>;

/** Maps each entity kind to its `layerEnabled` key (plural ids). */
/** Entity kinds that have fill layers (mutually exclusive via toggleEntityLayer).
 * "burg" is excluded — burgs are a point overlay toggled independently via
 * toggleLayer("burgs"), so they can appear on top of any fill layer. */
export const ENTITY_LAYER_KEYS: Record<Exclude<EntityKind, "burg">, LayerName> = {
	state: "states",
	province: "provinces",
	culture: "cultures",
	religion: "religions",
};

export type WorldgenState = {
	grid: Grid | null;
	mesh: Mesh | null;
	climate: Climate | null;
	generation: {
		seed: number;
		cellCount: number;
		startedAt: number;
		finishedAt: number;
	} | null;
	/** Per-layer visibility for the PixiJS renderer (Step 2.3). */
	layerEnabled: LayerState;
	/** Step 2.5.6: river polylines + lake polygons for the displayed Grid
	 * (from `getDrainageGeometry` on fresh gen, `recomputeDependents` on edit). */
	rivers: RiverGeo[];
	lakes: LakeGeo[];
	/** Step 3.2: the full states/provinces/burgs pack + per-cell arrays. */
	statesResult: StatesResult | null;
	/** Step 3.3: the culture/religion vectors + per-cell arrays. */
	culturesResult: CulturesResult | null;
	/** The currently selected entity (click-to-select or panel-select). */
	selectedEntity: { kind: EntityKind; id: number } | null;
	/** Step 2.5.4: heightmap editor tool mode. */
	editorTool: EditorTool;
	/** Step 2.5.4: brush radius (in relative units, 1.0 = moderate). */
	brushRadius: number;
	/** Step 2.5.4: brush strength (0.0 = no effect, 1.0 = max). */
	brushStrength: number;
	/** Step 2.5.4: currently selected cell (-1 = none). */
	selectedCellId: number;
	/** Phase 5: the generated timeline (events) for the current world. */
	timeline: Timeline | null;
	/** Phase 5: the era bounds [eraStart, eraEnd]. */
	eraStart: number;
	eraEnd: number;
	/** Phase 5: the current scrub year (0 = year-0 baseline). */
	currentYear: number;
	/** Phase 5: whether the timeline playback is active. */
	isPlaying: boolean;
	/** Phase 5: playback speed in years/sec. */
	playbackSpeed: number;
	/** Phase 5: current scrub request status (for UI feedback). */
	scrubStatus: "idle" | "loading" | "error";
	/** Phase 5: the projected WorldAt for the currentYear (or null). */
	projectedWorld: WorldAt | null;
};

export type EditorTool =
	| "raise"
	| "lower"
	| "flatten"
	| "smooth"
	| "range"
	| "trough"
	| "strait"
	| "mask"
	| "invert"
	| "add"
	| "multiply"
	| "select";

export type WorldgenActions = {
	setGrid: (grid: Grid) => void;
	setMesh: (mesh: Mesh) => void;
	setClimate: (climate: Climate) => void;
	setGenerationMeta: (meta: WorldgenState["generation"]) => void;
	/** Toggle a render layer on/off (terrain/biome). */
	toggleLayer: (layer: LayerName) => void;
	/**
	 * Toggle one of the four entity fill layers (state/province/culture/
	 * religion). Entity layers are mutually exclusive — enabling one
	 * disables the other three so only a single entity layer is ever
	 * displayed at a time (per the entity-UI spec). "burg" is excluded:
	 * burgs are a point overlay toggled via toggleLayer("burgs").
	 */
	toggleEntityLayer: (layer: Exclude<EntityKind, "burg">) => void;
	/** Select an entity (for highlight + the edit panel). */
	selectEntity: (sel: { kind: EntityKind; id: number } | null) => void;
	/** Update an entity's color + name in the relevant pack result in place. */
	updateEntity: (
		kind: EntityKind,
		id: number,
		patch: { color?: number; name?: string },
	) => void;
	/** Step 2.5.4: set the active editor tool. */
	setEditorTool: (tool: EditorTool) => void;
	/** Step 2.5.4: set brush radius. */
	setBrushRadius: (radius: number) => void;
	/** Step 2.5.4: set brush strength. */
	setBrushStrength: (strength: number) => void;
	/** Step 2.5.4: set the selected cell id (-1 = none). */
	setSelectedCellId: (id: number) => void;
	/** Step 2.5.6: set river + lake geometry for the displayed Grid. */
	setDrainageGeometry: (rivers: RiverGeo[], lakes: LakeGeo[]) => void;
	/** Step 3.2: store the states/provinces/burgs result. */
	setStatesResult: (r: StatesResult) => void;
	/** Step 3.3: store the culture/religion result. */
	setCulturesResult: (r: CulturesResult) => void;
	/** Phase 5.1: store the generated timeline + era bounds. */
	setTimeline: (timeline: Timeline | null, eraStart: number, eraEnd: number) => void;
	/** Phase 5.1: set the current scrub year (drives projection). */
	setCurrentYear: (year: number) => void;
	/** Phase 5.1: set playback state. */
	setIsPlaying: (playing: boolean) => void;
	/** Phase 5.1: set playback speed (years/sec). */
	setPlaybackSpeed: (speed: number) => void;
	/** Phase 5.1: set scrub request status. */
	setScrubStatus: (status: "idle" | "loading" | "error") => void;
	/** Phase 5.1: set the projected WorldAt from the worker. */
	setProjectedWorld: (world: WorldAt | null) => void;
	clear: () => void;
};

export const useWorldgenStore = create<WorldgenState & WorldgenActions>()(
	(set) => ({
		grid: null,
		mesh: null,
		climate: null,
		generation: null,
		layerEnabled: {
			terrain: true,
			biome: false,
			rivers: false,
			lakes: false,
			states: false,
			provinces: false,
			cultures: false,
			religions: false,
			burgs: false,
		},
		rivers: [],
		lakes: [],
		statesResult: null,
		culturesResult: null,
		selectedEntity: null,
		editorTool: "raise",
		brushRadius: 15,
		brushStrength: 0.05,
		selectedCellId: -1,
		timeline: null,
		eraStart: 0,
		eraEnd: 1000,
		currentYear: 0,
		isPlaying: false,
		playbackSpeed: 5,
		scrubStatus: "idle",
		projectedWorld: null,
		setGrid: (grid) => set({ grid }),
		setMesh: (mesh) => set({ mesh }),
		setClimate: (climate) => set({ climate }),
		setGenerationMeta: (generation) => set({ generation }),
		toggleLayer: (layer) =>
			set((s) => ({
				layerEnabled: { ...s.layerEnabled, [layer]: !s.layerEnabled[layer] },
			})),
		// Entity layers are mutually exclusive: enabling one turns the other
		// three off so only a single entity layer shows at once.
		toggleEntityLayer: (layer: Exclude<EntityKind, "burg">) =>
			set((s) => {
				const layerKey = ENTITY_LAYER_KEYS[layer];
				const turningOn = !s.layerEnabled[layerKey];
				const next: LayerState = { ...s.layerEnabled };
				for (const k of Object.values(ENTITY_LAYER_KEYS)) next[k] = false;
				if (turningOn) next[layerKey] = true;
				// Selecting an entity only makes sense while its layer is on;
				// clear the selection when the layer is turned off.
				return {
					layerEnabled: next,
					selectedEntity: turningOn ? s.selectedEntity : null,
				};
			}),
		selectEntity: (sel) => set({ selectedEntity: sel }),
		// Mutate the matching entity in the relevant pack result. We replace
		// the result object (new reference) so React/MapCanvas subscribers
		// re-run pushEntities and re-upload the entity color texture.
		updateEntity: (kind, id, patch) =>
			set((s) => {
				if (kind === "state") {
					if (!s.statesResult) return {};
					const pack = s.statesResult.pack;
					const idx = id - 1; // ids are 1-based
					if (idx < 0 || idx >= pack.states.length) return {};
					const updated: (typeof pack.states)[number] = {
						...pack.states[idx],
						...(patch.color !== undefined ? { color: patch.color } : {}),
						...(patch.name !== undefined ? { name: patch.name } : {}),
					};
					const newStates = pack.states.slice();
					newStates[idx] = updated;
					return {
						statesResult: {
							...s.statesResult,
							pack: { ...pack, states: newStates },
						},
					};
				}
				if (kind === "province") {
					if (!s.statesResult) return {};
					const pack = s.statesResult.pack;
					const idx = id - 1; // ids are 1-based
					if (idx < 0 || idx >= pack.provinces.length) return {};
					const updated: (typeof pack.provinces)[number] = {
						...pack.provinces[idx],
						...(patch.color !== undefined ? { color: patch.color } : {}),
						...(patch.name !== undefined ? { name: patch.name } : {}),
					};
					const newProvinces = pack.provinces.slice();
					newProvinces[idx] = updated;
					return {
						statesResult: {
							...s.statesResult,
							pack: { ...pack, provinces: newProvinces },
						},
					};
				}
				if (kind === "culture") {
					if (!s.culturesResult) return {};
					const idx = id; // ids are 0-based
					if (idx < 0 || idx >= s.culturesResult.cultures.length) return {};
					const updated: (typeof s.culturesResult.cultures)[number] = {
						...s.culturesResult.cultures[idx],
						...(patch.color !== undefined ? { color: patch.color } : {}),
						...(patch.name !== undefined ? { name: patch.name } : {}),
					};
					const newCultures = s.culturesResult.cultures.slice();
					newCultures[idx] = updated;
					return {
						culturesResult: { ...s.culturesResult, cultures: newCultures },
					};
				}
				if (kind === "religion") {
					if (!s.culturesResult) return {};
					const idx = id; // ids are 0-based
					if (idx < 0 || idx >= s.culturesResult.religions.length) return {};
					const updated: (typeof s.culturesResult.religions)[number] = {
						...s.culturesResult.religions[idx],
						...(patch.color !== undefined ? { color: patch.color } : {}),
						...(patch.name !== undefined ? { name: patch.name } : {}),
					};
					const newReligions = s.culturesResult.religions.slice();
					newReligions[idx] = updated;
					return {
						culturesResult: { ...s.culturesResult, religions: newReligions },
					};
				}
				return {};
			}),
		// Switching away from select clears the selection; switching to
		// select preserves any existing selection (the user may have a cell
		// picked from a previous interaction).
		setEditorTool: (editorTool) =>
			set((s) => ({
				editorTool,
				selectedCellId: editorTool === "select" ? s.selectedCellId : -1,
			})),
		setBrushRadius: (brushRadius) => set({ brushRadius }),
		setBrushStrength: (brushStrength) => set({ brushStrength }),
		setSelectedCellId: (selectedCellId) => set({ selectedCellId }),
		setDrainageGeometry: (rivers, lakes) => set({ rivers, lakes }),
		setStatesResult: (statesResult) => set({ statesResult }),
		setCulturesResult: (culturesResult) => set({ culturesResult }),
		// Phase 5.1: timeline state.
		setTimeline: (timeline, eraStart, eraEnd) =>
			set({ timeline, eraStart, eraEnd, currentYear: eraStart, projectedWorld: null }),
		setCurrentYear: (currentYear) => set({ currentYear }),
		setIsPlaying: (isPlaying) => set({ isPlaying }),
		setPlaybackSpeed: (playbackSpeed) => set({ playbackSpeed }),
		setScrubStatus: (scrubStatus) => set({ scrubStatus }),
		setProjectedWorld: (projectedWorld) => set({ projectedWorld }),
		clear: () =>
			set({
				grid: null,
				mesh: null,
				climate: null,
				generation: null,
				selectedCellId: -1,
				rivers: [],
				lakes: [],
				statesResult: null,
				culturesResult: null,
				timeline: null,
				eraStart: 0,
				eraEnd: 1000,
				currentYear: 0,
				isPlaying: false,
				playbackSpeed: 5,
				scrubStatus: "idle",
				projectedWorld: null,
			}),
	}),
);

// Selectors for the renderer — avoids re-renders when only grid changes
export const useGrid = () => useWorldgenStore((s) => s.grid);
export const useMesh = () => useWorldgenStore((s) => s.mesh);
export const useClimate = () => useWorldgenStore((s) => s.climate);

// Plain (non-hook) projections of the same slices. `useGrid`/`useMesh`/
// `useClimate` wrap these so the React hook and any non-React consumer
// (tests, workers) read the identical slice. Centralizing the projection
// here means a rename of the underlying state field can't silently break
// the hook without also breaking these tests.
export const selectGrid = (s: WorldgenState): WorldgenState["grid"] => s.grid;
export const selectMesh = (s: WorldgenState): WorldgenState["mesh"] => s.mesh;
export const selectClimate = (s: WorldgenState): WorldgenState["climate"] =>
	s.climate;

// Phase 5.1: timeline selectors.
export const selectTimeline = (s: WorldgenState): WorldgenState["timeline"] =>
	s.timeline;
export const selectCurrentYear = (
	s: WorldgenState,
): WorldgenState["currentYear"] => s.currentYear;
export const selectIsPlaying = (
	s: WorldgenState,
): WorldgenState["isPlaying"] => s.isPlaying;
export const selectProjectedWorld = (
	s: WorldgenState,
): WorldgenState["projectedWorld"] => s.projectedWorld;

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
} from "../core/api";
import type { EntityKind, LayerName } from "../render/layers";

export type LayerState = Record<LayerName, boolean>;

/** Maps each entity kind to its `layerEnabled` key (plural ids). */
export const ENTITY_LAYER_KEYS: Record<EntityKind, LayerName> = {
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
	 * Toggle one of the four entity layers. Entity layers are mutually
	 * exclusive — enabling one disables the other three so only a single
	 * entity layer is ever displayed at a time (per the entity-UI spec).
	 */
	toggleEntityLayer: (layer: EntityKind) => void;
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
		toggleEntityLayer: (layer) =>
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
					const updated: typeof pack.states[number] = {
						...pack.states[idx],
						...(patch.color !== undefined ? { color: patch.color } : {}),
						...(patch.name !== undefined ? { name: patch.name } : {}),
					};
					const newStates = pack.states.slice();
					newStates[idx] = updated;
					return {
						statesResult: { ...s.statesResult, pack: { ...pack, states: newStates } },
					};
				}
				if (kind === "province") {
					if (!s.statesResult) return {};
					const pack = s.statesResult.pack;
					const idx = id - 1; // ids are 1-based
					if (idx < 0 || idx >= pack.provinces.length) return {};
					const updated: typeof pack.provinces[number] = {
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
					const updated: typeof s.culturesResult.cultures[number] = {
						...s.culturesResult.cultures[idx],
						...(patch.color !== undefined ? { color: patch.color } : {}),
						...(patch.name !== undefined ? { name: patch.name } : {}),
					};
					const newCultures = s.culturesResult.cultures.slice();
					newCultures[idx] = updated;
					return { culturesResult: { ...s.culturesResult, cultures: newCultures } };
				}
				if (kind === "religion") {
					if (!s.culturesResult) return {};
					const idx = id; // ids are 0-based
					if (idx < 0 || idx >= s.culturesResult.religions.length) return {};
					const updated: typeof s.culturesResult.religions[number] = {
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

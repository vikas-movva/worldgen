// Zustand store for worldgen state.
// Step 2.2: holds the current Grid for the renderer.
// Step 2.3: adds per-layer visibility (terrain/biome) so the UI can toggle
// without forcing a geometry rebuild.

import { create } from "zustand";
import type { Climate, Grid, LakeGeo, Mesh, RiverGeo } from "../core/api";
import type { LayerName } from "../render/layers";

export type LayerState = Record<LayerName, boolean>;

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
	clear: () => void;
};

export const useWorldgenStore = create<WorldgenState & WorldgenActions>()(
	(set) => ({
		grid: null,
		mesh: null,
		climate: null,
		generation: null,
		layerEnabled: { terrain: true, biome: false, rivers: false, lakes: false },
		rivers: [],
		lakes: [],
		editorTool: "raise",
		brushRadius: 30,
		brushStrength: 0.5,
		selectedCellId: -1,
		setGrid: (grid) => set({ grid }),
		setMesh: (mesh) => set({ mesh }),
		setClimate: (climate) => set({ climate }),
		setGenerationMeta: (generation) => set({ generation }),
		toggleLayer: (layer) =>
			set((s) => ({
				layerEnabled: { ...s.layerEnabled, [layer]: !s.layerEnabled[layer] },
			})),
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
		clear: () =>
			set({
				grid: null,
				mesh: null,
				climate: null,
				generation: null,
				selectedCellId: -1,
				rivers: [],
				lakes: [],
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

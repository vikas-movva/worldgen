// Zustand store for worldgen state.
// Step 2.2: holds the current Grid for the renderer.
// Step 2.3: adds per-layer visibility (terrain/biome) so the UI can toggle
// without forcing a geometry rebuild.

import { create } from "zustand";
import type { Climate, Grid, Mesh } from "../core/api";
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
};

export type WorldgenActions = {
	setGrid: (grid: Grid) => void;
	setMesh: (mesh: Mesh) => void;
	setClimate: (climate: Climate) => void;
	setGenerationMeta: (meta: WorldgenState["generation"]) => void;
	/** Toggle a render layer on/off (terrain/biome). */
	toggleLayer: (layer: LayerName) => void;
	clear: () => void;
};

export const useWorldgenStore = create<WorldgenState & WorldgenActions>()(
	(set) => ({
		grid: null,
		mesh: null,
		climate: null,
		generation: null,
		layerEnabled: { terrain: true, biome: false },
		setGrid: (grid) => set({ grid }),
		setMesh: (mesh) => set({ mesh }),
		setClimate: (climate) => set({ climate }),
		setGenerationMeta: (generation) => set({ generation }),
		toggleLayer: (layer) =>
			set((s) => ({
				layerEnabled: { ...s.layerEnabled, [layer]: !s.layerEnabled[layer] },
			})),
		clear: () =>
			set({ grid: null, mesh: null, climate: null, generation: null }),
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
export const selectClimate = (s: WorldgenState): WorldgenState["climate"] => s.climate;

// Zustand store for worldgen state.
// Step 2.2: holds the current Grid for the renderer.

import { create } from "zustand";
import type { Climate, Grid, Mesh } from "../core/api";

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
};

export type WorldgenActions = {
	setGrid: (grid: Grid) => void;
	setMesh: (mesh: Mesh) => void;
	setClimate: (climate: Climate) => void;
	setGenerationMeta: (meta: WorldgenState["generation"]) => void;
	clear: () => void;
};

export const useWorldgenStore = create<WorldgenState & WorldgenActions>()(
	(set) => ({
		grid: null,
		mesh: null,
		climate: null,
		generation: null,
		setGrid: (grid) => set({ grid }),
		setMesh: (mesh) => set({ mesh }),
		setClimate: (climate) => set({ climate }),
		setGenerationMeta: (generation) => set({ generation }),
		clear: () =>
			set({ grid: null, mesh: null, climate: null, generation: null }),
	}),
);

// Selectors for the renderer — avoids re-renders when only grid changes
export const useGrid = () => useWorldgenStore((s) => s.grid);
export const useMesh = () => useWorldgenStore((s) => s.mesh);
export const useClimate = () => useWorldgenStore((s) => s.climate);

// Step 2.2 unit tests - the zustand store (`src/state/worldgenStore.ts`).
//
// The store holds the Grid for the PixiJS renderer and the per-layer visibility
// flags (terrain / biome). These tests pin the Step 2.2 contract:
//   - Initial state has null Grid / Mesh / Climate and terrain=on, biome=off.
//   - setGrid / setMesh / setClimate store their payload and replace prior value.
//   - toggleLayer flips exactly one flag, leaving the other untouched.
//   - clear resets Grid / Mesh / Climate / generation to null but preserves
//     layerEnabled (so a toggle survives a world regeneration).
//   - Selectors return the slice they name.
//
// No PixiJS or WebGL needed: the store is plain zustand and the test data is
// shaped to satisfy the TypeScript types without a real Grid.

import { beforeEach, describe, expect, it } from "vitest";
import type { Climate, Grid, Mesh } from "../core/api";
import { useWorldgenStore } from "./worldgenStore";

// Restore the store between tests so setGrid in one test does not leak into
// the next. zustand exposes `setState` + `getState` on the hook itself.
beforeEach(() => {
	useWorldgenStore.setState({
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
	});
});

// ---- small fixtures shaped to the Grid / Mesh / Climate types -----------

function fakeMesh(n: number, seed: number): Mesh {
	return {
		points: Array.from({ length: n }, () => [0, 0] as [number, number]),
		cells: {
			v: [],
			c: [],
			i: [],
			b: [],
			spacing: [],
			cells_x: 0,
			cells_y: 0,
		},
		vertices: { p: [] },
		world_w: 10_000,
		world_h: 8_000,
		seed: seed as unknown as undefined,
	} as unknown as Mesh;
}

function fakeGrid(n: number, seed: number): Grid {
	return {
		seed,
		mesh: fakeMesh(n, seed),
		cells: {
			h: new Array(n).fill(0),
			temp: new Array(n).fill(0),
			prec: new Array(n).fill(0),
			biome: new Array(n).fill(0),
			state: new Array(n).fill(0),
			province: new Array(n).fill(0),
			culture: new Array(n).fill(0),
			religion: new Array(n).fill(0),
			burg: new Array(n).fill(0),
			fl: new Array(n).fill(0),
			r: new Array(n).fill(0),
			conf: new Array(n).fill(0),
		},
	};
}

function fakeClimate(n: number): Climate {
	return {
		temp: new Int8Array(n),
		prec: new Uint8Array(n),
	};
}

// ---- initial state -----------------------------------------------------

describe("worldgenStore initial state", () => {
	it("starts with null Grid / Mesh / Climate / generation", () => {
		const s = useWorldgenStore.getState();
		expect(s.grid).toBeNull();
		expect(s.mesh).toBeNull();
		expect(s.climate).toBeNull();
		expect(s.generation).toBeNull();
	});

	it("defaults layerEnabled to terrain=on, biome=off", () => {
		const s = useWorldgenStore.getState();
		expect(s.layerEnabled).toEqual({
			terrain: true,
			biome: false,
			rivers: false,
			lakes: false,
			states: false,
			provinces: false,
			cultures: false,
			religions: false,
			burgs: false,
		});
	});
});

// ---- setters -----------------------------------------------------------

describe("worldgenStore setters", () => {
	it("setGrid stores the Grid and replaces a previous Grid", () => {
		const g1 = fakeGrid(100, 1);
		useWorldgenStore.getState().setGrid(g1);
		expect(useWorldgenStore.getState().grid).toBe(g1);

		const g2 = fakeGrid(200, 2);
		useWorldgenStore.getState().setGrid(g2);
		expect(useWorldgenStore.getState().grid).toBe(g2);
		expect(useWorldgenStore.getState().grid).not.toBe(g1);
	});

	it("setMesh stores the Mesh", () => {
		const m = fakeMesh(50, 7);
		useWorldgenStore.getState().setMesh(m);
		expect(useWorldgenStore.getState().mesh).toBe(m);
	});

	it("setClimate stores the Climate", () => {
		const c = fakeClimate(30);
		useWorldgenStore.getState().setClimate(c);
		expect(useWorldgenStore.getState().climate).toBe(c);
	});

	it("setGenerationMeta stores the generation metadata", () => {
		const meta = {
			seed: 42,
			cellCount: 1000,
			startedAt: 100,
			finishedAt: 200,
		};
		useWorldgenStore.getState().setGenerationMeta(meta);
		expect(useWorldgenStore.getState().generation).toEqual(meta);
	});
});

// ---- layer toggles (Step 2.3 contract) --------------------------------

describe("worldgenStore.toggleLayer", () => {
	it("flips terrain from true to false, leaving biome untouched", () => {
		useWorldgenStore.getState().toggleLayer("terrain");
		const s = useWorldgenStore.getState();
		expect(s.layerEnabled.terrain).toBe(false);
		expect(s.layerEnabled.biome).toBe(false); // still off
	});

	it("flips biome from false to true, leaving terrain untouched", () => {
		useWorldgenStore.getState().toggleLayer("biome");
		const s = useWorldgenStore.getState();
		expect(s.layerEnabled.biome).toBe(true);
		expect(s.layerEnabled.terrain).toBe(true); // still on
	});

	it("two toggles return to the original value (idempotent pair)", () => {
		useWorldgenStore.getState().toggleLayer("terrain");
		useWorldgenStore.getState().toggleLayer("terrain");
		expect(useWorldgenStore.getState().layerEnabled.terrain).toBe(true);
	});

	it("creates a new layerEnabled object on each toggle (immutable update)", () => {
		const before = useWorldgenStore.getState().layerEnabled;
		useWorldgenStore.getState().toggleLayer("biome");
		const after = useWorldgenStore.getState().layerEnabled;
		expect(after).not.toBe(before);
		expect(after).toEqual({
			terrain: true,
			biome: true,
			rivers: false,
			lakes: false,
			states: false,
			provinces: false,
			cultures: false,
			religions: false,
			burgs: false,
		});
	});
});

// ---- clear -------------------------------------------------------------

describe("worldgenStore.clear", () => {
	it("resets grid / mesh / climate / generation to null", () => {
		useWorldgenStore.getState().setGrid(fakeGrid(10, 1));
		useWorldgenStore.getState().setMesh(fakeMesh(10, 1));
		useWorldgenStore.getState().setClimate(fakeClimate(10));
		useWorldgenStore.getState().setGenerationMeta({
			seed: 1,
			cellCount: 10,
			startedAt: 0,
			finishedAt: 1,
		});

		useWorldgenStore.getState().clear();

		const s = useWorldgenStore.getState();
		expect(s.grid).toBeNull();
		expect(s.mesh).toBeNull();
		expect(s.climate).toBeNull();
		expect(s.generation).toBeNull();
	});

	it("resets layerEnabled to defaults across clear (stale toggles do not survive regeneration)", () => {
		// Turn extra layers on, then clear - a fresh WorldMap (built after a
		// later generate) must start from the pristine defaults rather than
		// whatever toggles the user had active, so stale/arbitrary visibility
		// doesn't leak from one world into the next.
		useWorldgenStore.getState().toggleLayer("biome");
		useWorldgenStore.getState().toggleLayer("states");
		useWorldgenStore.getState().clear();
		expect(useWorldgenStore.getState().layerEnabled).toEqual({
			terrain: true,
			biome: false,
			rivers: false,
			lakes: false,
			states: false,
			provinces: false,
			cultures: false,
			religions: false,
			burgs: false,
		});
	});

	it("resets selection + timeline + projected world across clear", () => {
		useWorldgenStore.getState().selectEntity({ kind: "state", id: 3 });
		useWorldgenStore.getState().setProjectedWorld({
			year: 500,
			cells_state: [],
			cells_province: [],
			cells_culture: [],
			cells_religion: [],
			cells_burg: [],
			pack: {
				states: [],
				provinces: [],
				cultures: [],
				religions: [],
				burgs: [],
			},
		});
		useWorldgenStore.getState().setTimeline([], 0, 500);
		useWorldgenStore.getState().clear();
		const s = useWorldgenStore.getState();
		expect(s.selectedEntity).toBeNull();
		expect(s.projectedWorld).toBeNull();
		expect(s.timeline).toBeNull();
		expect(s.currentYear).toBe(0);
	});
});

// ---- selectors ---------------------------------------------------------

// `useGrid`/`useMesh`/`useClimate` are React hooks (they call `useStore`,
// which needs a React render context and throws when called bare). Their
// projection logic is the body `selectGrid`/`selectMesh`/`selectClimate`,
// which read the same slice the hook wraps. We pin those selectors directly
// by feeding the store's `getState()` (the object the hook projects from),
// so a regression in the named selection (e.g. a typo'd slice key) fails
// here rather than only at runtime in a React component.
import { selectClimate, selectGrid, selectMesh } from "./worldgenStore";

describe("worldgenStore selectors", () => {
	it("selectGrid projects the grid slice (matches getState().grid)", () => {
		const g = fakeGrid(64, 9);
		useWorldgenStore.getState().setGrid(g);
		expect(selectGrid(useWorldgenStore.getState())).toBe(g);
	});

	it("selectMesh projects the mesh slice", () => {
		const m = fakeMesh(20, 3);
		useWorldgenStore.getState().setMesh(m);
		expect(selectMesh(useWorldgenStore.getState())).toBe(m);
	});

	it("selectClimate projects the climate slice", () => {
		const c = fakeClimate(15);
		useWorldgenStore.getState().setClimate(c);
		expect(selectClimate(useWorldgenStore.getState())).toBe(c);
	});

	it("selectors are pure projections (return null before any set)", () => {
		// Fresh getState() has null slices; selectors must reflect that.
		expect(selectGrid(useWorldgenStore.getState())).toBeNull();
		expect(selectMesh(useWorldgenStore.getState())).toBeNull();
		expect(selectClimate(useWorldgenStore.getState())).toBeNull();
	});
});

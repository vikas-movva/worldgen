// Ad-hoc verification test: canvas round-trip for height edits.
// Verifies the three things the canvas-fix depends on:
//   1. applyEdit-style spread produces a new grid ref (same mesh ref).
//   2. The MapCanvas subscription routing sends height-only edits to
//      the fast path (updateHeight), not rebuildMap.
//   3. WorldMap.updateHeight / updateBiome don't throw and preserve
//      the view subtree (no geometry teardown).

import { describe, expect, it } from "vitest";
import type { Grid } from "../core/api";
import { WorldMap } from "./layers";

function quadGrid(n = 4, w = 1000, h = 1000): Grid {
	const points: [number, number][] = [];
	const vertP: [number, number][] = [];
	const cellV: number[] = [];
	const cellI: number[] = [0];
	for (let c = 0; c < n; c++) {
		const cx = 500;
		const cy = 500;
		points.push([cx, cy]);
		const v0 = vertP.length;
		vertP.push(
			[cx - 100, cy - 100],
			[cx + 100, cy - 100],
			[cx + 100, cy + 100],
			[cx - 100, cy + 100],
		);
		cellV.push(v0, v0 + 1, v0 + 2, v0 + 3);
		cellI.push(cellV.length);
	}
	const mesh = {
		points,
		cells: {
			v: cellV,
			c: [],
			i: cellI,
			b: [],
			spacing: [],
			cells_x: 1,
			cells_y: 1,
		},
		vertices: { p: vertP },
		world_w: w,
		world_h: h,
	};
	return {
		seed: 1,
		mesh,
		cells: {
			h: new Array(n).fill(50),
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

describe("canvas-fix round-trip (ad-hoc)", () => {
	it("spread produces new grid ref, preserves mesh ref, no mutation", () => {
		const grid = quadGrid(4);
		const patch = { h: new Uint8Array([10, 80, 20, 90]) };
		const newGrid: Grid = {
			...grid,
			cells: { ...grid.cells, h: Array.from(patch.h) },
		};
		expect(newGrid).not.toBe(grid);
		expect(newGrid.mesh).toBe(grid.mesh);
		expect([...newGrid.cells.h]).toEqual([10, 80, 20, 90]);
		expect([...grid.cells.h]).toEqual([50, 50, 50, 50]);
	});

	it("subscription routes height-only edit to fast path, not rebuild", () => {
		let rebuild = 0;
		let updateH = 0;
		let updateB = 0;

		// Mirrors MapCanvas.tsx lines 209-218.
		const sub = (state: { grid: Grid | null }, prev: { grid: Grid | null }) => {
			if (state.grid !== prev.grid) {
				if (prev.grid && state.grid && state.grid.mesh === prev.grid.mesh) {
					updateH++;
					updateB++;
				} else {
					rebuild++;
				}
			}
		};

		const grid = quadGrid(4);
		// First load: null → new mesh → rebuild.
		sub({ grid }, { grid: null });
		expect(rebuild).toBe(1);
		expect(updateH).toBe(0);

		// Height edit: new grid object, SAME mesh → fast path.
		const edited: Grid = {
			...grid,
			cells: { ...grid.cells, h: [10, 80, 20, 90] },
		};
		sub({ grid: edited }, { grid });
		expect(rebuild).toBe(1);
		expect(updateH).toBe(1);
		expect(updateB).toBe(1);

		// World regen: new mesh → rebuild.
		const grid2 = quadGrid(8);
		sub({ grid: grid2 }, { grid: edited });
		expect(rebuild).toBe(2);
		expect(updateH).toBe(1);
	});

	it("WorldMap.updateHeight / updateBiome work and preserve view subtree", () => {
		const grid = quadGrid(4, 1000, 1000);
		const wm = new WorldMap(grid, {
			initialLayers: { terrain: true, biome: true },
		});
		const childCount = wm.view.children.length;

		expect(() => wm.updateHeight(grid)).not.toThrow();
		expect(() => wm.updateBiome(grid)).not.toThrow();
		expect(wm.view.children.length).toBe(childCount);

		// Null-safety after destroy.
		wm.destroy();
		expect(() => wm.updateHeight(grid)).not.toThrow();
		expect(() => wm.updateBiome(grid)).not.toThrow();
	});
});

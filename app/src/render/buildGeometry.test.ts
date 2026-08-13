// Step 2.3 unit tests - buildGeometry.ts (merged geometry builder).
//
// `buildWorldGeometry` fan-triangulates each Voronoi cell polygon from its
// seed point into a single vertex/index pair, with per-cell UV mapping into a
// 2-D pow2 data texture. These tests pin the contract:
//   - texDim is the smallest pow2 with texDim*texDim >= N.
//   - cellCount, worldW, worldH, texDim are returned correctly.
//   - Each cell's seed + ring vertices are present in the positions buffer in
//     normalized [0,1] space (y flipped, clamped).
//   - Each cell's UVs map to its texel center ((c % texDim + 0.5)/texDim,
//     (floor(c / texDim) + 0.5)/texDim).
//   - The fan index pattern is (seed, ring[r], ring[r+1]) for each triangle.
//   - Degenerate cells (< 3 polygon verts) are skipped.
//   - Buffers are tightly sliced to the actual written length (no slack).
//
// PixiJS note: in v8, positions live on `geometry.buffers[0].data` (a
// `Float32Array`), uvs on `buffers[1].data`, and indices on
// `geometry.indexBuffer.data` (a `Uint32Array`). `getBuffer("aPosition")`
// returns the same buffer object as `buffers[0]`. `getBuffer("indices")` is
// NOT a registered attribute name (indices live on `indexBuffer`); these
// tests read `buffers[2]` / `indexBuffer` for indices instead.

import { describe, expect, it } from "vitest";
import type { Grid, Mesh } from "../core/api";
import { buildWorldGeometry, type MeshGeometryData } from "./buildGeometry";

// ---- typed accessors into the PixiJS MeshGeometry buffers --------------

function positions(geo: MeshGeometryData): Float32Array {
	return (
		geo.geometry.getBuffer("aPosition") as unknown as { data: Float32Array }
	).data;
}

function indices(geo: MeshGeometryData): Uint32Array {
	return (geo.geometry as unknown as { indexBuffer: { data: Uint32Array } })
		.indexBuffer.data;
}

// ---- fixture helpers --------------------------------------------------

/** Build a synthetic Mesh + Grid with `n` cells, each a square quad (k=4). */
function squareGrid(n: number, worldW = 1000, worldH = 1000): Grid {
	// Lay the n seed points on a grid inside [0, worldW] x [0, worldH].
	const cols = Math.ceil(Math.sqrt(n));
	const rows = Math.ceil(n / cols);
	const dx = worldW / cols;
	const dy = worldH / rows;

	const points: [number, number][] = [];
	const cellV: number[] = []; // polygon vertex indices per cell (CSR via cellI)
	const cellI: number[] = [0];
	const vertP: [number, number][] = [];

	for (let c = 0; c < n; c++) {
		const col = c % cols;
		const row = Math.floor(c / cols);
		const cx = (col + 0.5) * dx;
		const cy = (row + 0.5) * dy;
		points.push([cx, cy]);

		// Square quad around the seed: 4 corners at +/- half cell.
		const v0 = vertP.length;
		vertP.push([cx - dx / 2, cy - dy / 2]); // top-left
		vertP.push([cx + dx / 2, cy - dy / 2]); // top-right
		vertP.push([cx + dx / 2, cy + dy / 2]); // bot-right
		vertP.push([cx - dx / 2, cy + dy / 2]); // bot-left
		cellV.push(v0, v0 + 1, v0 + 2, v0 + 3);
		cellI.push(cellV.length); // advance CSR end
	}

	const mesh: Mesh = {
		points,
		cells: {
			v: cellV,
			c: [],
			i: cellI,
			b: [],
			spacing: [],
			cells_x: cols,
			cells_y: rows,
		},
		vertices: { p: vertP },
		world_w: worldW,
		world_h: worldH,
	};
	return {
		seed: 1,
		mesh,
		cells: {
			h: new Array(n).fill(50),
			temp: new Array(n).fill(10),
			prec: new Array(n).fill(50),
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

/** Build a single triangular cell (k=3). */
function triangleGrid(worldW = 1000, worldH = 1000): Grid {
	const points: [number, number][] = [[500, 500]];
	const vertP: [number, number][] = [
		[100, 100],
		[900, 100],
		[500, 900],
	];
	const cellV = [0, 1, 2];
	const cellI = [0, 3];
	const mesh: Mesh = {
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
		world_w: worldW,
		world_h: worldH,
	};
	return {
		seed: 1,
		mesh,
		cells: { h: [50], temp: [10], prec: [50], biome: [0], state: [0], province: [0], culture: [0], religion: [0], burg: [0], fl: [0], r: [0], conf: [0] },
	};
}

// ---- texDim -----------------------------------------------------------

describe("buildWorldGeometry texDim", () => {
	it("returns the smallest pow2 with texDim*texDim >= N for small N", () => {
		// N=1 -> 16*16 = 256 (the function starts at 16 and only doubles)
		expect(buildWorldGeometry(squareGrid(1)).texDim).toBe(16);
		// N=4 -> still 16
		expect(buildWorldGeometry(squareGrid(4)).texDim).toBe(16);
	});

	it("doubles texDim when N exceeds the current capacity", () => {
		// 300 cells > 256 (16*16) -> texDim becomes 32 (32*32=1024)
		expect(buildWorldGeometry(squareGrid(300)).texDim).toBe(32);
		// 1100 cells > 1024 (32*32) -> texDim becomes 64
		expect(buildWorldGeometry(squareGrid(1100)).texDim).toBe(64);
	});

	it("fits the 60k MVP cap within 256 (256*256 = 65536)", () => {
		// 60000 <= 65536 -> 256
		expect(buildWorldGeometry(squareGrid(60_000)).texDim).toBe(256);
	});
});

// ---- metadata fields --------------------------------------------------

describe("buildWorldGeometry metadata", () => {
	it("returns cellCount = N, worldW/worldH from the mesh", () => {
		const geo = buildWorldGeometry(squareGrid(4, 1000, 800));
		expect(geo.cellCount).toBe(4);
		expect(geo.worldW).toBe(1000);
		expect(geo.worldH).toBe(800);
	});

	it("uv Float32Array length is 2 * vertex count", () => {
		const geo = buildWorldGeometry(squareGrid(9));
		expect(geo.uv.length % 2).toBe(0);
		expect(geo.uv).toBeInstanceOf(Float32Array);
	});
});

// ---- UV mapping -------------------------------------------------------

describe("buildWorldGeometry UV (texel center)", () => {
	it("maps cell 0 to texel center (0.5/texDim, 0.5/texDim)", () => {
		const grid = squareGrid(1);
		const geo = buildWorldGeometry(grid);
		const td = geo.texDim;
		// First vertex (the seed) carries cell 0's UV.
		expect(geo.uv[0]).toBeCloseTo((0 + 0.5) / td, 6);
		expect(geo.uv[1]).toBeCloseTo((Math.floor(0 / td) + 0.5) / td, 6);
	});

	it("maps cell c to ((c % td + 0.5)/td, (floor(c/td) + 0.5)/td)", () => {
		const N = 5;
		const geo = buildWorldGeometry(squareGrid(N));
		const td = geo.texDim;
		// Cell c's seed vertex is the first vertex of cell c. Each quad cell
		// has 5 vertices (seed + 4 ring). So cell c's seed is at offset c*5*2.
		for (let c = 0; c < N; c++) {
			const vIdx = c * 5; // seed vertex index for cell c
			const u = geo.uv[vIdx * 2];
			const v = geo.uv[vIdx * 2 + 1];
			expect(u).toBeCloseTo(((c % td) + 0.5) / td, 6);
			expect(v).toBeCloseTo((Math.floor(c / td) + 0.5) / td, 6);
		}
	});

	it("gives every vertex of a single cell the same UV", () => {
		const geo = buildWorldGeometry(squareGrid(1));
		const u0 = geo.uv[0];
		const v0 = geo.uv[1];
		for (let i = 0; i < 5; i++) {
			expect(geo.uv[i * 2]).toBeCloseTo(u0, 6);
			expect(geo.uv[i * 2 + 1]).toBeCloseTo(v0, 6);
		}
	});
});

// ---- positions (normalized + clamped) ---------------------------------

describe("buildWorldGeometry positions", () => {
	it("normalizes seed + ring positions to [0,1] (y flipped, clamped)", () => {
		const W = 1000;
		const H = 800;
		const geo = buildWorldGeometry(squareGrid(1, W, H));
		const pos = positions(geo);
		// cell 0 seed is at (dx/2, dy/2) = (500, 400) in world coords
		// normalized: x = 500/1000 = 0.5, y = 1 - 400/800 = 0.5
		expect(pos[0]).toBeCloseTo(0.5, 3);
		expect(pos[1]).toBeCloseTo(0.5, 3);
		// All positions are in [0, 1].
		for (let i = 0; i < pos.length; i += 2) {
			expect(pos[i]).toBeGreaterThanOrEqual(0);
			expect(pos[i]).toBeLessThanOrEqual(1);
			expect(pos[i + 1]).toBeGreaterThanOrEqual(0);
			expect(pos[i + 1]).toBeLessThanOrEqual(1);
		}
	});

	it("flips y so north (large world y) maps to small normalized y", () => {
		// Seed at world y = 900 (near top/north) should map to a SMALL
		// normalized y (near 0).
		const H = 1000;
		const grid = squareGrid(1, 1000, H);
		// Override the seed to sit high in world space.
		grid.mesh.points[0] = [500, 900];
		const geo = buildWorldGeometry(grid);
		const pos = positions(geo);
		// y normalized = 1 - 900/1000 = 0.1
		expect(pos[1]).toBeCloseTo(0.1, 3);
	});
});

// ---- fan triangulation indices ---------------------------------------

describe("buildWorldGeometry fan indices", () => {
	it("emits (seed, ring[r], ring[r+1]) for a triangular cell (k=3, 3 tris)", () => {
		const geo = buildWorldGeometry(triangleGrid());
		const idx = indices(geo);
		// k=3 -> 3 triangles -> 9 indices; vertices = seed + 3 ring = 4.
		expect(idx.length).toBe(9);
		// Triangle 0: (seed=0, ring[0]=1, ring[1]=2)
		expect(idx[0]).toBe(0);
		expect(idx[1]).toBe(1);
		expect(idx[2]).toBe(2);
		// Triangle 1: (seed=0, ring[1]=2, ring[2]=3)  (with wrap r+1 = (1+1)%3 = 2)
		expect(idx[3]).toBe(0);
		expect(idx[4]).toBe(2);
		expect(idx[5]).toBe(3);
		// Triangle 2: (seed=0, ring[2]=3, ring[0]=1)  (r=2, r+1 = (2+1)%3 = 0 -> vertex 1)
		expect(idx[6]).toBe(0);
		expect(idx[7]).toBe(3);
		expect(idx[8]).toBe(1);
	});

	it("emits k triangles (3k indices) for a quad cell (k=4)", () => {
		const geo = buildWorldGeometry(squareGrid(1));
		const idx = indices(geo);
		// k=4 -> 4 triangles -> 12 indices; vertices = seed + 4 ring = 5.
		expect(idx.length).toBe(12);
		// Last triangle wraps (r=3 -> r+1 = (3+1)%4 = 0 -> vertex seed+1+0)
		expect(idx[9]).toBe(0); // seed
		expect(idx[10]).toBe(4); // ring[3]
		expect(idx[11]).toBe(1); // ring[0] (wrapped)
	});

	it("total triangle count = sum of k over non-degenerate cells", () => {
		// 4 square quad cells, each k=4 -> 16 triangles -> 48 indices.
		const geo = buildWorldGeometry(squareGrid(4));
		const idx = indices(geo);
		expect(idx.length).toBe(4 * 4 * 3);
	});
});

// ---- degenerate cells -------------------------------------------------

describe("buildWorldGeometry degenerate cells", () => {
	it("skips a cell with fewer than 3 polygon vertices", () => {
		// Build a grid where cell 1 has only 2 ring vertices.
		const grid = squareGrid(2);
		// The CSR is already built; redefine cellV[start..end] to length 2.
		const cellV = grid.mesh.cells.v;
		const cellI = grid.mesh.cells.i;
		// cell 1's ring starts at cellI[1] = 4, currently ends at cellI[2] = 8.
		const base = cellI[1];
		cellV[base] = 0;
		cellV[base + 1] = 1;
		cellI[2] = base + 2; // cell 1 now has k=2
		const geo = buildWorldGeometry(grid);
		const idx = indices(geo);
		// Only cell 0 contributes (k=4 -> 12 indices). Cell 1 (k=2) skipped.
		expect(idx.length).toBe(12);
	});
});

// ---- buffer tightness -------------------------------------------------

describe("buildWorldGeometry buffer slicing", () => {
	it("positions/uv/indices are tightly sliced (no trailing slack)", () => {
		const geo = buildWorldGeometry(squareGrid(3));
		const pos = positions(geo);
		const idx = indices(geo);
		// 3 quad cells -> 5 verts each = 15 verts -> pos length = 30.
		expect(pos.length).toBe(15 * 2);
		// uv slice should match.
		expect(geo.uv.length).toBe(15 * 2);
		// 3 cells * 4 tris * 3 = 36 indices.
		expect(idx.length).toBe(36);
	});
});

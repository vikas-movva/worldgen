// Merged geometry builder (Step 2.3).
//
// Converts the Rust mesh CSR + the Grid cell layers into a SINGLE vertex/index
// buffer pair (one draw call for the whole map, per the design doc's "1–2 draw
// calls" mandate). Each Voronoi cell polygon is emitted as a fan of triangles
// from its seed `points[cell]`, and EVERY vertex of that cell carries the SAME
// `aUV` = the cell's texel center in a 1×N data texture:
//
//     aUV = ((cellId + 0.5) / N, 0.5)
//
// The default PixiJS mesh shader samples `uTexture` at `aUV`, so coloring a cell
// is just "put the cell's color in texel `cellId` of the data texture". Switching
// terrain <-> biome is therefore a texture swap (or a `mesh.visible` toggle), NOT
// a geometry rebuild. This is the data-texture color pattern the design mandates.
//
// World coordinates are in [0, world_w) × [0, world_h). We pre-transform them
// into a normalized [0,1]² space (y flipped so north = top). The camera (a scale
// + offset on `worldLayer`) maps that to the screen, so zoom/pan is a cheap
// container transform, not a geometry re-upload.

import { MeshGeometry } from "pixi.js";
import type { Grid } from "../core/api";

export type MeshGeometryData = {
  /** The PixiJS MeshGeometry: one buffer for the whole map (positions + per-cell UV). */
  geometry: MeshGeometry;
  /** Per-cell UV mapping (Float32Array, 2 per vertex) — cell texel center. */
  uv: Float32Array;
  /** Total number of cells (N). */
  cellCount: number;
  /** Original world dimensions, for aspect-correct fit if needed. */
  worldW: number;
  worldH: number;
};

/**
 * Build the merged geometry from a fully-populated `Grid`.
 *
 * Fan triangulation: for a cell with polygon vertices [v0..vk-1] and seed
 * point `p`, emit triangles (p, v0, v1), (p, v1, v2), ... (p, vk-1, v0).
 * This is correct for any simple (non-self-intersecting) convex-ish Voronoi
 * cell; FMG cells are convex, so the fan is exact.
 */
export function buildWorldGeometry(grid: Grid): MeshGeometryData {
	const { mesh } = grid;
	const { points, vertices } = mesh;
	const cellI = mesh.cells.i as unknown as number[];
	const cellV = mesh.cells.v as unknown as number[];
	const vpts = vertices.p as unknown as [number, number][]; // already [x,y]
	const worldW = mesh.world_w || 1;
	const worldH = mesh.world_h || 1;
	const N = points.length;

	// Worst-case sizing: a cell with k vertices fans into (k-2) triangles ->
	// (k-2)*3 vertices. Sum of k over all cells == cellV.length. So:
	//   max vertices == 3 * cellV.length
	//   max indices   == 3 * cellV.length
	const totalPolyVerts = cellV.length;
	const positions = new Float32Array(totalPolyVerts * 3 * 2);
	const uv = new Float32Array(totalPolyVerts * 3 * 2);
	const indices = new Uint32Array(totalPolyVerts * 3);

	let vWrite = 0; // vertex write cursor
	let iWrite = 0; // index write cursor

	const nx = (x: number) => x / worldW;
	const ny = (y: number) => 1 - y / worldH; // flip so north is up
	const uvx = (c: number) => (c + 0.5) / N; // cell texel center x

	for (let c = 0; c < N; c++) {
		const lo = cellI[c] as number;
		const hi = cellI[c + 1] as number;
		const k = hi - lo;
		if (k < 3) continue; // degenerate polygon -- skip

		// Seed point of the cell.
		const px = (points[c] as [number, number])[0];
		const py = (points[c] as [number, number])[1];
		const sx = nx(px);
		const sy = ny(py);
		const u = uvx(c);

		// Pre-fetch polygon ring in normalized space.
		const ringX = new Array<number>(k);
		const ringY = new Array<number>(k);
		for (let r = 0; r < k; r++) {
			const vid = cellV[lo + r] as number;
			const vpt = vpts[vid] as [number, number];
			ringX[r] = nx(vpt[0]);
			ringY[r] = ny(vpt[1]);
		}

		const baseSeed = vWrite;
		positions[vWrite * 2] = sx;
		positions[vWrite * 2 + 1] = sy;
		uv[vWrite * 2] = u;
		uv[vWrite * 2 + 1] = 0.5;
		vWrite++;

		for (let r = 0; r < k; r++) {
			positions[vWrite * 2] = ringX[r];
			positions[vWrite * 2 + 1] = ringY[r];
			uv[vWrite * 2] = u;
			uv[vWrite * 2 + 1] = 0.5;
			vWrite++;
		}

		// Fan: (seed, ring[r], ring[r+1])
		for (let r = 0; r < k; r++) {
			const r1 = (r + 1) % k;
			indices[iWrite++] = baseSeed; // seed
			indices[iWrite++] = baseSeed + 1 + r; // ring[r]
			indices[iWrite++] = baseSeed + 1 + r1; // ring[r+1]
		}
	}

	return {
	  geometry: new MeshGeometry({
	    positions: positions.subarray(0, vWrite * 2),
	    uvs: uv.subarray(0, vWrite * 2),
	    indices: indices.subarray(0, iWrite),
	    topology: "triangle-list",
	  }),
	  uv: uv.subarray(0, vWrite * 2),
	  cellCount: N,
	  worldW,
	  worldH,
	};
}

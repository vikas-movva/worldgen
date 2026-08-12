// Merged geometry builder (Step 2.3).
//
// Converts the Rust mesh CSR + the Grid cell layers into a SINGLE vertex/index
// buffer pair (one draw call for the whole map, per the design doc's "1–2 draw
// calls" mandate). Each Voronoi cell polygon is emitted as a fan of triangles
// from its seed `points[cell]`, and EVERY vertex of that cell carries the SAME
// `aUV` = the cell's texel center in a 2-D data texture:
//
//     texDim = smallest pow2 with texDim*texDim >= N
//     aUV = ( ((cellId % texDim) + 0.5) / texDim,
//             (floor(cellId / texDim) + 0.5) / texDim )
//
// The default PixiJS mesh shader samples `uTexture` at `aUV`, so coloring a cell
// is just "put the cell's color in texel `cellId` of the data texture". Switching
// terrain <-> biome is therefore a texture swap (or a `mesh.visible` toggle), NOT
// a geometry rebuild. This is the data-texture color pattern the design mandates.
//
// A 2-D texture is used instead of 1×N because the 1×N form exceeds
// MAX_TEXTURE_SIZE (8192 on many GPUs) at 60k cells. A 256×256 texture holds
// 65536 texels — enough for the 60k MVP cap — and every dimension stays well
// within GPU limits.
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
	/** Side length of the square data texture (texDim × texDim >= cellCount). */
	texDim: number;
	/** Original world dimensions, for aspect-correct fit if needed. */
	worldW: number;
	worldH: number;
};

/**
 * Compute the side length of the square data texture.
 *
 * Uses the next power of two so the texture is GPU-friendly (many drivers
 * require POT for non-mipmapped textures with certain wrap modes). 256×256 =
 * 65536 texels covers the 60k MVP cap with headroom.
 */
function texDimFor(cellCount: number): number {
	let dim = 16;
	while (dim * dim < cellCount) dim *= 2;
	return dim;
}

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
	const texDim = texDimFor(N);

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

	const nx = (x: number) => Math.min(1, Math.max(0, x / worldW));
	const ny = (y: number) => Math.min(1, Math.max(0, 1 - y / worldH)); // flip so north is up, clamp to [0,1]

	// 2-D texture UV for a cell: maps cellId to its texel center in the
	// texDim × texDim data texture.
	const uvOf = (c: number): [number, number] => {
		const tx = (c % texDim) + 0.5;
		const ty = Math.floor(c / texDim) + 0.5;
		return [tx / texDim, ty / texDim];
	};

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
		const [u0, u1] = uvOf(c);

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
		uv[vWrite * 2] = u0;
		uv[vWrite * 2 + 1] = u1;
		vWrite++;

		for (let r = 0; r < k; r++) {
			positions[vWrite * 2] = ringX[r];
			positions[vWrite * 2 + 1] = ringY[r];
			uv[vWrite * 2] = u0;
			uv[vWrite * 2 + 1] = u1;
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
		texDim,
		worldW,
		worldH,
	};
}

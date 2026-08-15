// World map layers (Step 2.3).
//
// Builds the terrain + biome render from a `Grid` using the merged geometry
// (buildWorldGeometry in buildGeometry.ts) and the data-texture color pattern
// (palette.ts).
//
// Design (see worldbuilding-tool-design.md, "Render layers"): every cell is one
// polygon in a SINGLE vertex/index buffer. Each vertex stores its cell id as a
// UV into a square data texture (texDim × texDim >= N). Colouring is then a
// pure texture swap:
//   - terrain  -> heightmap gradient texture (h -> RGBA)
//   - biome    -> biome palette texture (biome id -> RGBA)
// Two Mesh objects share ONE geometry (one draw call each) and differ only by
// their texture. Toggling a layer is `mesh.visible = ...` -- no re-tessellation.
//
// A small camera (drag-to-pan, wheel-to-zoom) transforms `view` so the map can
// be inspected at 60k cells without re-tessellation.

import { Container, Graphics, Mesh, Texture, TextureStyle } from "pixi.js";
import type { Grid } from "../core/api";
import { buildWorldGeometry, type MeshGeometryData } from "./buildGeometry";
import {
	BIOME_COLORS,
	buildBiomeTextureData,
	buildHeightTextureData,
	heightColor,
	rgb,
} from "./palette";

export type LayerName = "terrain" | "biome";
export type LayerState = Record<LayerName, boolean>;

const STYLE = new TextureStyle({
	scaleMode: "nearest",
	addressMode: "clamp-to-edge",
});

/** Build a texDim×texDim RGBA data texture (one texel per cell) from a Uint8Array. */
function dataTexture(data: Uint8Array, texDim: number): Texture {
	return Texture.from({
		resource: data,
		width: texDim,
		height: texDim,
		style: STYLE,
		format: "rgba8unorm",
	});
}

/**
 * Owns the merged geometry + terrain/biome meshes for one `Grid`.
 *
 * `view` is the `Container` you add to your world layer; pan/zoom is applied to
 * it. Call `setLayers` to toggle without rebuilding geometry. `destroy()` frees
 * GPU resources and detaches from the parent so the render loop won't touch a
 * dead subtree.
 */
export class WorldMap {
	readonly view: Container;
	private geometry: MeshGeometryData["geometry"] | null = null;
	private terrainMesh: Mesh | null = null;
	private biomeMesh: Mesh | null = null;
	private textures: Texture[] = [];
	private layers: LayerState = { terrain: true, biome: false };
	private worldW: number;
	private worldH: number;
	/** The height texture data buffer — updated in place by `updateHeight`. */
	private heightData: Uint8Array | null = null;
	/** The biome texture data buffer — updated in place by `updateBiome`. */
	private biomeData: Uint8Array | null = null;
	/**
	 * Step 2.5.5: the currently selected cell + the grid it was picked on,
	 * stashed so `fitToScreen` can re-stroke the outline after every camera
	 * change (pan/zoom/resize). Without re-stroking, the `Graphics` stroke
	 * width is fixed in view-local units and would balloon on zoom.
	 */
	private selectedGrid: Grid | null = null;
	private selectedCellId = -1;
	/** Last on-screen stroke width applied to the selection outline (px). */
	private selectionStrokeWidth = 0;

	constructor(grid: Grid, opts: { initialLayers?: Partial<LayerState> } = {}) {
		if (opts.initialLayers) Object.assign(this.layers, opts.initialLayers);
		const geoData = buildWorldGeometry(grid);
		this.geometry = geoData.geometry;
		this.worldW = geoData.worldW;
		this.worldH = geoData.worldH;
		const texDim = geoData.texDim;
		const heightData = buildHeightTextureData(grid.cells.h, texDim);
		this.heightData = heightData;
		const biomeData = buildBiomeTextureData(grid.cells.biome, texDim);
		this.biomeData = biomeData;
		const heightTex = dataTexture(heightData, texDim);
		const biomeTex = dataTexture(biomeData, texDim);
		this.textures = [heightTex, biomeTex];

		this.terrainMesh = new Mesh({
			geometry: this.geometry,
			texture: heightTex,
		});
		this.biomeMesh = new Mesh({ geometry: this.geometry, texture: biomeTex });

		this.view = new Container({ isRenderGroup: true });
		this.view.addChild(this.terrainMesh, this.biomeMesh);
		this.applyLayers();
	}

	/** User zoom multiplier (1 = fit-to-screen, 0.15..24). */
	private zoom = 1;
	/** Pan offset in screen pixels, applied on top of the centred fit position. */
	private panX = 0;
	private panY = 0;

	/**
	 * Base fit scale (without zoom multiplier), cached so zoom-toward-cursor
	 * can compute world-to-screen coordinate transforms.
	 */
	private baseScaleX = 1;
	private baseScaleY = 1;

	/**
	 * Scale and center the world map to fill the given screen dimensions.
	 *
	 * The merged geometry is in normalized [0,1]^2 space: each cell's position
	 * is (x / worldW, 1 - y / worldH). Since worldW and worldH differ, the
	 * pre-normalization step preserves the world's aspect by uniformly
	 * remapping both axes. Fit-contain the world's aspect ratio inside the
	 * screen, centering it, so the map is always fully visible regardless of
	 * viewport size. Set non-uniform view.scale(x, y) so the pre-normalized
	 * square [0,1]^2 renders with the world's true aspect ratio.
	 *
	 * The user's pan offset is preserved: the final position is the centred
	 * fit position plus (panX, panY). Only `resetView()` re-zeroes the pan.
	 */
	fitToScreen(screenW: number, screenH: number): void {
		if (screenW <= 0 || screenH <= 0) return;
		const worldAspect = this.worldW / this.worldH;
		const screenAspect = screenW / screenH;
		let xScale: number;
		let yScale: number;
		if (screenAspect > worldAspect) {
			// Screen is wider -> fit height; extra horizontal space = letterbox.
			yScale = screenH;
			xScale = screenH * worldAspect;
		} else {
			// Screen is narrower -> fit width; extra vertical space = pillarbox.
			xScale = screenW;
			yScale = screenW / worldAspect;
		}
		this.baseScaleX = xScale;
		this.baseScaleY = yScale;
		// Apply the current user zoom multiplier on top of the fit scale.
		// This separates "fit the map to the screen" from "let the user zoom
		// in/out with the wheel", so the zoom bounds operate on a sane [0.15, 24]
		// multiplier instead of the raw pixel scale (which is hundreds).
		const zx = xScale * this.zoom;
		const zy = yScale * this.zoom;
		this.view.scale.set(zx, zy);
		this.view.x = (screenW - zx) / 2 + this.panX;
		this.view.y = (screenH - zy) / 2 + this.panY;
		// Re-stroke the selection outline so its on-screen width stays
		// constant (the stroke lives in view-local units, which the scale
		// just changed). No-op when nothing is selected.
		this.drawSelection();
	}

	/**
	 * Set the user zoom multiplier (1 = fit-to-screen), zooming toward a
	 * focal point so the world coordinate under the cursor stays under the
	 * cursor. If `focus` is omitted the plain fit is re-applied (used by
	 * resize and programmatic zoom).
	 */
	setZoom(
		zoom: number,
		screenW: number,
		screenH: number,
		focus?: { x: number; y: number },
	): void {
		const clamped = Math.max(0.15, Math.min(24, zoom));
		if (focus) {
			// World coordinate under the cursor before zoom change.
			const oldScaleX = this.baseScaleX * this.zoom;
			const oldScaleY = this.baseScaleY * this.zoom;
			const oldOriginX = (screenW - oldScaleX) / 2 + this.panX;
			const oldOriginY = (screenH - oldScaleY) / 2 + this.panY;
			const worldX = (focus.x - oldOriginX) / oldScaleX;
			const worldW = (focus.y - oldOriginY) / oldScaleY;
			// Apply new zoom.
			this.zoom = clamped;
			const newScaleX = this.baseScaleX * this.zoom;
			const newScaleY = this.baseScaleY * this.zoom;
			// Adjust pan so the same world coordinate stays under the cursor.
			this.panX = focus.x - (screenW - newScaleX) / 2 - worldX * newScaleX;
			this.panY = focus.y - (screenH - newScaleY) / 2 - worldW * newScaleY;
		} else {
			this.zoom = clamped;
		}
		this.fitToScreen(screenW, screenH);
	}

	/** Add a screen-space delta to the pan offset (drag). Re-applies the fit. */
	panBy(dx: number, dy: number, screenW: number, screenH: number): void {
		this.panX += dx;
		this.panY += dy;
		this.fitToScreen(screenW, screenH);
	}

	/** Reset pan and zoom to defaults (fit-to-screen, centred). */
	resetView(screenW: number, screenH: number): void {
		this.zoom = 1;
		this.panX = 0;
		this.panY = 0;
		this.fitToScreen(screenW, screenH);
	}

	/** Get the current user zoom multiplier. */
	getZoom(): number {
		return this.zoom;
	}

	/**
	 * Step 2.5.4: inverse-transform a screen-space point to world-space
	 * coordinates. Used by the heightmap editor + cell picker to map a
	 * click on the canvas to a world (x, y) for `pickCell`.
	 *
	 * The forward transform (from `buildGeometry.ts` + `fitToScreen`):
	 *   nx = x / worldW          (normalize to [0,1])
	 *   ny = 1 - y / worldH       (flip y so north is up)
	 *   screen = view.origin + normalized * view.scale
	 *
	 * The inverse:
	 *   normalized = (screen - view.origin) / view.scale
	 *   x = nx * worldW
	 *   y = (1 - ny) * worldH
	 */
	screenToWorld(screenX: number, screenY: number): { x: number; y: number } {
		const originX = this.view.x;
		const originY = this.view.y;
		const scaleX = this.view.scale.x;
		const scaleY = this.view.scale.y;
		const nx = (screenX - originX) / scaleX;
		const ny = (screenY - originY) / scaleY;
		return {
			x: nx * this.worldW,
			y: (1 - ny) * this.worldH,
		};
	}

	/**
	 * Step 2.5.4: draw a selection outline around a cell. The outline is a
	 * `Graphics` polyline of the cell's polygon ring (from `mesh.cells.v` +
	 * `vertices.p`), added as a child of `view` so it inherits pan/zoom
	 * geometry. Pass `cellId = -1` to clear the selection.
	 *
	 * Step 2.5.5 fix: the stroke is drawn in view-local (normalized [0,1])
	 * units, but `view` is scaled by the fit-to-screen factor (hundreds of
	 * pixels). A `width: 2` stroke therefore rendered ~2*scaleX px wide on
	 * screen, filling the whole cell and reading as a big yellow polygon.
	 * The fix: stash the selection and re-stroke from `fitToScreen` with a
	 * scale-compensated width so the on-screen thickness stays ~2 px at any
	 * zoom. See `drawSelection`.
	 */
	private selectionGfx: Graphics | null = null;
	setSelected(grid: Grid, cellId: number): void {
		// Clear existing selection.
		if (this.selectionGfx) {
			this.selectionGfx.clear();
		}
		if (cellId < 0 || cellId >= grid.mesh.points.length) {
			this.selectedGrid = null;
			this.selectedCellId = -1;
			this.selectionStrokeWidth = 0;
			return;
		}
		this.selectedGrid = grid;
		this.selectedCellId = cellId;
		this.drawSelection();
	}

	/**
	 * Re-stroke the selection outline with the current camera scale. Computes
	 * a stroke width in view-local units that renders at a constant ~2px on
	 * screen: `localWidth = desiredPx / meanScale`. Called from `setSelected`
	 * (initial draw) and from `fitToScreen` (pan/zoom/resize re-stroke).
	 */
	private drawSelection(): void {
		if (this.selectionGfx) this.selectionGfx.clear();
		const grid = this.selectedGrid;
		const cellId = this.selectedCellId;
		if (!grid || cellId < 0 || cellId >= grid.mesh.points.length) {
			this.selectionStrokeWidth = 0;
			return;
		}
		if (!this.selectionGfx) {
			this.selectionGfx = new Graphics();
			this.selectionGfx.label = "selection";
			this.view.addChild(this.selectionGfx);
		}
		const cellI = grid.mesh.cells.i as unknown as number[];
		const cellV = grid.mesh.cells.v as unknown as number[];
		const vpts = grid.mesh.vertices.p as unknown as [number, number][];
		const worldW = grid.mesh.world_w || 1;
		const worldH = grid.mesh.world_h || 1;
		const nx = (x: number) => Math.min(1, Math.max(0, x / worldW));
		const ny = (y: number) => Math.min(1, Math.max(0, 1 - y / worldH));

		const lo = cellI[cellId] as number;
		const hi = cellI[cellId + 1] as number;
		const k = hi - lo;
		if (k < 3) {
			this.selectionStrokeWidth = 0;
			return;
		}

		const gfx = this.selectionGfx;
		gfx.clear();
		gfx.moveTo(
			nx(vpts[cellV[lo] as number][0]),
			ny(vpts[cellV[lo] as number][1]),
		);
		for (let r = 1; r < k; r++) {
			const vid = cellV[lo + r] as number;
			gfx.lineTo(nx(vpts[vid][0]), ny(vpts[vid][1]));
		}
		gfx.closePath();
		// Scale-compensate: stroke width is in view-local units; divide by
		// the mean of |scaleX|,|scaleY| so the on-screen width is ~2 px.
		// Clamp to a hairline minimum so a degenerate (near-zero) scale
		// never reads as a filled polygon.
		const sx = Math.abs(this.view.scale.x) || 1;
		const sy = Math.abs(this.view.scale.y) || 1;
		const meanScale = (sx + sy) / 2;
		const desiredPx = 2;
		const localWidth = Math.max(
			desiredPx / meanScale,
			1 / 4096, // hairline floor; below this strokes flicker out
		);
		this.selectionStrokeWidth = localWidth * meanScale; // store on-screen px
		gfx.stroke({
			color: 0xffff00,
			width: localWidth,
			alignment: 0.5,
		});
	}

	/**
	 * Step 2.5.5: the on-screen width (px) of the current selection
	 * outline — the value the scale-compensated local width renders at.
	 * Exposed for tests to assert the outline stays a thin hairline
	 * (not a giant polygon) across zoom levels. Returns 0 when no cell
	 * is selected.
	 */
	getSelectionStrokeWidth(): number {
		return this.selectionStrokeWidth;
	}

	private applyLayers(): void {
		if (this.terrainMesh) this.terrainMesh.visible = this.layers.terrain;
		if (this.biomeMesh) this.biomeMesh.visible = this.layers.biome;
	}

	/** Toggle layer visibility. No geometry rebuild -- pure visibility swap. */
	setLayers(layers: Partial<LayerState>): void {
		Object.assign(this.layers, layers);
		this.applyLayers();
	}

	/**
	 * Update the height texture from the grid's current `cells.h` without
	 * rebuilding geometry or meshes. Rebuilds the RGBA height color data in
	 * the existing buffer and triggers a GPU re-upload via `source.update()`.
	 * This is the live-painting fast path: ~O(N) CPU color fill + one texture
	 * upload, no tessellation.
	 */
	updateHeight(grid: Grid): void {
		if (!this.heightData || !this.textures[0]) return;
		const h = grid.cells.h;
		const n = h.length;
		// Rebuild the height color data in place — same buffer reference the
		// GPU texture was created from, so `source.update()` re-uploads it.
		const data = this.heightData;
		for (let i = 0; i < n; i++) {
			const [r, g, b] = heightColor(h[i]);
			data[i * 4 + 0] = r;
			data[i * 4 + 1] = g;
			data[i * 4 + 2] = b;
			// alpha stays 255 (set at construction, never changes)
		}
		this.textures[0].source.update();
	}

	/**
	 * Update the biome texture from the grid's current `cells.biome` without
	 * rebuilding geometry or meshes. Same in-place buffer + `source.update()`
	 * pattern as `updateHeight`.
	 */
	updateBiome(grid: Grid): void {
		if (!this.biomeData || !this.textures[1]) return;
		const biome = grid.cells.biome;
		if (!biome) return;
		const n = biome.length;
		const data = this.biomeData;
		for (let i = 0; i < n; i++) {
			const idx = Math.max(0, Math.min(biome[i] ?? 0, BIOME_COLORS.length - 1));
			const [r, g, b] = rgb(BIOME_COLORS[idx]);
			data[i * 4 + 0] = r;
			data[i * 4 + 1] = g;
			data[i * 4 + 2] = b;
			// alpha stays 255
		}
		this.textures[1].source.update();
	}

	/** Current layer visibility snapshot. */
	getLayers(): LayerState {
		return { ...this.layers };
	}

	destroy(): void {
		// Detach from parent FIRST so the render-group update list drops this
		// subtree before we null its renderable data -- otherwise the next
		// frame's _updateRenderGroups touches an undefined renderable and throws
		// "Cannot read properties of undefined (reading 'updateRenderable')".
		this.view.removeFromParent();
		this.view.destroy({ children: true });
		this.terrainMesh = null;
		this.biomeMesh = null;
		if (this.geometry) {
			this.geometry.destroy();
			this.geometry = null;
		}
		this.textures.forEach((t) => {
			// Destroy the texture but NOT the shared TextureStyle (passing
			// false). The module-level STYLE singleton is reused by every
			// WorldMap instance; destroying it corrupts subsequent maps.
			t.destroy(false);
		});
		this.textures = [];
		this.heightData = null;
		this.biomeData = null;
		// Drop the selection refs so a dangling re-stroke (e.g. a queued
		// `fitToScreen` after destroy) finds no cell and no-ops.
		this.selectionGfx = null;
		this.selectedGrid = null;
		this.selectedCellId = -1;
		this.selectionStrokeWidth = 0;
	}
}

/**
 * Attach pan/zoom camera controls to a Pixi `Container` (the world map view).
 *
 * The zoom operates as a MULTIPLIER on top of the base fit scale, not as an
 * absolute scale. This is because `fitToScreen` sets a non-uniform pixel scale
 * (e.g. x=896, y=717) that would blow past any reasonable zoom bound if treated
 * as the zoom level itself. Instead, the wheel adjusts a `zoom` factor (default
 * 1.0) clamped to [zoomMin, zoomMax], and the WorldMap applies
 * `fitScale * zoom` on both axes, preserving the non-uniform aspect ratio.
 *
 * Returns a detach function that removes the listeners.
 */
export function attachCamera(
	target: HTMLElement,
	opts: {
		worldMap: WorldMap;
		screenSize: () => { w: number; h: number };
		zoomMin?: number;
		zoomMax?: number;
	} = { worldMap: null as never, screenSize: () => ({ w: 0, h: 0 }) },
): () => void {
	const zoomMin = opts.zoomMin ?? 0.15;
	const zoomMax = opts.zoomMax ?? 24;
	const { worldMap, screenSize } = opts;
	let dragging = false;
	let lastX = 0;
	let lastY = 0;
	// Spacebar-to-pan: the camera only drag-pans while Space is held, so the
	// editor get clean pointer events for brush/select when Space is up. This
	// matches Figma/Photoshop's hand-tool modifier. We track Space with a
	// window keydown/keyup pair (not the canvas) so a held Space is recognized
	// even if focus is on a sibling control.
	let spaceDown = false;
	const onKeyDown = (e: KeyboardEvent) => {
		if (e.code === "Space" && !spaceDown) {
			spaceDown = true;
			// Prevent page scroll while Space is the pan modifier.
			e.preventDefault();
		}
	};
	const onKeyUp = (e: KeyboardEvent) => {
		if (e.code === "Space") {
			spaceDown = false;
		}
	};
	window.addEventListener("keydown", onKeyDown);
	window.addEventListener("keyup", onKeyUp);

	const onWheel = (e: WheelEvent) => {
		e.preventDefault();
		const factor = Math.exp(-e.deltaY * 0.0015);
		const current = worldMap.getZoom();
		const next = Math.max(zoomMin, Math.min(zoomMax, current * factor));
		const { w, h } = screenSize();
		// Zoom toward the cursor: the world coordinate under the mouse stays
		// under the mouse. `focus` is the cursor position relative to the
		// canvas element (the target of the wheel event).
		const rect = target.getBoundingClientRect();
		worldMap.setZoom(next, w, h, {
			x: e.clientX - rect.left,
			y: e.clientY - rect.top,
		});
	};
	const onDown = (e: PointerEvent) => {
		// Only start a pan drag while Space is held. Without Space, the editor
		// owns the pointer (brush / select / macro). Wheel-zoom stays always-on.
		if (!spaceDown) return;
		dragging = true;
		lastX = e.clientX;
		lastY = e.clientY;
		target.setPointerCapture?.(e.pointerId);
	};
	const onMove = (e: PointerEvent) => {
		if (!dragging) return;
		const { w, h } = screenSize();
		worldMap.panBy(e.clientX - lastX, e.clientY - lastY, w, h);
		lastX = e.clientX;
		lastY = e.clientY;
	};
	const onUp = (e: PointerEvent) => {
		if (!dragging) return;
		dragging = false;
		target.releasePointerCapture?.(e.pointerId);
	};

	target.addEventListener("wheel", onWheel, { passive: false });
	target.addEventListener("pointerdown", onDown);
	target.addEventListener("pointermove", onMove);
	target.addEventListener("pointerup", onUp);
	target.addEventListener("pointerleave", onUp);

	return () => {
		target.removeEventListener("wheel", onWheel);
		target.removeEventListener("pointerdown", onDown);
		target.removeEventListener("pointermove", onMove);
		target.removeEventListener("pointerup", onUp);
		target.removeEventListener("pointerleave", onUp);
		window.removeEventListener("keydown", onKeyDown);
		window.removeEventListener("keyup", onKeyUp);
	};
}

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
import { buildBiomeTextureData, buildHeightTextureData } from "./palette";

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

	constructor(grid: Grid, opts: { initialLayers?: Partial<LayerState> } = {}) {
		if (opts.initialLayers) Object.assign(this.layers, opts.initialLayers);
		const geoData = buildWorldGeometry(grid);
		this.geometry = geoData.geometry;
		this.worldW = geoData.worldW;
		this.worldH = geoData.worldH;
		const texDim = geoData.texDim;
		const heightTex = dataTexture(
			buildHeightTextureData(grid.cells.h, texDim),
			texDim,
		);
		const biomeTex = dataTexture(
			buildBiomeTextureData(grid.cells.biome, texDim),
			texDim,
		);
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
	 * `vertices.p`), added as a child of `view` so it inherits pan/zoom.
	 * Pass `cellId = -1` to clear the selection.
	 */
	private selectionGfx: Graphics | null = null;
	setSelected(grid: Grid, cellId: number): void {
		// Clear existing selection.
		if (this.selectionGfx) {
			this.selectionGfx.clear();
		}
		if (cellId < 0 || cellId >= grid.mesh.points.length) {
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
		if (k < 3) return;

		const gfx = this.selectionGfx;
		gfx.clear();
		gfx.moveTo(nx(vpts[cellV[lo] as number][0]), ny(vpts[cellV[lo] as number][1]));
		for (let r = 1; r < k; r++) {
			const vid = cellV[lo + r] as number;
			gfx.lineTo(nx(vpts[vid][0]), ny(vpts[vid][1]));
		}
		gfx.closePath();
		gfx.stroke({ color: 0xffff00, width: 2, alignment: 0.5 });
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
	};
}

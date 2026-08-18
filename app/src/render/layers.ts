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
import type { Grid, LakeGeo, RiverGeo, WorldAt } from "../core/api";
import { buildWorldGeometry, type MeshGeometryData } from "./buildGeometry";
import {
	BIOME_COLORS,
	buildBiomeTextureData,
	buildHeightTextureData,
	heightColor,
	rgb,
} from "./palette";

export type LayerName =
	| "terrain"
	| "biome"
	| "rivers"
	| "lakes"
	| "states"
	| "provinces"
	| "cultures"
	| "religions"
	| "burgs";
export type LayerState = Record<LayerName, boolean>;

/** The anthropological entity kinds (states/provinces/cultures/religions/burgs). */
export type EntityKind = "state" | "province" | "culture" | "religion" | "burg";

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
 * Normalize a world-space coordinate to the [0,1]^2 mesh space the renderer
 * draws in (x scaled by worldW, y flipped so north is up), clamped so a point
 * slightly outside the world still lands on a valid texel. Hoisted to module
 * scope — every overlay draw site (selection, rivers/lakes, state borders,
 * burgs) computes the same closure, so it's defined once instead of being
 * re-allocated as an arrow function per call/per loop iteration.
 */
function makeNormalizer(worldW: number, worldH: number) {
	const w = worldW || 1;
	const h = worldH || 1;
	return {
		nx: (x: number) => Math.min(1, Math.max(0, x / w)),
		ny: (y: number) => Math.min(1, Math.max(0, 1 - y / h)),
	};
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
	private layers: LayerState = {
		terrain: true,
		biome: false,
		rivers: false,
		lakes: false,
		states: false,
		provinces: false,
		cultures: false,
		religions: false,
		burgs: false,
	};
	private worldW: number;
	private worldH: number;
	/** The height texture data buffer — updated in place by `updateHeight`. */
	private heightData: Uint8Array | null = null;
	/** The biome texture data buffer — updated in place by `updateBiome`. */
	private biomeData: Uint8Array | null = null;
	/**
	 * Step 3.4: entity-layer meshes + color buffers. Each mesh shares the
	 * merged world `geometry` and carries its own 1×N RGBA data texture
	 * indexed by cell id (texel `cellId` = the per-cell entity color). The
	 * buffer is filled by `setEntities`/`updateEntities` from the
	 * `pack` + per-cell `state`/`province`/`culture`/`religion` arrays.
	 * Unassigned cells are transparent (alpha 0) so the terrain shows through.
	 */
	private stateMesh: Mesh | null = null;
	private provinceMesh: Mesh | null = null;
	private cultureMesh: Mesh | null = null;
	private religionMesh: Mesh | null = null;
	private stateData: Uint8Array | null = null;
	private provinceData: Uint8Array | null = null;
	private cultureData: Uint8Array | null = null;
	private religionData: Uint8Array | null = null;
	/**
	 * Step 3.4: the entity index arrays for the current grid, stashed so
	 * `drawSelection` can outline every cell belonging to the selected entity
	 * (a whole state / culture highlights, not just one cell). Cleared on
	 * `destroy()` so a dangling re-stroke finds no data and no-ops.
	 */
	private entityCells: {
		state: number[];
		province: number[];
		culture: number[];
		religion: number[];
	} | null = null;
	/**
	 * The ACTIVE source-of-truth for the entity color vectors —
	 * the last pack applied via `setEntities` (base year-0) or `updateEntities`
	 * (projected `WorldAt`). `patchEntityCells` re-fills the data textures from
	 * THESE vectors after an in-place per-cell/height edit, so the base pack
	 * never clobbers a projected view and vice-versa. Cleared on `destroy()`.
	 */
	private activePack: {
		states: { color: number }[];
		provinces: { color: number }[];
		cultures: { color: number }[];
		religions: { color: number }[];
	} | null = null;
	/** Step 3.4: the currently selected entity (for click-to-select highlight). */
	private selectedEntity: {
		kind: "state" | "province" | "culture" | "religion" | "burg";
		id: number;
	} | null = null;
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
	/** Step 2.5.6: rivers overlay Graphics (one polyline per river). */
	private riverGfx: Graphics | null = null;
	/** Step 2.5.6: lakes overlay Graphics (filled polygons). */
	private lakeGfx: Graphics | null = null;
	/** Last on-screen width (px) of river polylines; exposed for tests. */
	private riverStrokeWidth = 0;
	/**
	 * Step 3.4: burgs point overlay. Draws a marker per burg scaled by
	 * population, with capitals drawn distinctly (larger + crown color).
	 * Markers are scale-compensated so on-screen size stays ~constant
	 * across zoom (same pattern as rivers/selection). Re-drawn on every
	 * pan/zoom/resize via `fitToScreen`.
	 */
	private burgGfx: Graphics | null = null;
	/** Last on-screen radius (px) of burg markers; exposed for tests. */
	private burgRadius = 0;
	/** Stashed burg data + grid for re-drawing on camera change. */
	private burgGrid: Grid | null = null;
	private burgPoints: {
		id: number;
		cell: number;
		population: number;
		capital: number;
	}[] = [];
	/**
	 * Step 2.5.6: stashed river + lake geometry + the grid it was computed
	 * for, so `fitToScreen` can re-stroke the polylines at the new camera
	 * scale (stroke width is in view-local units; a resize/zoom changes the
	 * local->screen scale, so the on-screen thickness would balloon without
	 * re-stroking). Mirrors the selection-outline re-stroke pattern.
	 */
	private drainageGrid: Grid | null = null;
	private drainageRivers: RiverGeo[] = [];
	private drainageLakes: LakeGeo[] = [];
	/**
	 * state-border overlay Graphics. When the Provinces layer is
	 * visible, every state's outer boundary is stroked in white so the
	 * state map reads on top of the province fills (FMG draws state borders
	 * over provinces). Children of `view` so it inherits pan/zoom.
	 */
	private stateBorderGfx: Graphics | null = null;
	/** Last on-screen width (px) of the state-border strokes. */
	private stateBorderStrokeWidth = 5;

	/**
	 * Per-edge TRUE neighbor, derived from geometry (the other cell that
	 * shares BOTH endpoints of the segment), NOT from `cells.c`. The Rust
	 * mesh packs `cells.c` with a one-slot rotation against `cells.v`, so
	 * `cells.c[r]` is NOT the neighbor across segment `(v[r], v[r+1])` —
	 * using it makes `drawStateBorders` test the state of the wrong
	 * neighbor and outline non-border edges. Indexed by the global edge
	 * index `r` in `cells.v`/`cells.c`. `-1` = no neighbor (hull / coast
	 * edge). Built once per grid in `buildEdgeNeighbor`.
	 */
	private edgeNeighbor: Int32Array | null = null;

	constructor(grid: Grid, opts: { initialLayers?: Partial<LayerState> } = {}) {
		if (opts.initialLayers) Object.assign(this.layers, opts.initialLayers);
		const geoData = buildWorldGeometry(grid);
		this.geometry = geoData.geometry;
		this.worldW = geoData.worldW;
		this.worldH = geoData.worldH;
		this.stashGridForBorders = grid;
		// Build the per-edge TRUE-neighbor map from geometry. This is what
		// `drawStateBorders` uses to decide if a segment is a state border,
		// because `cells.c` is not edge-aligned to `cells.v` in the mesh.
		this.edgeNeighbor = this.buildEdgeNeighbor(grid);
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

		// Step 3.4: entity layers start transparent (no entities exist until
		// generateStates / generateCulturesReligions run). Each mesh shares the
		// world geometry and gets its own initially-transparent data texture;
		// setEntities fills + uploads it once the pack is available.
		const transparent = new Uint8Array(texDim * texDim * 4); // alpha 0
		this.stateData = transparent.slice();
		this.provinceData = transparent.slice();
		this.cultureData = transparent.slice();
		this.religionData = transparent.slice();
		const stateTex = dataTexture(this.stateData, texDim);
		const provinceTex = dataTexture(this.provinceData, texDim);
		const cultureTex = dataTexture(this.cultureData, texDim);
		const religionTex = dataTexture(this.religionData, texDim);
		this.textures.push(stateTex, provinceTex, cultureTex, religionTex);
		this.stateMesh = new Mesh({ geometry: this.geometry, texture: stateTex });
		this.provinceMesh = new Mesh({
			geometry: this.geometry,
			texture: provinceTex,
		});
		this.cultureMesh = new Mesh({
			geometry: this.geometry,
			texture: cultureTex,
		});
		this.religionMesh = new Mesh({
			geometry: this.geometry,
			texture: religionTex,
		});

		this.view = new Container({ isRenderGroup: true });
		this.burgGfx = new Graphics();
		this.burgGfx.label = "burgs";
		this.view.addChild(
			this.terrainMesh,
			this.biomeMesh,
			this.stateMesh,
			this.provinceMesh,
			this.cultureMesh,
			this.religionMesh,
			this.burgGfx,
		);
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
	 * apply ONLY the camera transform (`view.x/y/scale`) for the
	 * current fit-scale × zoom and pan offset. Cheap — no overlay re-tessellation.
	 * Used by `panBy` (drag) where neither the scale nor the stored overlays
	 * change, so we don't re-run the expensive `drawStateBorders`/`drawBurgs`/
	 * `drawRiversLakes` on every pointermove.
	 */
	private updateCameraView(screenW: number, screenH: number): void {
		if (screenW <= 0 || screenH <= 0) return;
		const zx = this.baseScaleX * this.zoom;
		const zy = this.baseScaleY * this.zoom;
		this.view.scale.set(zx, zy);
		this.view.x = (screenW - zx) / 2 + this.panX;
		this.view.y = (screenH - zy) / 2 + this.panY;
	}

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
	 *
	 * Because the fit (and therefore the local→screen scale) changes here, the
	 * fixed-width overlays are re-tessellated so their on-screen stroke width
	 * stays constant. Drag-pan via `panBy` does NOT call this, so those
	 * re-tessellations are avoided on every pointermove (see `updateCameraView`).
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
		this.updateCameraView(screenW, screenH);
		// Re-stroke the selection outline so its on-screen width stays
		// constant (the stroke lives in view-local units, which the scale
		// just changed). No-op when nothing is selected.
		this.drawSelection();
		// Re-stroke the river polylines so their on-screen width stays
		// constant (same scale-compensation as the selection outline).
		this.drawRiversLakes();
		// Re-stroke state borders (Provinces layer) at the new scale.
		this.drawStateBorders();
		// Re-draw burg markers at the new scale (scale-compensated).
		this.drawBurgs();
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

	/** Add a screen-space delta to the pan offset (drag). Only the
	 * camera transform is re-applied here — the overlays hold no pan-dependent
	 * geometry (their local→screen stroke width depends only on scale, which a
	 * pan does not change), so we skip the expensive `fitToScreen` re-tessellation
	 * that would otherwise run on every pointermove during a drag. */
	panBy(dx: number, dy: number, screenW: number, screenH: number): void {
		this.panX += dx;
		this.panY += dy;
		this.updateCameraView(screenW, screenH);
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
	 * return a stroke width in view-local (normalized [0,1]) units
	 * that renders at approximately `desiredPx` screen pixels under the current
	 * camera. Scale-compensation is `localWidth = desiredPx / meanScale`, floored
	 * so a tiny local width never collapses to 0. Previously copy-pasted at every
	 * overlay draw site — centralized here. Returns the local width; callers
	 * multiply by `currentMeanScale()` to recover the on-screen px for tests/
	 * inspectors.
	 */
	private scaleCompensatedWidth(desiredPx: number): number {
		return Math.max(desiredPx / this.currentMeanScale(), 1 / 4096);
	}

	/** The current average of the two camera scale axes (used to turn a
	 * view-local stroke width back into on-screen px). */
	private currentMeanScale(): number {
		const sx = Math.abs(this.view.scale.x) || 1;
		const sy = Math.abs(this.view.scale.y) || 1;
		return (sx + sy) / 2;
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
	 * Step 3.4: click-to-select an ENTITY. Given a picked `cellId`, find the
	 * owning entity of whichever entity layer is currently "on top" (priority
	 * religions > cultures > provinces > states), then highlight every cell
	 * belonging to that entity (not just the clicked cell). If no entity layer
	 * is visible, this falls back to single-cell selection.
	 *
	 * The highlighted-cells set is stashed in `selectedEntity` so `fitToScreen`
	 * can re-stroke the outline at the new camera scale (the same
	 * scale-compensation pattern as `drawSelection`).
	 */
	setSelectedEntity(grid: Grid, cellId: number): void {
		if (cellId < 0 || cellId >= grid.mesh.points.length) {
			this.selectedEntity = null;
			this.setSelected(grid, -1);
			return;
		}
		const cells = grid.cells;
		const burgCells = (cells.burg as number[] | undefined) ?? [];
		// Priority: most-specific visible layer wins. We detect the owning
		// entity by reading BOTH the grid's per-cell arrays AND the
		// entity-layer stash populated by setEntities — whichever has real
		// (non-sentinel) data wins. The main-thread grid.cells.* may lag the
		// worker's heldGrid (e.g. right after generation, before the
		// App splices them back), so falling back to entityCells keeps
		// click-to-select working regardless of that timing.
		const probe = (
			kind: "state" | "province" | "culture" | "religion" | "burg",
			id: number,
		) => id > (kind === "state" || kind === "province" ? -1 : 0);
		let kind: "state" | "province" | "culture" | "religion" | "burg" | null =
			null;
		let id = -1;
		const stash = this.entityCells;
		// Burg click-to-select: if the burgs layer is visible and the clicked
		// cell has a burg, select that burg (highest priority — most specific).
		if (this.layers.burgs) {
			const burgId = burgCells[cellId] ?? -1;
			if (burgId > 0) {
				kind = "burg";
				id = burgId;
			}
		}
		if (!kind && this.layers.religions) {
			const v = (stash?.religion ?? cells.religion)[cellId] ?? 0;
			if (probe("religion", v)) {
				kind = "religion";
				id = v;
			}
		}
		if (!kind && this.layers.cultures) {
			const v = (stash?.culture ?? cells.culture)[cellId] ?? 0;
			if (probe("culture", v)) {
				kind = "culture";
				id = v;
			}
		}
		if (!kind && this.layers.provinces) {
			const v = (stash?.province ?? cells.province)[cellId] ?? -1;
			if (probe("province", v)) {
				kind = "province";
				id = v;
			}
		}
		if (!kind && this.layers.states) {
			const v = (stash?.state ?? cells.state)[cellId] ?? -1;
			if (probe("state", v)) {
				kind = "state";
				id = v;
			}
		}
		if (!kind || id < 0) {
			// No entity layer active, or clicked cell is unassigned -> single cell.
			this.selectedEntity = null;
			this.setSelected(grid, cellId);
			this.drawStateBorders();
			return;
		}
		this.selectEntity(grid, kind, id);
	}

	/**
	 * Step 3.5: select an entity directly by (kind, id) — used by the entity
	 * panel list. Highlights every cell belonging to that entity and (if a
	 * state is selected while the Provinces layer is shown) its border.
	 */
	selectEntity(
		grid: Grid,
		kind: "state" | "province" | "culture" | "religion" | "burg",
		id: number,
	): void {
		// Guard against re-entrancy: the store mirrors this selection back
		// onto the map (store -> map subscription). If the requested
		// selection equals the current one, skip the re-stroke + the
		// onSelectEntity callback so we don't loop map <-> store forever.
		const cur = this.selectedEntity;
		if (cur && cur.kind === kind && cur.id === id) return;
		this.selectedEntity = { kind, id };
		this.selectedGrid = grid;
		this.selectedCellId = -1;
		// For a burg selection, find the cell the burg sits on so the
		// single-cell highlight (drawSelection fallback) can outline it.
		if (kind === "burg") {
			const burg = this.burgPoints.find((b) => b.id === id);
			if (burg) this.selectedCellId = burg.cell;
		}
		this.drawSelection();
		this.drawStateBorders();
		// Keep the store-facing selection in sync via the canvas hook.
		this.onSelectEntity?.(kind, id);
	}

	/** Optional callback the canvas wires to mirror selection into the store. */
	onSelectEntity:
		| ((
				kind: "state" | "province" | "culture" | "religion" | "burg",
				id: number,
		  ) => void)
		| null = null;

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
		const entity = this.selectedEntity;

		// Entity selection: outline every cell belonging to the entity.
		// This path must NOT depend on `cellId >= 0` because selectEntity
		// sets selectedCellId = -1 (no single cell picked).
		if (entity && this.entityCells && grid) {
			if (!this.selectionGfx) {
				this.selectionGfx = new Graphics();
				this.selectionGfx.label = "selection";
				this.view.addChild(this.selectionGfx);
			}
			const gfx = this.selectionGfx;
			gfx.clear();

			const localWidth = this.scaleCompensatedWidth(2);
			this.selectionStrokeWidth = localWidth * this.currentMeanScale();

			const arr =
				entity.kind === "state"
					? this.entityCells.state
					: entity.kind === "province"
						? this.entityCells.province
						: entity.kind === "culture"
							? this.entityCells.culture
							: this.entityCells.religion;
			const n = grid.mesh.points.length;
			let drawn = 0;
			for (let c = 0; c < n; c++) {
				if (arr[c] === entity.id) {
					drawn++;
				}
			}
			if (drawn > 0) {
				// Border-only outline of the entity silhouette (not the
				// full per-cell rings — those trace interior edges too).
				this.strokeEntityBoundary(gfx, grid, arr, entity.id, localWidth);
			} else if (cellId >= 0 && cellId < grid.mesh.points.length) {
				// Fallback: if the entity somehow has no cells, outline the picked one.
				this.strokeCellOutline(gfx, grid, cellId, localWidth);
			}
			return;
		}

		// Single-cell selection (no entity, or no entityCells data).
		if (!grid || cellId < 0 || cellId >= grid.mesh.points.length) {
			this.selectionStrokeWidth = 0;
			return;
		}
		if (!this.selectionGfx) {
			this.selectionGfx = new Graphics();
			this.selectionGfx.label = "selection";
			this.view.addChild(this.selectionGfx);
		}
		const gfx = this.selectionGfx;
		gfx.clear();

		const localWidth = this.scaleCompensatedWidth(2);
		this.selectionStrokeWidth = localWidth * this.currentMeanScale();

		this.strokeCellOutline(gfx, grid, cellId, localWidth);
	}

	/**
	 * Stroke only the OUTER boundary of an entity (the set of cells with
	 * `arr[c] === id`) — i.e. the segments along which a cell of the entity
	 * borders a cell of a DIFFERENT entity (or the map void). This is what a
	 * "select a state" highlight should show: the silhouette of the state,
	 * not every internal cell-to-cell edge.
	 *
	 * Uses the geometry-derived `edgeNeighbor` map (not `cells.c`, which is
	 * rotated) to find each segment's true neighbor. Segments whose neighbor
	 * is also in the entity are skipped; segments whose neighbor is outside
	 * the entity (or -1, the hull/coast) are stroked.
	 */
	private strokeEntityBoundary(
		gfx: Graphics,
		grid: Grid,
		arr: number[],
		id: number,
		localWidth: number,
	): void {
		const cellI = grid.mesh.cells.i as unknown as number[];
		const cellV = grid.mesh.cells.v as unknown as number[];
		const vpts = grid.mesh.vertices.p as unknown as [number, number][];
		const { nx, ny } = makeNormalizer(grid.mesh.world_w, grid.mesh.world_h);
		const edgeNb = this.edgeNeighbor;
		const n = grid.mesh.points.length;
		const inEntity = (c: number) => c >= 0 && c < n && arr[c] === id;

		for (let c = 0; c < n; c++) {
			if (arr[c] !== id) continue;
			const lo = cellI[c] as number;
			const hi = (cellI[c + 1] as number) ?? lo;
			const k = hi - lo;
			if (k < 3) continue;
			for (let r = lo; r < hi; r++) {
				const vid0 = cellV[r] as number;
				const vid1 =
					r + 1 < hi ? (cellV[r + 1] as number) : (cellV[lo] as number);
				const p0 = vpts[vid0];
				const p1 = vpts[vid1];
				if (!p0 || !p1) continue;
				const nb = edgeNb ? edgeNb[r] : -1;
				if (inEntity(nb)) continue; // internal edge — not a boundary
				gfx.moveTo(nx(p0[0]), ny(p0[1]));
				gfx.lineTo(nx(p1[0]), ny(p1[1]));
			}
		}
		gfx.stroke({ color: 0xffff00, width: localWidth, alignment: 0.5 });
	}

	/** Stroke the polygon ring of a single cell onto the selection Graphics. */
	private strokeCellOutline(
		gfx: Graphics,
		grid: Grid,
		cellId: number,
		localWidth: number,
	): void {
		const cellI = grid.mesh.cells.i as unknown as number[];
		const cellV = grid.mesh.cells.v as unknown as number[];
		const vpts = grid.mesh.vertices.p as unknown as [number, number][];
		const { nx, ny } = makeNormalizer(grid.mesh.world_w, grid.mesh.world_h);

		const lo = cellI[cellId] as number;
		const hi = cellI[cellId + 1] as number;
		const k = hi - lo;
		if (k < 3) return;

		gfx.moveTo(
			nx(vpts[cellV[lo] as number][0]),
			ny(vpts[cellV[lo] as number][1]),
		);
		for (let r = 1; r < k; r++) {
			const vid = cellV[lo + r] as number;
			gfx.lineTo(nx(vpts[vid][0]), ny(vpts[vid][1]));
		}
		gfx.closePath();
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

	/**
	 * Step 3.5: return the currently selected entity (for mirroring into the
	 * store / panel after a map click). Returns `{ kind, id }` or `null` when
	 * nothing is selected (or a single cell is picked, which has no entity).
	 */
	getSelectedEntity(): { kind: EntityKind; id: number } | null {
		return this.selectedEntity;
	}

	/**
	 * Step 2.5.6: the on-screen width (px) of the current river polylines
	 * (the value the scale-compensated local width renders at). Exposed for
	 * tests to assert the strokes stay a thin hairline (~2 px) across zoom,
	 * not a giant thick line. Returns 0 when no rivers are drawn.
	 */
	getRiverStrokeWidth(): number {
		return this.drainageRivers.length === 0 ? 0 : this.riverStrokeWidth;
	}

	/**
	 * Step 3.4: the on-screen radius (px) of the current burg markers — the
	 * value the scale-compensated local radius renders at. Exposed for tests
	 * to assert the markers stay a thin hairline (~4 px) across zoom. Returns
	 * 0 when no burgs are drawn.
	 */
	getBurgRadius(): number {
		return this.layers.burgs ? this.burgRadius : 0;
	}

	/**
	 * Step 3.4: set the burg markers to overlay on the map. Each burg is drawn
	 * as a circle centered on its cell's centroid, with radius scaled by
	 * population (FMG scales marker size by `population`). Capitals are drawn
	 * with a distinct gold crown color and a larger base radius.
	 *
	 * The geometry is stashed so `fitToScreen` can re-draw at the new camera
	 * scale (marker radius is in view-local units, same scale-compensation
	 * pattern as rivers/selection/state-borders).
	 *
	 * @param grid  the Grid (for `mesh.points` cell centroids).
	 * @param burgs the burg list from `pack.burgs`, including `id`.
	 */
	setBurgs(
		grid: Grid | null,
		burgs: { id: number; cell: number; population: number; capital: number }[],
	): void {
		this.burgGrid = grid;
		this.burgPoints = burgs;
		this.drawBurgs();
	}

	/**
	 * Step 3.5: the on-screen width (px) of the current state-border
	 * strokes — the value the scale-compensated local width renders at.
	 * Exposed for tests to assert the borders stay a thin hairline (~1.5 px)
	 * across zoom. Returns 0 when the Provinces layer is off / no borders.
	 */
	getStateBorderStrokeWidth(): number {
		return this.layers.provinces ? this.stateBorderStrokeWidth : 0;
	}

	/**
	 * Step 2.5.6: set the river + lake geometry to overlay on the map. Both
	 * are drawn as children of `view` in normalized [0,1]^2 space (y flipped
	 * so north is up), so they inherit pan/zoom. River polylines are stroked
	 * with a scale-compensated width (see `drawRiversLakes`) so the on-screen
	 * thickness stays ~constant across zoom; lakes are filled polygons.
	 *
	 * Pass empty arrays to clear the overlays. The geometry is stashed so
	 * `fitToScreen` can re-stroke at a new camera scale (the local-unit
	 * stroke width must be recomputed on every resize/zoom, identical to the
	 * selection-outline pattern).
	 */
	setRiversLakes(
		grid: Grid | null,
		rivers: RiverGeo[],
		lakes: LakeGeo[],
	): void {
		this.drainageGrid = grid;
		this.drainageRivers = rivers;
		this.drainageLakes = lakes;
		this.drawRiversLakes(true);
	}

	/**
	 * Re-draw the river polylines + lake polygons from the stashed geometry
	 * at the current camera scale. Stroke widths are scale-compensated
	 * (`localWidth = desiredPx / meanScale`) so an on-screen target of ~2 px
	 * survives zoom/resize. Called from `setRiversLakes` (initial draw, with
	 * `force` so the buffer is primed regardless of the current toggle) and
	 * from `fitToScreen`/`applyLayers` (camera re-stroke). When not forced and
	 * neither the Rivers nor the Lakes layer is visible, the cached buffer is
	 * reused and this is a no-op — avoiding a full O(edges) re-tessellation on
	 * every camera change while the overlays are hidden.
	 */
	private drawRiversLakes(force = false): void {
		if (!force && !this.layers.rivers && !this.layers.lakes) return;
		const grid = this.drainageGrid;
		if (!grid) {
			if (this.riverGfx) this.riverGfx.clear();
			if (this.lakeGfx) this.lakeGfx.clear();
			this.riverStrokeWidth = 0;
			return;
		}
		const rivers = this.drainageRivers;
		const lakes = this.drainageLakes;
		const points = grid.mesh.points as unknown as [number, number][];
		const { nx, ny } = makeNormalizer(grid.mesh.world_w, grid.mesh.world_h);

		// Lakes: render each lake cell as a filled polygon using its mesh
		// quad vertices (`mesh.cells.v` groups + `cells.i` offsets). FMG
		// draws lakes as filled water polygons; `LakeGeo.cells` are CELL
		// ids (not vertex ids), so we look up each cell's polygon ring and
		// fill it. Visually this paints the lake-cell quads blue — the
		// tightest correct silhouette. To hide the hairline antialiasing
		// seams where adjacent lake quads meet, we ALSO stroke a thin
		// scale-compensated water-coloured outline over every lake ring so
		// the lake reads as one solid body with a crisp shoreline (Issue:
		// "lakes layer rendering").
		if (!this.lakeGfx) {
			this.lakeGfx = new Graphics();
			this.lakeGfx.label = "lakes";
			this.view.addChild(this.lakeGfx);
		}
		const lakeGfx = this.lakeGfx;
		lakeGfx.clear();
		const LAKE_FILL = 0x3b6ea5; // lake blue, reads clearly on terrain
		const SHORE_LINE = 0x6f9fd0; // lighter shoreline for definition
		const cellV = grid.mesh.cells.v as unknown as number[];
		const cellI = grid.mesh.cells.i as unknown as number[];
		const vpts =
			(grid.mesh.vertices?.p as unknown as [number, number][]) ?? points;
		// Pre-collect each lake ring as normalized points so we can draw both
		// the fill pass and the shoreline-stroke pass without re-walking geometry.
		const rings: Array<Array<{ x: number; y: number }>> = [];
		for (const lake of lakes) {
			for (const cellId of lake.cells) {
				const lo = cellI[cellId] as number;
				const hi = (cellI[cellId + 1] as number) ?? lo;
				if (hi <= lo) continue;
				const ring: Array<{ x: number; y: number }> = [];
				let ok = true;
				for (let r = lo; r < hi; r++) {
					const vid = cellV[r] as number;
					const p = vpts[vid] ?? (points as [number, number][])[vid];
					if (!p) {
						ok = false;
						break;
					}
					ring.push({ x: nx(p[0]), y: ny(p[1]) });
				}
				if (ok) rings.push(ring);
			}
		}
		// Fill pass.
		for (const ring of rings) {
			lakeGfx.poly(ring).fill({ color: LAKE_FILL, alpha: 0.92 });
		}
		// Shoreline pass: stroke a thin water-coloured line so adjacent lake
		// quads tile seamlessly and the lake edge reads crisply against terrain.
		// Scale-compensated so it stays ~1px on screen at any zoom.
		const shoreLocal = this.scaleCompensatedWidth(1);
		for (const ring of rings) {
			lakeGfx.poly(ring).stroke({
				color: SHORE_LINE,
				width: shoreLocal,
				alpha: 0.55,
			});
		}
		lakeGfx.visible = this.layers.lakes;

		// Rivers: one polyline per river, traced through its ordered
		// `points` (already source-to-mouth from `compute_drainage`).
		if (!this.riverGfx) {
			this.riverGfx = new Graphics();
			this.riverGfx.label = "rivers";
			this.view.addChild(this.riverGfx);
		}
		const riverGfx = this.riverGfx;
		riverGfx.clear();
		const RIVER_COLOR = 0x3b6e8f;
		// Improved river rendering (Issue: "wide flow, scale with zoom, LOD").
		//
		// Width scales with ZOOM: a river's stroke is stored in world (normalized)
		// units so its on-screen thickness is `basePx * flow * zoomClamp` — zooming
		// in visibly thickens the river (it's part of the world geometry), instead
		// of the old scheme which clamped it to ~2px at every zoom. The width also
		// varies with `discharge`, so major rivers draw wider than tributaries.
		//
		// LOD: at low zoom the polyline is decimated (fewer stroke points) and the
		// stroke is faded by `zoomClamp`, keeping the zoomed-out map light without
		// losing the flow at high detail.
		const zoom = this.zoom;
		// Cap zoom's effect on thickness with a gentle clamp: never thinner than
		// 0.4x nor thicker than 3x the baseline, so rivers stay legible at both
		// extremes without balloons.
		const zoomClamp = Math.min(3, Math.max(0.4, zoom));
		const RIVER_BASE_PX = 2.5;
		const flowOf = (d: number) =>
			0.7 + 0.6 * Math.min(1.5, Math.log1p(Math.max(0, d ?? 1) / 4));
		// LOD: drop intermediate polyline points while zoomed out.
		const stride =
			zoom < 0.6 ? Math.max(1, Math.round((1 / Math.max(zoom, 0.15)) / 2)) : 1;
		// On-screen px of the widest river drawn (for tests / inspectors).
		this.riverStrokeWidth = 0;
		for (const river of rivers) {
			const pts = river.points;
			const n = pts.length;
			if (n < 2) continue;
			const flow = flowOf(river.discharge);
			// target on-screen px (includes the flow + zoom scale-up).
			const targetPx = RIVER_BASE_PX * flow * zoomClamp;
			// Convert to world-local (normalized [0,1]) width: dividing by the
			// CURRENT mean scale (base*zoom) makes the on-screen width = targetPx.
			const localWidth = Math.max(targetPx / this.currentMeanScale(), 1 / 4096);
			this.riverStrokeWidth = Math.max(this.riverStrokeWidth, targetPx);
			const first = pts[0];
			riverGfx.moveTo(nx(first[0]), ny(first[1]));
			for (let i = 1; i < n; i += stride) {
				const p = pts[i];
				riverGfx.lineTo(nx(p[0]), ny(p[1]));
			}
			// Always close the path on the final point so the mouth is painted
			// even when decimation skipped past it.
			const last = pts[n - 1];
			if ((n - 1) % stride !== 0) {
				riverGfx.lineTo(nx(last[0]), ny(last[1]));
			}
			riverGfx.stroke({
				color: RIVER_COLOR,
				width: localWidth,
				alpha: 0.92,
				cap: "round",
				join: "round",
			});
		}
		riverGfx.visible = this.layers.rivers;
	}

	/**
	 * Step 3.4: draw burg markers as circles on the burgGfx overlay.
	 *
	 * Each burg is a circle centered on its cell's centroid (`mesh.points[c]`),
	 * projected to normalized [0,1] space (same nx/ny as the rest of the map).
	 * The radius is in view-local units — the on-screen size is
	 * `localRadius * meanScale`, so we divide the desired px by the base fit
	 * scale to get a scale-compensated local radius (same pattern as
	 * rivers/selection, but using the base fit scale so markers scale with the
	 * world rather than staying absolutely fixed on zoom).
	 *
	 * Marker radius grows with population: capitals get a larger base + gold
	 * crown color; towns get a smaller radius proportional to `sqrt(pop)`.
	 *
	 * `fitToScreen` re-calls this on every camera change so markers stay
	 * correctly positioned; it no-ops (and hides) when the Burgs layer is off.
	 */
	private drawBurgs(): void {
		if (!this.burgGfx) return;
		const gfx = this.burgGfx;
		gfx.clear();
		gfx.visible = this.layers.burgs;
		this.burgRadius = 0;
		if (!this.layers.burgs || !this.burgGrid || this.burgPoints.length === 0) {
			return;
		}
		const grid = this.burgGrid;
		const { nx, ny } = makeNormalizer(grid.mesh.world_w, grid.mesh.world_h);
		// World-normalized radius. baseScaleX/Y is the fit-to-screen px scale;
		// dividing by it converts a desired px size into [0,1] world units.
		// The zoom multiplier is NOT applied here (unlike rivers/selection),
		// because burg markers should be part of the world — they scale with
		// view.scale (which already includes zoom). A minimum floor keeps them
		// visible at max-out zoom.
		const meanScale = (this.baseScaleX + this.baseScaleY) / 2;
		// Desired on-screen radius in px: capitals ~5px, towns scaled by pop.
		const desiredPx = (pop: number, capital: number) =>
			capital > 0 ? 5 : 2.5 + Math.min(2.5, Math.sqrt(pop) / 25);
		// Floor: at minimum zoom (0.15), keep burg ≥ ~2 px on screen so it's
		// never lost in the terrain.
		const minScreenPx = 2;
		const localRadius = (pop: number, capital: number) =>
			Math.max(
				minScreenPx / meanScale / Math.max(this.zoom, 0.15),
				desiredPx(pop, capital) / meanScale,
			);
		const pts = grid.mesh.points as unknown as [number, number][];
		gfx.beginFill(0x222222, 0.85);
		gfx.lineStyle(1 / 4096, 0xd4af37, 0.7);
		let maxScreen = 0;
		for (let i = 0; i < this.burgPoints.length; i++) {
			const b = this.burgPoints[i];
			if (!b || b.cell < 0 || b.cell >= pts.length) continue;
			const pt = pts[b.cell];
			if (!pt) continue;
			const r = localRadius(b.population, b.capital);
			gfx.drawCircle(nx(pt[0]), ny(pt[1]), r);
			if (b.capital > 0)
				maxScreen = Math.max(maxScreen, r * this.baseScaleX * this.zoom);
		}
		gfx.endFill();

		// Burg selection highlight (Issue: "burg selection UX"): draw a bright
		// gold ring around the currently selected burg marker so it stands out
		// from both the dark town dots and the yellow cell outline. Uses a
		// scale-compensated ring width (~1.5px on screen) so it stays crisply
		// visible at any zoom.
		const sel = this.selectedEntity;
		if (sel && sel.kind === "burg" && this.burgPoints.length) {
			const chosen = this.burgPoints.find((b) => b.id === sel.id);
			if (chosen && chosen.cell >= 0 && chosen.cell < pts.length) {
				const pt = pts[chosen.cell];
				if (pt) {
					const ringR = localRadius(chosen.population, chosen.capital);
					const ringLocal = this.scaleCompensatedWidth(1.5);
					gfx.lineStyle(ringLocal, 0xffd700, 0.95);
					gfx.drawCircle(nx(pt[0]), ny(pt[1]), ringR + ringLocal * 3);
				}
			}
		}
		// Track the largest on-screen radius for capitals (for tests).
		this.burgRadius = maxScreen;
	}

	/** Build the per-edge TRUE-neighbor map from geometry.
	 *
	 * The mesh's `cells.c` is NOT edge-aligned to `cells.v` (it's rotated by
	 * one per cell), so `cells.c[r]` is the wrong neighbor for segment
	 * `(v[r], v[r+1])`. Instead we derive the true neighbor directly: every
	 * interior Voronoi vertex is shared by exactly the 2 cells on either side
	 * of that edge, so for segment `r` of cell `c` (from vertex `v[r]` to
	 * `v[r+1]`) the true neighbor is the OTHER cell that also contains both
	 * `v[r]` and `v[r+1]`. We precompute `vertexToCells[vid]` (the list of
	 * cells owning `vid`), then for each edge pick the one non-`c` cell in the
	 * intersection of the two endpoints' cell lists. Edges on the hull
	 * (outer/clamped vertices) have only one owning cell -> `-1`.
	 *
	 * This is exact for every edge (interior, coast, and hull), unlike any
	 * `cells.c` rotation which breaks on spade's spurious hull neighbors.
	 */
	private buildEdgeNeighbor(grid: Grid): Int32Array {
		const cellI = grid.mesh.cells.i as unknown as number[];
		const cellV = grid.mesh.cells.v as unknown as number[];
		const n = grid.mesh.points.length;
		const totalEdges = cellV.length;
		const vertexToCells: number[][] = [];
		for (let c = 0; c < n; c++) {
			const lo = cellI[c] as number;
			const hi = (cellI[c + 1] as number) ?? lo;
			for (let r = lo; r < hi; r++) {
				const vid = cellV[r] as number;
				(vertexToCells[vid] ??= []).push(c);
			}
		}
		const edgeNeighbor = new Int32Array(totalEdges).fill(-1);
		for (let c = 0; c < n; c++) {
			const lo = cellI[c] as number;
			const hi = (cellI[c + 1] as number) ?? lo;
			const k = hi - lo;
			if (k < 3) continue;
			for (let r = lo; r < hi; r++) {
				const vid0 = cellV[r] as number;
				const vid1 =
					r + 1 < hi ? (cellV[r + 1] as number) : (cellV[lo] as number);
				const a = vertexToCells[vid0] ?? [];
				const b = vertexToCells[vid1] ?? [];
				// The true neighbor is the cell (other than c) present in both
				// endpoint lists. Interior edges have exactly 2 shared cells.
				let nb = -1;
				for (const x of a) {
					if (x === c) continue;
					if (b.includes(x)) {
						nb = x;
						break;
					}
				}
				edgeNeighbor[r] = nb;
			}
		}
		return edgeNeighbor;
	}

	/**
	 * stroke every state's outer boundary in white, drawn over the
	 * province fills so the state map reads on top (FMG draws state borders
	 * over provinces). Only visible while the Provinces layer is active.
	 *
	 * A state boundary runs along the edges between a state's cells and a
	 * DIFFERENT state's cells (or map water/void). For each cell edge we
	 * compute the shared normalized-space segment; if the two endpoints'
	 * states differ, we stroke that segment. Edges on the map border (a
	 * neighbor index == -1) are drawn only when the cell itself is land and
	 * belongs to a state, giving each state a coast/outline.
	 *
	 * The stroke width is scale-compensated (same pattern as rivers/selection)
	 * so it stays ~1.5 px on screen at any zoom. `fitToScreen` re-strokes on
	 * every pan/zoom/resize.
	 */
	private drawStateBorders(): void {
		if (!this.stateBorderGfx) {
			this.stateBorderGfx = new Graphics();
			this.stateBorderGfx.label = "state-borders";
			this.view.addChild(this.stateBorderGfx);
		}
		const gfx = this.stateBorderGfx;
		gfx.clear();
		const grid = this.stashGridForBorders;
		// Draw black state borders whenever the Provinces layer is shown OR a
		// state / province / religion is selected in the legend (per spec).
		const sel = this.selectedEntity;
		const selTriggersBorder =
			!!sel && (sel.kind === "state" || sel.kind === "province");
		if (!grid || (!this.layers.provinces && !selTriggersBorder)) {
			gfx.visible = false;
			this.stateBorderStrokeWidth = 0;
			return;
		}
		gfx.visible = true;
		const cellI = grid.mesh.cells.i as unknown as number[];
		const cellV = grid.mesh.cells.v as unknown as number[];
		const vpts = (grid.mesh.vertices?.p as unknown as [number, number][]) ?? [];
		const n = grid.mesh.points.length;
		const { nx, ny } = makeNormalizer(grid.mesh.world_w, grid.mesh.world_h);

		// Use the entity cells from setEntities (the authoritative per-cell
		// state assignment), NOT grid.cells.state (which is -1/unassigned
		// until the entity layer is filled). This is what makes state borders
		// render after the entity data arrives.
		const stateCells = this.entityCells?.state ?? null;

		const stateOf = (cellId: number) =>
			stateCells && cellId >= 0 && cellId < n ? stateCells[cellId] : -1;

		// Per-edge TRUE neighbor, derived from geometry (see buildEdgeNeighbor).
		// This replaces the mesh's `cells.c`, which is rotated one slot and so
		// would match each segment against the wrong neighbor.
		const edgeNb = this.edgeNeighbor;

		// Accumulate every border segment into ONE path, then stroke once.
		// (Per-segment stroke() calls are O(segments) expensive in PixiJS;
		// batching keeps the border overlay a single draw call.)
		let anySegment = false;
		for (let c = 0; c < n; c++) {
			const myState = stateOf(c);
			if (myState < 0) continue; // unassigned cell — no border
			const lo = cellI[c] as number;
			const hi = (cellI[c + 1] as number) ?? lo;
			if (hi <= lo) continue;
			for (let r = lo; r < hi; r++) {
				const vid0 = cellV[r] as number;
				const vid1 =
					r + 1 < hi ? (cellV[r + 1] as number) : (cellV[lo] as number);
				const p0 = vpts[vid0];
				const p1 = vpts[vid1];
				if (!p0 || !p1) continue;
				// TRUE neighbor of this segment, or -1 (hull/coast edge).
				const nb = edgeNb ? edgeNb[r] : -1;
				const nbState = stateOf(nb);
				if (nbState !== myState) {
					gfx.moveTo(nx(p0[0]), ny(p0[1]));
					gfx.lineTo(nx(p1[0]), ny(p1[1]));
					anySegment = true;
				}
			}
		}

		if (!anySegment) {
			// No border segments to draw (all cells unassigned or no entity
			// data). Report 0 stroke width so tests/inspectors see "no
			// borders" rather than a phantom 1.5px.
			this.stateBorderStrokeWidth = 0;
			return;
		}

		// Scale-compensate the stroke width (~1.5 px on screen).
		const localWidth = this.scaleCompensatedWidth(1.5);
		this.stateBorderStrokeWidth = localWidth * this.currentMeanScale();

		gfx.stroke({ color: 0x000000, width: localWidth, alpha: 0.95 });
	}

	/** Grid stashed for state-border re-stroking (set in setEntities/setGrid). */
	private stashGridForBorders: Grid | null = null;

	private applyLayers(): void {
		if (this.terrainMesh) this.terrainMesh.visible = this.layers.terrain;
		if (this.biomeMesh) this.biomeMesh.visible = this.layers.biome;
		if (this.riverGfx) this.riverGfx.visible = this.layers.rivers;
		if (this.lakeGfx) this.lakeGfx.visible = this.layers.lakes;
		if (this.stateMesh) this.stateMesh.visible = this.layers.states;
		if (this.provinceMesh) this.provinceMesh.visible = this.layers.provinces;
		if (this.cultureMesh) this.cultureMesh.visible = this.layers.cultures;
		if (this.religionMesh) this.religionMesh.visible = this.layers.religions;
		// Rivers/lakes: re-tessellate from the stashed geometry whenever a
		// relevant layer is visible (no-op when both are hidden), so a toggle-on
		// re-strokes at the current camera scale instead of showing a buffer
		// primed at a stale scale. When hidden this is a cheap early-return.
		this.drawRiversLakes();
		// State borders render on top of the provinces fill. Their visibility is
		// managed entirely by drawStateBorders (which also shows them when a
		// state/province is selected even with the Provinces layer off, honoring
		// selTriggersBorder) — do NOT force `visible = layers.provinces` here, or
		// turning the layer off would hide borders that a selection still wants.
		this.drawStateBorders();
		// Burgs overlay: re-draw at the current scale (visible flag checked
		// inside drawBurgs).
		this.drawBurgs();
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

	/**
	 *
	 *
	 * `pack` holds the entity color vectors (`states[i].color` etc., packed
	 * 0xRRGGBB). `cells_state[i]` is the owning entity id for cell `i` (-1 if
	 * unassigned). We fill each layer's 1×N RGBA buffer — texel `cell` gets
	 * the color of `pack_X[cells_X[cell]]`, or transparent (alpha 0) if the
	 * cell has no entity — then `source.update()` uploads it. Unassigned cells
	 * show the terrain beneath.
	 *
	 * This is the entity-layer equivalent of `updateHeight`: O(N) CPU fill +
	 * one texture upload, NO geometry rebuild. Call `applyLayers()` afterwards
	 * is not needed — visibility is unchanged; only the texture data updates.
	 *
	 * @param grid  the Grid (used for cell count + to stash for selection).
	 * @param result (states pack + per-cell arrays + culture/religion vectors + per-cell arrays).
	 */
	setEntities(
		grid: Grid,
		result: {
			pack: {
				states: { color: number }[];
				provinces: { color: number }[];
				cultures: { color: number }[];
				religions: { color: number }[];
			};
			cells_state: number[];
			cells_province: number[];
			cells_culture: number[];
			cells_religion: number[];
		},
	): void {
		this.stashGridForBorders = grid;
		const n = grid.mesh.points.length;
		// record the base pack as the active source-of-truth so a
		// later in-place `patchEntityCells` re-fills from it (and a projected
		// `updateEntities` supersedes it without leaving stale refs behind).
		this.activePack = {
			states: result.pack.states,
			provinces: result.pack.provinces,
			cultures: result.pack.cultures,
			religions: result.pack.religions,
		};
		this.entityCells = {
			state: result.cells_state,
			province: result.cells_province,
			culture: result.cells_culture,
			religion: result.cells_religion,
		};
		this.fillEntityBuffer(
			this.stateData,
			n,
			result.pack.states,
			result.cells_state,
			-1, // -1 == unassigned state
			1, // state ids are 1-based; subtract to index pack.states (0-based)
		);
		this.fillEntityBuffer(
			this.provinceData,
			n,
			result.pack.provinces,
			result.cells_province,
			-1,
			1, // province ids are 1-based
		);
		this.fillEntityBuffer(
			this.cultureData,
			n,
			result.pack.cultures,
			result.cells_culture,
			0, // 0 == Wildlands (no culture)
			0, // culture ids are 0-based
		);
		this.fillEntityBuffer(
			this.religionData,
			n,
			result.pack.religions,
			result.cells_religion,
			0, // 0 == no religion
			0, // religion ids are 0-based
		);
		// Upload all four textures.
		this.textures[2]?.source.update();
		this.textures[3]?.source.update();
		this.textures[4]?.source.update();
		this.textures[5]?.source.update();
		// Re-draw state borders now that the entity cells are populated
		// (drawStateBorders reads this.entityCells.state, which was just
		// filled — without this call the borders never render after the
		// entity data arrives).
		this.drawStateBorders();
	}

	clearEntities(): void {
		if (this.stateData) this.stateData.fill(0);
		if (this.provinceData) this.provinceData.fill(0);
		if (this.cultureData) this.cultureData.fill(0);
		if (this.religionData) this.religionData.fill(0);
		this.entityCells = null;
		this.activePack = null;
		this.selectedEntity = null;
		this.textures[2]?.source.update();
		this.textures[3]?.source.update();
		this.textures[4]?.source.update();
		this.textures[5]?.source.update();
	}

	/**
	 * refresh the four entity data-texture buffers, the state
	 * borders, and the burg markers from the CURRENT grid's per-cell entity
	 * arrays after an in-place edit (heightmap/entity repair) that leaves the
	 * mesh unchanged. No geometry rebuild — this is the entity-layer equivalent
	 * of `updateHeight`/`updateBiome`.
	 *
	 * Fill colors come from `activePack`: the last source that
	 * wrote the buffers — the base `setEntities` pack OR the projected
	 * `WorldAt` pack — never a stale combination. This is what guarantees a
	 * height-edit / timeline scrub never reverts the layer to the wrong year.
	 *
	 * Normalises the grid's per-cell conventions to the internal `i32` form
	 * (`-1`/`0` sentinels) before re-filling, so border drawing + click-to-
	 * select keep working unchanged.
	 */
	patchEntityCells(grid: Grid): void {
		const pack = this.activePack;
		if (!pack) return; // no entity source loaded yet — nothing to refresh
		const n = grid.mesh.points.length;
		this.stashGridForBorders = grid;

		// Normalise each per-cell array to the `fillEntityBuffer` convention:
		//   state/province: u32 0  -> -1  (unassigned sentinel; ids 1-based)
		//   culture/religion: 0    -> 0   (id 0 IS the "none" sentinel)
		const normState = grid.cells.state.map((v) => (v <= 0 ? -1 : v));
		const normProvince = grid.cells.province.map((v) => (v <= 0 ? -1 : v));
		const culture = grid.cells.culture;
		const religion = grid.cells.religion;

		this.entityCells = {
			state: normState,
			province: normProvince,
			culture,
			religion,
		};

		this.fillEntityBuffer(this.stateData, n, pack.states, normState, -1, 1);
		this.fillEntityBuffer(
			this.provinceData,
			n,
			pack.provinces,
			normProvince,
			-1,
			1,
		);
		this.fillEntityBuffer(this.cultureData, n, pack.cultures, culture, 0, 0);
		this.fillEntityBuffer(this.religionData, n, pack.religions, religion, 0, 0);

		this.textures[2]?.source.update();
		this.textures[3]?.source.update();
		this.textures[4]?.source.update();
		this.textures[5]?.source.update();
		this.drawStateBorders();
	}

	/**
	 * recolor ONE entity across every cell that belongs to it without
	 * re-uploading all four entity layers. Scans only the matching per-cell
	 * array, rewrites the affected texels' RGB in the buffer, then uploads just
	 * that texture. When a projected `WorldAt` is the active source, the new
	 * color is written straight into the stored `activePack` vector for that
	 * entity so re-fills (`patchEntityCells`) stay consistent.
	 *
	 * @param kind  the entity kind (state/province/culture/religion)
	 * @param id    the entity id (1-based for state/province, 0-based for
	 *              culture/religion)
	 * @param color packed 0xRRGGBB
	 */
	updateEntityColor(kind: EntityKind, id: number, color: number): void {
		const pack = this.activePack;
		const cells = this.entityCells;
		if (!pack || !cells) return;
		let buf: Uint8Array | null = null;
		let cellArr: number[];
		let vector: { color: number }[] | undefined;
		switch (kind) {
			case "state":
				buf = this.stateData;
				cellArr = cells.state;
				vector = pack.states;
				break;
			case "province":
				buf = this.provinceData;
				cellArr = cells.province;
				vector = pack.provinces;
				break;
			case "culture":
				buf = this.cultureData;
				cellArr = cells.culture;
				vector = pack.cultures;
				break;
			case "religion":
				buf = this.religionData;
				cellArr = cells.religion;
				vector = pack.religions;
				break;
			case "burg":
				// Burgs are a point overlay, not a fill layer — no texture to
				// patch here.
				buf = null;
				cellArr = [];
				vector = undefined;
				break;
		}
		if (!buf) return;

		// idBias + unassigned sentinel per kind (mirrors fillEntityBuffer).
		const idBias = kind === "state" || kind === "province" ? 1 : 0;
		const idx = id - idBias;
		const maxId = (vector?.length ?? 0) - 1;
		if (idx < 0 || idx > maxId) return;

		const r = (color >> 16) & 0xff;
		const g = (color >> 8) & 0xff;
		const b = color & 0xff;
		const n = cellArr.length;
		let changed = false;
		for (let i = 0; i < n; i++) {
			if (cellArr[i] === id) {
				const o = i * 4;
				buf[o + 0] = r;
				buf[o + 1] = g;
				buf[o + 2] = b;
				buf[o + 3] = 255;
				changed = true;
			}
		}
		if (changed && vector) {
			// Keep the active source in sync so a later full re-fill
			// (`patchEntityCells`) preserves the edited color.
			vector[idx] = { ...vector[idx], color };
		}
		// Upload just the affected texture (global texel index = the layer's
		// data-buffer index).
		const texIndex =
			kind === "state"
				? 2
				: kind === "province"
					? 3
					: kind === "culture"
						? 4
						: 5;
		if (changed) this.textures[texIndex]?.source.update();
	}

	/**
	 * Live morph (Step 5.2): update entity data textures + borders from a
	 * projected `WorldAt` WITHOUT rebuilding base geometry.
	 *
	 * `WorldAt` uses the `u32` per-cell convention (`0` = unassigned) for
	 * state/province/culture/religion, while the internal `entityCells` stash
	 * and `drawStateBorders` expect the `i32` convention (`-1` = unassigned)
	 * for state/province. We normalise during the copy so border drawing and
	 * click-to-select keep working unchanged.
	 *
	 * `cells_province` is optional: the Rust `WorldAt` struct does not carry it
	 * (province cell assignments are stable across projection — only state
	 * ownership changes). When absent, the province layer is left as-is from
	 * the initial `setEntities` call.
	 *
	 * @param world  the projected world at the target year
	 * @param grid   the cached grid (for cell count). Falls back to
	 *               `stashGridForBorders` when omitted.
	 */
	updateEntities(world: WorldAt, grid?: Grid): void {
		const g = grid ?? this.stashGridForBorders;
		if (!g) return;
		const n = g.mesh.points.length;
		this.stashGridForBorders = g;

		// Normalise state cells: u32 0 -> i32 -1 (unassigned sentinel).
		// Also accept i32 -1 (the year-0 convention) for robustness.
		const stateCells: number[] = [];
		for (const v of world.cells_state) stateCells.push(v <= 0 ? -1 : v);

		// Province cells: optional. Normalise u32 0 -> -1 when present and
		// non-empty (the Rust WorldAt struct does not carry province cells).
		const provinceCells: number[] | null =
			world.cells_province && world.cells_province.length > 0
				? Array.from(world.cells_province, (v: number) => (v <= 0 ? -1 : v))
				: null;

		// Culture/religion: 0 = none/Wildlands — same sentinel in both
		// conventions, so no normalisation needed.
		const cultureCells = world.cells_culture;
		const religionCells = world.cells_religion;

		// Stash for click-to-select + border drawing.
		this.entityCells = {
			state: stateCells,
			province: provinceCells ?? this.entityCells?.province ?? [],
			culture: cultureCells,
			religion: religionCells,
		};

		// The projected WorldAt supersedes the base pack as the
		// active source-of-truth, so any later in-place `patchEntityCells`
		// re-fills from the projected vectors, not the year-0 ones.
		this.activePack = {
			states: world.pack.states,
			provinces: world.pack.provinces,
			cultures: world.pack.cultures,
			religions: world.pack.religions,
		};

		// Re-fill the four data-texture buffers (texture swap, no tessellation).
		this.fillEntityBuffer(
			this.stateData,
			n,
			world.pack.states,
			stateCells,
			-1, // i32 convention after normalisation
			1, // state ids are 1-based
		);
		this.fillEntityBuffer(
			this.provinceData,
			n,
			world.pack.provinces,
			this.entityCells.province,
			-1,
			1, // province ids are 1-based
		);
		this.fillEntityBuffer(
			this.cultureData,
			n,
			world.pack.cultures,
			cultureCells,
			0, // 0 == Wildlands / no culture
			0, // culture ids are 0-based
		);
		this.fillEntityBuffer(
			this.religionData,
			n,
			world.pack.religions,
			religionCells,
			0, // 0 == no religion
			0, // religion ids are 0-based
		);

		// Upload all four textures.
		this.textures[2]?.source.update();
		this.textures[3]?.source.update();
		this.textures[4]?.source.update();
		this.textures[5]?.source.update();

		// Re-draw state borders from projected ownership.
		this.drawStateBorders();
	}

	/**
	 * Fill one entity layer's 1×N RGBA buffer. For each cell `i` in
	 * `[0, count)`, look up its entity id `eid = cells[i]`; if `eid` is valid
	 * (>= 0, != the unassigned sentinel, and within `entities`), write the
	 * packed `0xRRGGBB` color at texel `i` with alpha 255; otherwise leave the
	 * texel transparent (alpha 0).
	 *
	 * `idBias` compensates for the wire encoding of entity ids: states /
	 * provinces use 1-based ids (0 is unused, -1 is unassigned), so we
	 * subtract `1` to index the 0-based `pack.*` vectors; cultures /
	 * religions use 0-based ids (0 is a valid "none" sentinel, not -1), so
	 * `idBias` is 0 there.
	 */
	private fillEntityBuffer(
		buf: Uint8Array | null,
		count: number,
		entities: { color: number }[],
		cells: number[],
		unassigned: number,
		idBias: number,
	): void {
		if (!buf) return;
		const maxId = entities.length - 1;
		// Unassigned cell -> fully clear the texel (RGB + alpha 0) so a
		// removed/re-dissolved entity leaves a completely blank texel rather
		// than stale colour bytes that just happen to be hidden by alpha 0.
		const clear = (offset: number) => {
			buf[offset + 0] = 0;
			buf[offset + 1] = 0;
			buf[offset + 2] = 0;
			buf[offset + 3] = 0;
		};
		for (let i = 0; i < count; i++) {
			const eid = cells[i] ?? unassigned;
			const o = i * 4;
			if (eid === unassigned || eid < 0) {
				clear(o);
				continue;
			}
			// Map the (possibly 1-based) wire id onto the 0-based entity vec.
			const idx = eid - idBias;
			if (idx < 0 || idx > maxId) {
				clear(o);
				continue;
			}
			const color = entities[idx].color;
			buf[o + 0] = (color >> 16) & 0xff;
			buf[o + 1] = (color >> 8) & 0xff;
			buf[o + 2] = color & 0xff;
			buf[o + 3] = 255;
		}
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
		this.stateMesh = null;
		this.provinceMesh = null;
		this.cultureMesh = null;
		this.religionMesh = null;
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
		this.stateData = null;
		this.provinceData = null;
		this.cultureData = null;
		this.religionData = null;
		// Drop the selection refs so a dangling re-stroke (e.g. a queued
		// `fitToScreen` after destroy) finds no cell and no-ops.
		this.selectionGfx = null;
		this.selectedGrid = null;
		this.selectedCellId = -1;
		this.selectionStrokeWidth = 0;
		// Step 3.4: drop entity-layer refs + selection so a dangling re-stroke
		// finds no data and no-ops.
		this.entityCells = null;
		this.activePack = null;
		this.selectedEntity = null;
		// Drop the geometry-derived edge-neighbor map (depends on the mesh
		// that backs this instance) so a dangling re-stroke finds nothing.
		this.edgeNeighbor = null;
		// Step 2.5.6: drop the river/lake overlay refs so a dangling
		// re-stroke (a queued `fitToScreen` after destroy) finds no
		// drainage and no-ops.
		this.riverGfx = null;
		this.lakeGfx = null;
		this.drainageGrid = null;
		this.drainageRivers = [];
		this.drainageLakes = [];
		this.riverStrokeWidth = 0;
		// Step 3.4: drop the burg overlay refs so a dangling re-draw finds
		// no data and no-ops.
		this.burgGrid = null;
		this.burgPoints = [];
		this.burgRadius = 0;
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
		/**
		 * Returns true when the active editor tool is the hand/pan tool. When
		 * true, a single-pointer drag pans the camera (no Space modifier needed).
		 * This replaces the old "hold Space to pan" flow: Space still acts as an
		 * ad-hoc pan modifier, but the pan tool makes single-finger drags pan on
		 * their own — required for touch devices where Space isn't available.
		 */
		isPanMode?: () => boolean;
	} = { worldMap: null as never, screenSize: () => ({ w: 0, h: 0 }) },
): () => void {
	const zoomMin = opts.zoomMin ?? 0.15;
	const zoomMax = opts.zoomMax ?? 24;
	const { worldMap, screenSize } = opts;
	const isPanMode = opts.isPanMode ?? (() => false);
	let dragging = false;
	let lastX = 0;
	let lastY = 0;
	// Spacebar-to-pan: the camera only drag-pans a single pointer while Space is
	// held, so the editor gets clean pointer events for brush/select when Space
	// is up. This matches Figma/Photoshop's hand-tool modifier. We track Space
	// with a window keydown/keyup pair (not the canvas) so a held Space is
	// recognized even if focus is on a sibling control. In addition, activating
	// the standalone Pan tool (see `isPanMode`) makes single-pointer drags pan
	// without Space.
	let spaceDown = false;

	// Multi-touch gesture state (mobile): track every active pointer so two
	// fingers give pinch-to-zoom + two-finger pan. Single-finger gestures are
	// left to the editor (brush/select) — the camera deliberately does NOT
	// single-pointer-pan unless Space is held, so editing stays intact on touch.
	const pointers = new Map<number, { x: number; y: number }>();
	let gestureActive = false;
	let lastDist = 0;
	let lastMidX = 0;
	let lastMidY = 0;

	const activePointers = () => [...pointers.values()];
	const gestureMidpoint = (): { x: number; y: number } => {
		const pts = activePointers();
		let x = 0;
		let y = 0;
		for (const p of pts) {
			x += p.x;
			y += p.y;
		}
		const n = pts.length || 1;
		return { x: x / n, y: y / n };
	};
	const gestureDistance = (): number => {
		const pts = activePointers();
		if (pts.length < 2) return 0;
		return Math.hypot(pts[0].x - pts[1].x, pts[0].y - pts[1].y);
	};
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
		pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
		// A second finger enters pinch-to-zoom / two-finger pan mode. Record
		// the starting distance + midpoint so onMove can compute deltas.
		if (pointers.size >= 2) {
			gestureActive = true;
			dragging = false;
			lastDist = gestureDistance();
			const m = gestureMidpoint();
			lastMidX = m.x;
			lastMidY = m.y;
			target.setPointerCapture?.(e.pointerId);
			return;
		}
		// Single pointer pans while Space is held OR while the Pan tool is the
		// active editor tool (the editor owns the pointer otherwise). Without
		// either, plain pointerdown does not start a pan so brush/select stay
		// clean.
		if (!spaceDown && !isPanMode()) return;
		dragging = true;
		lastX = e.clientX;
		lastY = e.clientY;
		target.setPointerCapture?.(e.pointerId);
	};
	const onMove = (e: PointerEvent) => {
		if (!pointers.has(e.pointerId)) return;
		pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
		const { w, h } = screenSize();
		// Multi-touch gesture: pinch-to-zoom around the midpoint + pan by
		// midpoint movement (two-finger drag). No Space needed.
		if (gestureActive && pointers.size >= 2) {
			const d = gestureDistance();
			const m = gestureMidpoint();
			const rect = target.getBoundingClientRect();
			if (lastDist > 0 && d > 0) {
				worldMap.setZoom(worldMap.getZoom() * (d / lastDist), w, h, {
					x: m.x - rect.left,
					y: m.y - rect.top,
				});
			}
			worldMap.panBy(m.x - lastMidX, m.y - lastMidY, w, h);
			lastDist = d;
			lastMidX = m.x;
			lastMidY = m.y;
			return;
		}
		if (!dragging) return;
		worldMap.panBy(e.clientX - lastX, e.clientY - lastY, w, h);
		lastX = e.clientX;
		lastY = e.clientY;
	};
	const endPointer = (e: PointerEvent) => {
		pointers.delete(e.pointerId);
		if (pointers.size < 2) {
			gestureActive = false;
			lastDist = 0;
		}
		if (dragging) {
			dragging = false;
			target.releasePointerCapture?.(e.pointerId);
		}
	};
	const onUp = (e: PointerEvent) => endPointer(e);
	const onCancel = (e: PointerEvent) => endPointer(e);

	target.addEventListener("wheel", onWheel, { passive: false });
	target.addEventListener("pointerdown", onDown);
	target.addEventListener("pointermove", onMove);
	target.addEventListener("pointerup", onUp);
	target.addEventListener("pointercancel", onCancel);
	target.addEventListener("pointerleave", onUp);

	return () => {
		target.removeEventListener("wheel", onWheel);
		target.removeEventListener("pointerdown", onDown);
		target.removeEventListener("pointermove", onMove);
		target.removeEventListener("pointerup", onUp);
		target.removeEventListener("pointercancel", onCancel);
		target.removeEventListener("pointerleave", onUp);
		window.removeEventListener("keydown", onKeyDown);
		window.removeEventListener("keyup", onKeyUp);
	};
}

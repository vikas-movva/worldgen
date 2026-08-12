// World map layers (Step 2.3).
//
// Builds the terrain + biome render from a `Grid` using the merged geometry
// (buildWorldGeometry in buildGeometry.ts) and the data-texture color pattern
// (palette.ts).
//
// Design (see worldbuilding-tool-design.md, "Render layers"): every cell is one
// polygon in a SINGLE vertex/index buffer. Each vertex stores its cell id as a
// UV, which indexes a 1×N cell-id data texture. Colouring is then a pure texture
// swap:
//   - terrain  -> heightmap gradient texture (h -> RGBA)
//   - biome    -> biome palette texture (biome id -> RGBA)
// Two Mesh objects share ONE geometry (one draw call each) and differ only by
// their texture. Toggling a layer is `mesh.visible = ...` -- no re-tessellation.
//
// A small camera (drag-to-pan, wheel-to-zoom) transforms `view` so the map can
// be inspected at 60k cells without re-tessellation.

import { Container, Mesh, Texture, TextureStyle } from "pixi.js";
import type { Grid } from "../core/api";
import { buildWorldGeometry, type MeshGeometryData } from "./buildGeometry";
import { buildBiomeTextureData, buildHeightTextureData } from "./palette";

export type LayerName = "terrain" | "biome";
export type LayerState = Record<LayerName, boolean>;

const STYLE = new TextureStyle({
	scaleMode: "nearest",
	addressMode: "clamp-to-edge",
});

/** Build a 1×N RGBA data texture (one texel per cell) from a Uint8Array. */
function dataTexture(data: Uint8Array, cellCount: number): Texture {
	return Texture.from(
		{ resource: data, width: cellCount, height: 1, style: STYLE, format: "rgba8unorm" },
	);
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

	constructor(grid: Grid, opts: { initialLayers?: Partial<LayerState> } = {}) {
		if (opts.initialLayers) Object.assign(this.layers, opts.initialLayers);
		const geoData = buildWorldGeometry(grid);
		this.geometry = geoData.geometry;
		const heightTex = dataTexture(
			buildHeightTextureData(grid.cells.h),
			geoData.cellCount,
		);
		const biomeTex = dataTexture(
			buildBiomeTextureData(grid.cells.biome),
			geoData.cellCount,
		);
		this.textures = [heightTex, biomeTex];

		this.terrainMesh = new Mesh({ geometry: this.geometry, texture: heightTex });
		this.biomeMesh = new Mesh({ geometry: this.geometry, texture: biomeTex });

		this.view = new Container({ isRenderGroup: true });
		this.view.addChild(this.terrainMesh, this.biomeMesh);
		this.applyLayers();
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
			t.destroy(true);
		});
		this.textures = [];
	}
}

/**
 * Attach pan/zoom camera controls to a Pixi `Container` (the world map view).
 * Returns a detach function that removes the listeners.
 */
export function attachCamera(
	view: Container,
	target: HTMLElement,
	bounds = { min: 0.15, max: 24 },
): () => void {
	let dragging = false;
	let lastX = 0;
	let lastY = 0;

	const onWheel = (e: WheelEvent) => {
		e.preventDefault();
		const factor = Math.exp(-e.deltaY * 0.0015);
		const next = Math.max(bounds.min, Math.min(bounds.max, view.scale.x * factor));
		view.scale.set(next);
	};
	const onDown = (e: PointerEvent) => {
		dragging = true;
		lastX = e.clientX;
		lastY = e.clientY;
		target.setPointerCapture?.(e.pointerId);
	};
	const onMove = (e: PointerEvent) => {
		if (!dragging) return;
		view.x += e.clientX - lastX;
		view.y += e.clientY - lastY;
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

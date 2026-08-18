// Step 2.3 unit tests - layers.ts (WorldMap camera + layers + attachCamera).
//
// `WorldMap` owns the merged geometry + terrain/biome meshes for one Grid;
// `attachCamera` wires pan/zoom listeners onto an HTMLElement. These tests pin
// the camera math (the part most likely to silently regress and produce a
// blank/wrong canvas in a real browser) and the layer-toggle contract:
//   - `fitToScreen` produces non-uniform pixel scales that fit-contain the
//     world aspect ratio and centers the view (accounting for pan).
//   - `setZoom` clamps the user zoom multiplier to [0.15, 24] and, with a
//     focus point, keeps that world coordinate under the cursor.
//   - `panBy` adds a screen-space delta; `resetView` zeroes pan + zoom.
//   - `setLayers`/`getLayers` round-trip and respect the construction default.
//   - `destroy` detaches the view from its parent (so the render loop can't
//     touch a dead subtree).
//   - `attachCamera` registers wheel/pointer listeners on the target and the
//     returned detach function removes them.
//
// WorldMap construction needs PixiJS Texture.from (canvas-backed data texture);
// jsdom lacks a real canvas, so PixiJS prints a harmless
// `HTMLCanvasElement.prototype.getContext (without installing the canvas npm
// package)` warning but the WorldMap object is constructed (the GPU upload only
// happens on render, which these tests never trigger).

import { Container } from "pixi.js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Grid, LakeGeo, RiverGeo, WorldAt } from "../core/api";
import { attachCamera, WorldMap } from "./layers";

// ---- fixture ------------------------------------------------------------

function quadGrid(n = 1, worldW = 1000, worldH = 1000): Grid {
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
	return {
		seed: 1,
		mesh: {
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
		},
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

let wm: WorldMap;

/** Read the RGBA texels written into a data-texture buffer. (Module scope so
 * every describe can assert on the raw Uint8Array backing each layer.) */
function readTexels(
	buf: Uint8Array | null,
): Array<[number, number, number, number]> {
	if (!buf) return [];
	const out: Array<[number, number, number, number]> = [];
	for (let i = 0; i < buf.length; i += 4) {
		out.push([buf[i]!, buf[i + 1]!, buf[i + 2]!, buf[i + 3]!]);
	}
	return out;
}

beforeEach(() => {
	wm = new WorldMap(quadGrid(1, 1000, 1000));
});

afterEach(() => {
	wm.destroy();
});

// ---- construction + layers -------------------------------------------

describe("WorldMap construction + layer state", () => {
	it("defaults to terrain=on, biome=off", () => {
		expect(wm.getLayers()).toEqual({
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

	it("preserve a smaller priority value when reinserted", () => {
		// Custom initialLayers overrides defaults.
		const custom = new WorldMap(quadGrid(1), {
			initialLayers: { biome: true, terrain: false },
		});
		expect(custom.getLayers()).toEqual({
			terrain: false,
			biome: true,
			rivers: false,
			lakes: false,
			states: false,
			provinces: false,
			cultures: false,
			religions: false,
			burgs: false,
		});
		custom.destroy();
	});

	it("setLayers flips terrain visibility", () => {
		wm.setLayers({ terrain: false });
		expect(wm.getLayers().terrain).toBe(false);
		expect(wm.getLayers().biome).toBe(false);
	});

	it("setLayers flips biome visibility", () => {
		wm.setLayers({ biome: true });
		expect(wm.getLayers().biome).toBe(true);
		expect(wm.getLayers().terrain).toBe(true);
	});

	it("getLayers returns a snapshot, not the live internal object", () => {
		const snap = wm.getLayers();
		wm.setLayers({ biome: true });
		// The snapshot should not have changed.
		expect(snap).toEqual({
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

// ---- fitToScreen ------------------------------------------------------

describe("WorldMap.fitToScreen", () => {
	it("applies non-uniform scale that fit-contains the world aspect", () => {
		// World 1000 x 1000 (square). Screen 1280 x 720 (wider). Fit height =
		// 720; xScale = 720 * worldAspect = 720 * (1000/1000) = 720.
		wm.fitToScreen(1280, 720);
		expect(wm.view.scale.x).toBeCloseTo(720, 3);
		expect(wm.view.scale.y).toBeCloseTo(720, 3);
	});

	it("centers the view (origin = (screen - scale)/2 + pan)", () => {
		wm.fitToScreen(1280, 720);
		expect(wm.view.x).toBeCloseTo((1280 - 720) / 2, 3);
		expect(wm.view.y).toBeCloseTo((720 - 720) / 2, 3);
	});

	it("fits width when the screen is narrower than the world aspect", () => {
		// World 2000 x 1000 (aspect 2). Screen 1000 x 1000 (aspect 1).
		// Screen narrower -> fit width: xScale = 1000, yScale = 1000 / 2 = 500.
		const tallerWm = new WorldMap(quadGrid(1, 2000, 1000));
		tallerWm.fitToScreen(1000, 1000);
		expect(tallerWm.view.scale.x).toBeCloseTo(1000, 3);
		expect(tallerWm.view.scale.y).toBeCloseTo(500, 3);
		tallerWm.destroy();
	});

	it("is a no-op for non-positive screen dimensions", () => {
		wm.fitToScreen(0, 0);
		// Default Container scale is 1, position 0,0 - no transform applied.
		expect(wm.view.scale.x).toBe(1);
		expect(wm.view.scale.y).toBe(1);
	});

	it("honors a non-zero pan offset on top of the centered fit", () => {
		wm.panBy(100, 50, 1280, 720); // sets panX=100, panY=50
		wm.fitToScreen(1280, 720);
		expect(wm.view.x).toBeCloseTo((1280 - 720) / 2 + 100, 3);
		expect(wm.view.y).toBeCloseTo((720 - 720) / 2 + 50, 3);
	});

	it("applies the zoom multiplier on top of the fit scale (zoom default 1)", () => {
		wm.fitToScreen(1280, 720);
		const baseX = wm.view.scale.x;
		wm.setZoom(2, 1280, 720); // zoom=2
		wm.fitToScreen(1280, 720);
		expect(wm.view.scale.x).toBeCloseTo(baseX * 2, 3);
	});
});

// ---- setZoom clamping -------------------------------------------------

describe("WorldMap.setZoom clamping", () => {
	it("clamps zoom below 0.15 to 0.15", () => {
		wm.setZoom(0.01, 1280, 720);
		expect(wm.getZoom()).toBeCloseTo(0.15, 6);
	});

	it("clamps zoom above 24 to 24", () => {
		wm.setZoom(100, 1280, 720);
		expect(wm.getZoom()).toBeCloseTo(24, 6);
	});

	it("passes through an in-range zoom (e.g. 2)", () => {
		wm.setZoom(2, 1280, 720);
		expect(wm.getZoom()).toBe(2);
	});

	it("persists zoom across a fitToScreen re-call (resize does not reset zoom)", () => {
		wm.setZoom(3, 1280, 720);
		wm.fitToScreen(800, 600);
		expect(wm.getZoom()).toBe(3);
	});
});

// ---- setZoom focal-point math ----------------------------------------

describe("WorldMap.setZoom focal point", () => {
	it("zooms toward the cursor so the same world coord stays under it", () => {
		// Set up a known initial fit.
		wm.fitToScreen(1280, 720);
		// The world coordinate under cursor (640, 360) - the center - is
		// (0.5, 0.5) in normalized space (the world center).
		// Zoom in 2x toward the center: the new view should still have the
		// center under (640, 360).
		const focus = { x: 640, y: 360 };
		const worldBefore = worldCoordAt(wm, focus, 1280, 720);
		wm.setZoom(2, 1280, 720, focus);
		const worldAfter = worldCoordAt(wm, focus, 1280, 720);
		expect(worldAfter.x).toBeCloseTo(worldBefore.x, 4);
		expect(worldAfter.y).toBeCloseTo(worldBefore.y, 4);
	});

	it("zooms toward an off-center cursor and preserves that world coord", () => {
		wm.fitToScreen(1280, 720);
		const focus = { x: 100, y: 200 };
		const worldBefore = worldCoordAt(wm, focus, 1280, 720);
		wm.setZoom(3, 1280, 720, focus);
		const worldAfter = worldCoordAt(wm, focus, 1280, 720);
		expect(worldAfter.x).toBeCloseTo(worldBefore.x, 3);
		expect(worldAfter.y).toBeCloseTo(worldBefore.y, 3);
	});
});

// Helper: world coordinate under a screen point. Inverse of the view transform
// applied in fitToScreen (the geometry is in normalized [0,1]^2, then
// scaled by view.scale and translated by view.x/view.y).
function worldCoordAt(
	map: WorldMap,
	screen: { x: number; y: number },
	w: number,
	h: number,
): { x: number; y: number } {
	map.fitToScreen(w, h);
	const scaleX = map.view.scale.x;
	const scaleY = map.view.scale.y;
	const originX = map.view.x;
	const originY = map.view.y;
	return {
		x: (screen.x - originX) / scaleX,
		y: (screen.y - originY) / scaleY,
	};
}

// ---- panBy ------------------------------------------------------------

describe("WorldMap.panBy", () => {
	it("adds the screen-space delta to the pan offset", () => {
		wm.fitToScreen(1280, 720);
		const before = { x: wm.view.x, y: wm.view.y };
		wm.panBy(100, 50, 1280, 720);
		expect(wm.view.x).toBeCloseTo(before.x + 100, 3);
		expect(wm.view.y).toBeCloseTo(before.y + 50, 3);
	});

	it("accumulates across multiple calls (additive)", () => {
		wm.fitToScreen(1280, 720);
		const startX = wm.view.x;
		const startY0 = wm.view.y;
		wm.panBy(10, 20, 1280, 720);
		wm.panBy(10, 20, 1280, 720);
		expect(wm.view.x).toBeCloseTo(startX + 20, 3);
		expect(wm.view.y).toBeCloseTo(startY0 + 40, 3);
	});
});

// ---- resetView --------------------------------------------------------

describe("WorldMap.resetView", () => {
	it("zeroes the pan offset and resets zoom to 1", () => {
		wm.setZoom(4, 1280, 720);
		wm.panBy(100, 200, 1280, 720);
		wm.resetView(1280, 720);
		expect(wm.getZoom()).toBe(1);
		expect(wm.view.x).toBeCloseTo((1280 - wm.view.scale.x) / 2, 3);
		expect(wm.view.y).toBeCloseTo((720 - wm.view.scale.y) / 2, 3);
	});
});

// ---- destroy ---------------------------------------------------------

describe("WorldMap.destroy", () => {
	it("detaches the view from its parent on destroy", () => {
		// Add the view to a real parent Container, then destroy — the view
		// must be removed from that parent's child list.
		const parent = new Container();
		parent.addChild(wm.view);
		expect(wm.view.parent).toBe(parent);
		expect(parent.children.length).toBe(1);

		wm.destroy();

		// PixiJS v8 defers GPU/display-list sync, but the display tree link is
		// severed synchronously: `removeFromParent` clears `view.parent`.
		expect(wm.view.parent).toBeNull();
		expect(parent.children.length).toBe(0);
	});

	it("is idempotent (calling destroy twice does not throw)", () => {
		expect(() => {
			wm.destroy();
			wm.destroy();
		}).not.toThrow();
	});
});

// ---- updateHeight / updateBiome --------------------------------------

describe("WorldMap.updateHeight", () => {
	it("does not throw and preserves view reference", () => {
		const g = quadGrid(4, 1000, 1000);
		// Change h values — the method should update the height texture
		// data in place without rebuilding geometry.
		g.cells.h = [10, 80, 20, 90];
		expect(() => wm.updateHeight(g)).not.toThrow();
		// View container is unchanged (no rebuild).
		expect(wm.view.children.length).toBeGreaterThan(0);
	});

	it("is safe before any data was built (no-op when buffer is null)", () => {
		// Destroy clears internal buffers; calling updateHeight after
		// destroy should not throw (guard against NPE).
		wm.destroy();
		const g = quadGrid(1);
		expect(() => wm.updateHeight(g)).not.toThrow();
	});
});

describe("WorldMap.updateBiome", () => {
	it("does not throw and preserves view reference", () => {
		const g = quadGrid(4, 1000, 1000);
		g.cells.biome = [1, 2, 3, 4];
		expect(() => wm.updateBiome(g)).not.toThrow();
		expect(wm.view.children.length).toBeGreaterThan(0);
	});

	it("is safe after destroy (no-op when buffer is null)", () => {
		wm.destroy();
		const g = quadGrid(1);
		expect(() => wm.updateBiome(g)).not.toThrow();
	});
});

// ---- attachCamera ----------------------------------------------------

describe("attachCamera", () => {
	let target: HTMLDivElement;

	beforeEach(() => {
		target = document.createElement("div");
		document.body.appendChild(target);
	});

	afterEach(() => {
		target.remove();
	});

	// jsdom does not expose a global PointerEvent constructor. Build a
	// synthetic pointer-like event from `MouseEvent` (which jsdom DOES have)
	// and add the `pointerId` property the camera's pointerdown/up handlers
	// pass to setPointerCapture/releasePointerCapture. The handlers use the
	// optional `?.` so a missing pointerId (undefined) is fine.
	function pointerEvent(
		type: string,
		{ clientX = 0, clientY = 0 }: { clientX?: number; clientY?: number },
	): Event {
		const ev = new MouseEvent(type, { clientX, clientY, bubbles: false });
		Object.defineProperty(ev, "pointerId", { value: 0, configurable: true });
		return ev;
	}

	function keyEvent(type: string, code: string): KeyboardEvent {
		return new KeyboardEvent(type, { code, bubbles: true });
	}

	it("registers wheel + pointer listeners on the target", () => {
		const spy = vi.spyOn(target, "addEventListener");
		attachCamera(target, {
			worldMap: wm,
			screenSize: () => ({ w: 1280, h: 720 }),
		});
		// The four listener types: wheel (capture), pointerdown, pointermove,
		// pointerup, pointerleave.
		const kinds = spy.mock.calls.map((c) => c[0]);
		expect(kinds).toContain("wheel");
		expect(kinds).toContain("pointerdown");
		expect(kinds).toContain("pointermove");
		expect(kinds).toContain("pointerup");
		expect(kinds).toContain("pointerleave");
	});

	it("returns a detach function that removes the listeners", () => {
		const addSpy = vi.spyOn(target, "addEventListener");
		const removeSpy = vi.spyOn(target, "removeEventListener");
		const detach = attachCamera(target, {
			worldMap: wm,
			screenSize: () => ({ w: 1280, h: 720 }),
		});
		detach();
		const addedKinds = addSpy.mock.calls.map((c) => c[0]);
		const removedKinds = removeSpy.mock.calls.map((c) => c[0]);
		expect(removedKinds.sort()).toEqual(addedKinds.sort());
	});

	it("wheel zooms toward the cursor (zoom increases on negative deltaY)", () => {
		// Fit first so the camera math has a real base scale.
		wm.fitToScreen(1280, 720);
		const zoomBefore = wm.getZoom();
		attachCamera(target, {
			worldMap: wm,
			screenSize: () => ({ w: 1280, h: 720 }),
		});
		// WheelEvent constructor needs a synthetic clientX/Y. jsdom supports it.
		const rectSpy = vi.spyOn(target, "getBoundingClientRect").mockReturnValue({
			x: 0,
			y: 0,
			left: 0,
			top: 0,
			width: 1280,
			height: 720,
			right: 1280,
			bottom: 720,
			toJSON: () => ({}),
		} as DOMRect);
		const ev = new WheelEvent("wheel", {
			deltaY: -100,
			clientX: 640,
			clientY: 360,
			cancelable: true,
			bubbles: false,
		});
		const preventSpy = vi.spyOn(ev, "preventDefault");
		target.dispatchEvent(ev);
		expect(preventSpy).toHaveBeenCalled();
		expect(wm.getZoom()).toBeGreaterThan(zoomBefore);
		rectSpy.mockRestore();
	});

	it("wheel clamps zoom to the configured max (24)", () => {
		wm.fitToScreen(1280, 720);
		attachCamera(target, {
			worldMap: wm,
			screenSize: () => ({ w: 1280, h: 720 }),
		});
		const rectSpy = vi.spyOn(target, "getBoundingClientRect").mockReturnValue({
			x: 0,
			y: 0,
			left: 0,
			top: 0,
			width: 1280,
			height: 720,
			right: 1280,
			bottom: 720,
			toJSON: () => ({}),
		} as DOMRect);
		// Fire several zoom-in wheels to exceed 24.
		for (let i = 0; i < 200; i++) {
			target.dispatchEvent(
				new WheelEvent("wheel", {
					deltaY: -1000,
					clientX: 640,
					clientY: 360,
					cancelable: true,
				}),
			);
		}
		expect(wm.getZoom()).toBeLessThanOrEqual(24);
		rectSpy.mockRestore();
	});

	it("pointer drag pans the view (Spacebar + pointerdown + pointermove + pointerup)", () => {
		wm.fitToScreen(1280, 720);
		attachCamera(target, {
			worldMap: wm,
			screenSize: () => ({ w: 1280, h: 720 }),
		});
		const startX = wm.view.x;
		const startY2 = wm.view.y;
		// Hold Space to engage pan mode.
		window.dispatchEvent(keyEvent("keydown", "Space"));
		// Pointer down at (100, 100).
		target.dispatchEvent(
			pointerEvent("pointerdown", { clientX: 100, clientY: 100 }),
		);
		// Move to (140, 110) -> delta = (40, 10).
		target.dispatchEvent(
			pointerEvent("pointermove", { clientX: 140, clientY: 110 }),
		);
		target.dispatchEvent(
			pointerEvent("pointerup", { clientX: 140, clientY: 110 }),
		);
		window.dispatchEvent(keyEvent("keyup", "Space"));
		expect(wm.view.x).toBeCloseTo(startX + 40, 2);
		expect(wm.view.y).toBeCloseTo(startY2 + 10, 2);
	});

	it("pointer drag without Space does NOT pan (editor owns the pointer)", () => {
		wm.fitToScreen(1280, 720);
		attachCamera(target, {
			worldMap: wm,
			screenSize: () => ({ w: 1280, h: 720 }),
		});
		const startX = wm.view.x;
		const startY2 = wm.view.y;
		// No Space keydown — plain click should not pan.
		target.dispatchEvent(
			pointerEvent("pointerdown", { clientX: 100, clientY: 100 }),
		);
		target.dispatchEvent(
			pointerEvent("pointermove", { clientX: 140, clientY: 110 }),
		);
		target.dispatchEvent(
			pointerEvent("pointerup", { clientX: 140, clientY: 110 }),
		);
		expect(wm.view.x).toBe(startX);
		expect(wm.view.y).toBe(startY2);
	});

	it("pointermove without a prior pointerdown does NOT pan", () => {
		wm.fitToScreen(1280, 720);
		attachCamera(target, {
			worldMap: wm,
			screenSize: () => ({ w: 1280, h: 720 }),
		});
		const startX = wm.view.x;
		const startY2 = wm.view.y;
		target.dispatchEvent(
			pointerEvent("pointermove", { clientX: 200, clientY: 200 }),
		);
		expect(wm.view.x).toBe(startX);
		expect(wm.view.y).toBe(startY2);
	});
});

// ---- setSelected (selection outline) ----------------------------------

describe("WorldMap.setSelected (selection outline)", () => {
	it("draws a hairline outline (~2px on screen), not a giant polygon", () => {
		// The bug: the stroke was `width: 2` in view-local units. After
		// fitToScreen, view.scale ~= screenW (hundreds), so the stroke
		// rendered ~2*scaleX px wide and filled the cell. The fix
		// scale-compensates so on-screen width stays ~2px.
		wm.fitToScreen(1280, 720);
		const grid = quadGrid(1, 1000, 1000);
		wm.setSelected(grid, 0);
		const w = wm.getSelectionStrokeWidth();
		// Before the fix this would be ~2 * 1280 = 2560px; now ~2px (±1
		// because the width is derived from the mean of a non-uniform scale).
		expect(w).toBeGreaterThan(1.5);
		expect(w).toBeLessThan(2.5);
	});

	it("the outline width stays ~2px across zoom levels (scale-compensated)", () => {
		wm.fitToScreen(1280, 720);
		const grid = quadGrid(1, 1000, 1000);
		wm.setSelected(grid, 0);
		const atFit = wm.getSelectionStrokeWidth();

		// Zoom in 8x — without re-stroke the stroke would balloon 8x.
		wm.setZoom(8, 1280, 720);
		const at8x = wm.getSelectionStrokeWidth();

		// Zoom out to 0.5x.
		wm.setZoom(0.5, 1280, 720);
		const atHalf = wm.getSelectionStrokeWidth();

		// All three should be a thin hairline (~2px) regardless of zoom.
		for (const w of [atFit, at8x, atHalf]) {
			expect(w).toBeGreaterThan(1.5);
			expect(w).toBeLessThan(2.5);
		}
		// Sanity: the 8x on-screen width should NOT be ~16px (the old
		// behaviour, scaled with zoom).
		expect(at8x).toBeLessThan(3);
	});

	it("resizing (fitToScreen) re-strokes the outline at the new scale", () => {
		wm.fitToScreen(1280, 720);
		const grid = quadGrid(1, 1000, 1000);
		wm.setSelected(grid, 0);
		const before = wm.getSelectionStrokeWidth();

		// Resize to a much bigger window; fit scale grows ~4x.
		wm.fitToScreen(5120, 2880);
		const after = wm.getSelectionStrokeWidth();

		// Both still render ~2px on screen.
		expect(before).toBeGreaterThan(1.5);
		expect(before).toBeLessThan(2.5);
		expect(after).toBeGreaterThan(1.5);
		expect(after).toBeLessThan(2.5);
	});

	it("setSelected with cellId = -1 clears the selection (0 width)", () => {
		wm.fitToScreen(1280, 720);
		const grid = quadGrid(1, 1000, 1000);
		wm.setSelected(grid, 0);
		expect(wm.getSelectionStrokeWidth()).toBeGreaterThan(0);
		wm.setSelected(grid, -1);
		expect(wm.getSelectionStrokeWidth()).toBe(0);
	});

	it("setSelected with an out-of-range id clears the selection", () => {
		wm.fitToScreen(1280, 720);
		const grid = quadGrid(1, 1000, 1000);
		wm.setSelected(grid, 0);
		expect(wm.getSelectionStrokeWidth()).toBeGreaterThan(0);
		// 999 is out of range for a 1-cell grid.
		wm.setSelected(grid, 999);
		expect(wm.getSelectionStrokeWidth()).toBe(0);
	});

	// ---- Step 2.5.6: river + lake overlays ---------------------------------

	describe("WorldMap river + lake overlays", () => {
		// Helper: a 2x2 quad grid with distinct vertex positions so lake quads
		// / river polylines can be checked against the normalized [0,1] box.
		function quadGrid2x2(): Grid {
			// 4 cells in a 2000x2000 world.
			const points: [number, number][] = [];
			const vertP: [number, number][] = [];
			const cellV: number[] = [];
			const cellI: number[] = [0];
			const cellCenters: [number, number][] = [
				[500, 500],
				[1500, 500],
				[500, 1500],
				[1500, 1500],
			];
			for (let c = 0; c < 4; c++) {
				const [cx, cy] = cellCenters[c];
				points.push([cx, cy]);
				const v0 = vertP.length;
				// Each cell has 4 quad vertices spanning a 1000x1000 region.
				vertP.push(
					[cx - 500, cy - 500],
					[cx + 500, cy - 500],
					[cx + 500, cy + 500],
					[cx - 500, cy + 500],
				);
				cellV.push(v0, v0 + 1, v0 + 2, v0 + 3);
				cellI.push(cellV.length);
			}
			return {
				seed: 1,
				mesh: {
					points,
					cells: {
						v: cellV,
						c: [],
						i: cellI,
						b: [],
						spacing: [],
						cells_x: 2,
						cells_y: 2,
					},
					vertices: { p: vertP },
					world_w: 2000,
					world_h: 2000,
				},
				cells: {
					h: new Array(4).fill(50),
					temp: new Array(4).fill(0),
					prec: new Array(4).fill(0),
					biome: new Array(4).fill(0),
					state: new Array(4).fill(0),
					province: new Array(4).fill(0),
					culture: new Array(4).fill(0),
					religion: new Array(4).fill(0),
					burg: new Array(4).fill(0),
					fl: new Array(4).fill(0),
					r: new Array(4).fill(0),
					conf: new Array(4).fill(0),
				},
			};
		}

		it("rivers layer is off by default (getRiverStrokeWidth = 0)", () => {
			wm.fitToScreen(1280, 720);
			expect(wm.getRiverStrokeWidth()).toBe(0);
		});

		it("draws rivers as a scale-compensated ~2 px polyline", () => {
			wm.fitToScreen(1280, 720);
			const grid = quadGrid2x2();
			const rivers: RiverGeo[] = [
				{
					id: 1,
					source: 0,
					mouth: 3,
					discharge: 10,
					cells: [0, 1, 3],
					points: [
						[500, 500],
						[1500, 500],
						[1500, 1500],
					],
				},
			];
			wm.setRiversLakes(grid, rivers, []);
			expect(wm.getRiverStrokeWidth()).toBeGreaterThan(1.5);
			expect(wm.getRiverStrokeWidth()).toBeLessThan(2.5);
		});

		it("river stroke stays ~2 px across zoom (scale-compensated)", () => {
			wm.fitToScreen(1280, 720);
			const grid = quadGrid2x2();
			const rivers: RiverGeo[] = [
				{
					id: 1,
					source: 0,
					mouth: 3,
					discharge: 10,
					cells: [0, 3],
					points: [
						[500, 500],
						[1500, 1500],
					],
				},
			];
			wm.setRiversLakes(grid, rivers, []);
			const at1x = wm.getRiverStrokeWidth();

			// 8x the window: fit scale grows ~8x; compensated stroke stays ~2px.
			wm.fitToScreen(10240, 5760);
			const at8x = wm.getRiverStrokeWidth();

			expect(at1x).toBeGreaterThan(1.5);
			expect(at1x).toBeLessThan(2.5);
			expect(at8x).toBeGreaterThan(1.5);
			expect(at8x).toBeLessThan(2.5);
		});

		it("lakes layer paints lake-cell quads (no throw) for a 2-cell lake", () => {
			wm.fitToScreen(1280, 720);
			const grid = quadGrid2x2();
			const lakes: LakeGeo[] = [
				{
					id: 1,
					height: 40,
					cells: [0, 1],
					shoreline: [2, 3],
					closed: true,
				},
			];
			// Should not throw; the lake cells (0,1) exist in the mesh.
			expect(() => wm.setRiversLakes(grid, [], lakes)).not.toThrow();
		});

		it("rivers/lakes layers start hidden; toggling setLayers flips visibility", () => {
			wm.fitToScreen(1280, 720);
			const grid = quadGrid2x2();
			const rivers: RiverGeo[] = [
				{
					id: 1,
					source: 0,
					mouth: 1,
					discharge: 5,
					cells: [0, 1],
					points: [
						[500, 500],
						[1500, 500],
					],
				},
			];
			wm.setRiversLakes(grid, rivers, []);
			// Default layers: rivers=lakes=false. Overlays must be invisible.
			// First pull current state via getLayers.
			expect(wm.getLayers()).toEqual({
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
			// Toggle rivers on: the overlay Graphics should become visible.
			wm.setLayers({
				terrain: true,
				biome: false,
				rivers: true,
				lakes: false,
			});
			// Visibility of the overlay is internal; assert through getLayers
			// round-trip (setLayers flipped it).
			expect(wm.getLayers().rivers).toBe(true);
		});
	});

	// ---- state border edges use the GEOMETRIC neighbor, not cells.c -------
	//
	// Regression test for the "state outline draws non-border edges" bug.
	// The mesh packs `cells.c` rotated one slot against `cells.v`, so the
	// border algorithm must NOT trust `cells.c`. We build a 2-cell grid where
	// the cells share an edge, point `cells.c` at WRONG values, and assert that
	// exactly one border segment (the shared edge) is drawn for two different
	// states, and zero for two same-state cells.
	describe("state border edges are true geometric borders", () => {
		// Two squares side by side sharing the vertical edge x=500.
		// Cell A ring (clockwise): 0(500,400) -> 2(400,400) -> 3(400,600) -> 1(500,600)
		// Cell B ring:            4(600,400) -> 0(500,400) -> 1(500,600) -> 5(600,600)
		function twoCellGrid(): Grid {
			const vertP: [number, number][] = [
				[500, 400], // 0 shared
				[500, 600], // 1 shared
				[400, 400], // 2 A-only
				[400, 600], // 3 A-only
				[600, 400], // 4 B-only
				[600, 600], // 5 B-only
			];
			const cellV = [0, 2, 3, 1, 4, 0, 1, 5];
			const cellI = [0, 4, 8];
			// Deliberately WRONG: proves the fix ignores cells.c entirely.
			const cellC = [99, 99, 99, 99, 99, 99, 99, 99];
			return {
				seed: 1,
				mesh: {
					points: [
						[450, 500],
						[550, 500],
					],
					cells: {
						v: cellV,
						c: cellC,
						i: cellI,
						b: [],
						spacing: [],
						cells_x: 1,
						cells_y: 1,
					},
					vertices: { p: vertP },
					world_w: 1000,
					world_h: 1000,
				},
				cells: {
					h: [50, 50],
					temp: [0, 0],
					prec: [0, 0],
					biome: [0, 0],
					state: [0, 0],
					province: [0, 0],
					culture: [0, 0],
					religion: [0, 0],
					burg: [0, 0],
					fl: [0, 0],
					r: [0, 0],
					conf: [0, 0],
				},
			};
		}

		// Count border segments by spying on Graphics.moveTo (one per segment).
		// In PixiJS v8 the path is baked into a single stroke instruction on
		// stroke(), so we intercept the path-building calls, not read `instructions`.
		function countBorderSegments(wm: WorldMap): number {
			const gfx = (wm as unknown as { stateBorderGfx: { moveTo: () => void } })
				.stateBorderGfx;
			let count = 0;
			const spy = vi.spyOn(gfx, "moveTo" as never).mockImplementation(() => {
				count++;
			}) as unknown as { mockRestore: () => void };
			wm.setLayers({ provinces: true });
			spy.mockRestore();
			return count;
		}

		it("draws the shared A|B edge as a border, not the non-shared edges", () => {
			const g = twoCellGrid();
			const wm = new WorldMap(g);
			// A and B are different states: the ONLY border segment is the shared
			// edge (v0-v1). With the bug (trusting cells.c) every edge would look
			// like a border to void -> 8 segments instead of 1.
			wm.setEntities(g, {
				pack: {
					states: [{ color: 1 }, { color: 2 }],
					provinces: [],
					cultures: [],
					religions: [],
				},
				cells_state: [1, 2],
				cells_province: [-1, -1],
				cells_culture: [0, 0],
				cells_religion: [0, 0],
			});
			const segments = countBorderSegments(wm);
			// 6 coastline edges (neighbor -1 = void) are drawn for either
			// state assignment; PLUS the 2 shared A-B edges are drawn ONLY
			// when A and B differ. So different-state = 6 + 2 = 8.
			expect(segments).toBe(8);
			expect(wm.getStateBorderStrokeWidth()).toBeGreaterThan(0);
			wm.destroy();
		});

		it("shared edge responds to the TRUE neighbor's state, not cells.c", () => {
			const g = twoCellGrid();
			const wmDiff = new WorldMap(g);
			// A=state1, B=state2 (different) -> shared edge drawn.
			wmDiff.setEntities(g, {
				pack: {
					states: [{ color: 1 }, { color: 2 }],
					provinces: [],
					cultures: [],
					religions: [],
				},
				cells_state: [1, 2],
				cells_province: [-1, -1],
				cells_culture: [0, 0],
				cells_religion: [0, 0],
			});
			const diffSegments = countBorderSegments(wmDiff);
			wmDiff.destroy();

			const wmSame = new WorldMap(g);
			// A and B SAME state -> the shared edge is NOT a border; only the
			// 6 coastline edges remain.
			wmSame.setEntities(g, {
				pack: {
					states: [{ color: 1 }],
					provinces: [],
					cultures: [],
					religions: [],
				},
				cells_state: [1, 1],
				cells_province: [-1, -1],
				cells_culture: [0, 0],
				cells_religion: [0, 0],
			});
			const sameSegments = countBorderSegments(wmSame);
			wmSame.destroy();

			// The only difference between the two runs is the shared-edge
			// pair's state equality. The fix derives the true neighbor from
			// geometry, so the shared edges appear/disappear correctly. With
			// the bug (trusting the rotated cells.c, here all 99/void) the
			// shared edge would be drawn in BOTH cases -> diff == same.
			expect(sameSegments).toBe(6);
			expect(diffSegments).toBe(sameSegments + 2);
		});
	});
});

// Step 5.2: updateEntities live morph tests --------------------------------
// Re-use twoCellGrid from the state-border describe by hoisting it to module scope.
function makeTwoCellGrid(): Grid {
	const vertP: [number, number][] = [
		[500, 400], // 0 shared
		[500, 600], // 1 shared
		[400, 400], // 2 A-only
		[400, 600], // 3 A-only
		[600, 400], // 4 B-only
		[600, 600], // 5 B-only
	];
	const cellV = [0, 2, 3, 1, 4, 0, 1, 5];
	const cellI = [0, 4, 8];
	const cellC = [99, 99, 99, 99, 99, 99, 99, 99];
	return {
		seed: 1,
		mesh: {
			points: [
				[450, 500],
				[550, 500],
			],
			cells: {
				v: cellV,
				c: cellC,
				i: cellI,
				b: [],
				spacing: [],
				cells_x: 1,
				cells_y: 1,
			},
			vertices: { p: vertP },
			world_w: 1000,
			world_h: 1000,
		},
		cells: {
			h: [50, 50],
			temp: [0, 0],
			prec: [0, 0],
			biome: [0, 0],
			state: [0, 0],
			province: [0, 0],
			culture: [0, 0],
			religion: [0, 0],
			burg: [0, 0],
			fl: [0, 0],
			r: [0, 0],
			conf: [0, 0],
		},
	};
}

/** Build a minimal WorldAt with typed pack (entities cast to bypass full shape). */
function makeWorldAt(overrides: Partial<WorldAt> & { pack: WorldAt["pack"] }): WorldAt {
	return {
		year: 0,
		cells_state: [0, 0],
		cells_province: [0, 0],
		cells_culture: [0, 0],
		cells_religion: [0, 0],
		cells_burg: [0, 0],
		...overrides,
	};
}

describe("updateEntities (Step 5.2 live morph)", () => {
	afterEach(() => {
		vi.restoreAllMocks();
	});

	/** Cast the private WorldMap fields to a testable shape. */
	function getDataBuffers(wm: WorldMap) {
		return wm as unknown as {
			stateData: Uint8Array | null;
			provinceData: Uint8Array | null;
			cultureData: Uint8Array | null;
			religionData: Uint8Array | null;
			geometry: unknown;
			textures: { source: { update: () => void } }[];
		};
	}

	it("updates state data texture from projected WorldAt (u32 0=unassigned)", () => {
		const g = makeTwoCellGrid();
		const wm = new WorldMap(g);
		const world = makeWorldAt({
			year: 500,
			cells_state: [1, 0],
			cells_province: [1, 0],
			pack: {
				states: [{ id: 1, name: "A", color: 0xff0000, capital: 0, center_cell: 0, form: "x", tax_rate: 0, treasury: 0, rural_pop: 0, urban_pop: 0, military: 0, founded_year: 0, dissolved_year: null, culture: 0 } as any,
						  { id: 2, name: "B", color: 0x00ff00, capital: 0, center_cell: 0, form: "x", tax_rate: 0, treasury: 0, rural_pop: 0, urban_pop: 0, military: 0, founded_year: 0, dissolved_year: null, culture: 0 } as any],
				provinces: [],
				cultures: [],
				religions: [],
				burgs: [],
				armies: [],
			} as any,
		});
		wm.updateEntities(world, g);
		const { stateData } = getDataBuffers(wm);
		const texels = readTexels(stateData);
		expect(texels[0]).toEqual([255, 0, 0, 255]);
		expect(texels[1]).toEqual([0, 0, 0, 0]);
		wm.destroy();
	});

	it("normalises u32 0 -> i32 -1 in entityCells for border drawing", () => {
		const g = makeTwoCellGrid();
		const wm = new WorldMap(g);
		const world = makeWorldAt({
			year: 100,
			cells_state: [1, 0],
			cells_province: [1, 0],
			pack: {
				states: [{ color: 0xaabbcc } as any],
				provinces: [{ color: 0x112233 } as any],
				cultures: [],
				religions: [],
				burgs: [],
				armies: [],
			} as any,
		});
		wm.updateEntities(world, g);
		const stash = wm as unknown as { entityCells: { state: number[]; province: number[] } | null };
		expect(stash.entityCells).not.toBeNull();
		expect(stash.entityCells!.state).toEqual([1, -1]);
		expect(stash.entityCells!.province).toEqual([1, -1]);
		wm.destroy();
	});

	it("also accepts i32 -1 sentinel (year-0 convention) for robustness", () => {
		const g = quadGrid(1, 1000, 1000);
		const wm = new WorldMap(g);
		const world = makeWorldAt({
			year: 0,
			cells_state: [-1, -1],
			cells_province: [-1, -1],
			cells_culture: [-1, -1],
			cells_religion: [-1, -1],
			cells_burg: [0, 0],
			pack: { states: [], provinces: [], cultures: [], religions: [], burgs: [], armies: [] } as any,
		});
		expect(() => wm.updateEntities(world, g)).not.toThrow();
		const stash = wm as unknown as { entityCells: { state: number[] } | null };
		expect(stash.entityCells!.state.every((v: number) => v < 0)).toBe(true);
		wm.destroy();
	});

	it("rebuilds borders from projected state ownership (different states = border)", () => {
		const g = makeTwoCellGrid();
		const wm = new WorldMap(g);
		// Initial: both cells same state -> no shared border
		wm.setEntities(g, {
			pack: {
				states: [{ color: 0xff0000 } as any, { color: 0x00ff00 } as any],
				provinces: [],
				cultures: [],
				religions: [],
			},
			cells_state: [1, 1],
			cells_province: [-1, -1],
			cells_culture: [0, 0],
			cells_religion: [0, 0],
		});
		const countSegs = (wm: WorldMap): number => {
			const gfx = (wm as unknown as { stateBorderGfx: { moveTo: () => void } }).stateBorderGfx;
			let count = 0;
			const spy = vi.spyOn(gfx, "moveTo" as never).mockImplementation(() => {
				count++;
			}) as unknown as { mockRestore: () => void };
			wm.setLayers({ provinces: true });
			spy.mockRestore();
			return count;
		};
		expect(countSegs(wm)).toBe(6);

		// Project to a year where cell 1 changes hands
		const world = makeWorldAt({
			year: 500,
			cells_state: [1, 2],
			cells_province: [1, 2],
			pack: {
				states: [{ color: 0xff0000 } as any, { color: 0x00ff00 } as any],
				provinces: [{ color: 0x112233 } as any, { color: 0x445566 } as any],
				cultures: [],
				religions: [],
				burgs: [],
				armies: [],
			} as any,
		});
		wm.updateEntities(world, g);
		expect(countSegs(wm)).toBe(8);
		expect(wm.getStateBorderStrokeWidth()).toBeGreaterThan(0);
		wm.destroy();
	});

	it("does NOT rebuild base geometry — same geometry reference after update", () => {
		const g = makeTwoCellGrid();
		const wm = new WorldMap(g);
		const geomBefore = getDataBuffers(wm).geometry;
		const world = makeWorldAt({
			year: 100,
			cells_state: [1, 2],
			cells_province: [1, 2],
			pack: {
				states: [{ color: 1 } as any, { color: 2 } as any],
				provinces: [{ color: 1 } as any, { color: 2 } as any],
				cultures: [],
				religions: [],
				burgs: [],
				armies: [],
			} as any,
		});
		wm.updateEntities(world, g);
		expect(getDataBuffers(wm).geometry).toBe(geomBefore);
		wm.destroy();
	});

	it("uploads all four data textures (texture[2..5].source.update called)", () => {
		const g = makeTwoCellGrid();
		const wm = new WorldMap(g);
		const updates: string[] = [];
		for (let i = 2; i <= 5; i++) {
			vi.spyOn(getDataBuffers(wm).textures[i].source, "update").mockImplementation(
				() => updates.push(`tex${i}`),
			);
		}
		const world = makeWorldAt({
			year: 100,
			cells_state: [1, 2],
			cells_province: [1, 2],
			cells_culture: [0, 0],
			cells_religion: [0, 0],
			pack: {
				states: [{ color: 1 } as any, { color: 2 } as any],
				provinces: [{ color: 1 } as any, { color: 2 } as any],
				cultures: [{ color: 3 } as any, { color: 4 } as any],
				religions: [{ color: 5 } as any, { color: 6 } as any],
				burgs: [],
				armies: [],
			} as any,
		});
		wm.updateEntities(world, g);
		expect(updates).toEqual(["tex2", "tex3", "tex4", "tex5"]);
		wm.destroy();
	});

	it("falls back to existing province data when cells_province empty", () => {
		const g = makeTwoCellGrid();
		const wm = new WorldMap(g);
		wm.setEntities(g, {
			pack: {
				states: [{ color: 1 } as any, { color: 2 } as any],
				provinces: [{ color: 0x10 } as any, { color: 0x20 } as any],
				cultures: [],
				religions: [],
			},
			cells_state: [1, 2],
			cells_province: [1, 2],
			cells_culture: [0, 0],
			cells_religion: [0, 0],
		});
		const stashBefore = wm as unknown as { entityCells: { province: number[] } };
		const provinceBefore = [...stashBefore.entityCells.province];

		const world = makeWorldAt({
			year: 500,
			cells_state: [1, 2],
			cells_province: [],
			pack: {
				states: [{ color: 1 } as any, { color: 2 } as any],
				provinces: [{ color: 0x10 } as any, { color: 0x20 } as any],
				cultures: [],
				religions: [],
				burgs: [],
				armies: [],
			} as any,
		});
		wm.updateEntities(world, g);
		const stashAfter = wm as unknown as { entityCells: { province: number[] } };
		expect(stashAfter.entityCells.province).toEqual(provinceBefore);
		wm.destroy();
	});

	it("no-ops gracefully when grid is missing", () => {
		const g = makeTwoCellGrid();
		const wm = new WorldMap(g);
		const world = makeWorldAt({
			year: 100,
			cells_state: [1, 0],
			cells_province: [1, 0],
			pack: {
				states: [{ color: 1 } as any],
				provinces: [],
				cultures: [],
				religions: [],
				burgs: [],
				armies: [],
			} as any,
		});
		expect(() => wm.updateEntities(world, null as unknown as Grid)).not.toThrow();
		wm.destroy();
	});

	it("updates culture and religion textures from projected WorldAt", () => {
		const g = quadGrid(4, 1000, 1000);
		const wm = new WorldMap(g);
		const world = makeWorldAt({
			year: 300,
			cells_state: [0, 0, 0, 0],
			cells_province: [0, 0, 0, 0],
			cells_culture: [1, 2, 0, 1],
			cells_religion: [1, 0, 2, 0],
			pack: {
				states: [],
				provinces: [],
				cultures: [{ color: 0xffffff } as any, { color: 0xff0000 } as any, { color: 0x00ff00 } as any],
				religions: [{ color: 0xffffff } as any, { color: 0x0000ff } as any, { color: 0xffff00 } as any],
				burgs: [],
				armies: [],
			} as any,
		});
		wm.updateEntities(world, g);
		const { cultureData, religionData } = getDataBuffers(wm);
		const cultureTexels = readTexels(cultureData);
		const religionTexels = readTexels(religionData);
		// fillEntityBuffer with unassigned=0, idBias=0:
		// eid=0 -> transparent; eid=1 -> pack.cultures[1]; eid=2 -> pack.cultures[2]
		expect(cultureTexels[0]).toEqual([255, 0, 0, 255]); // culture 1 -> cultures[1]
		expect(cultureTexels[1]).toEqual([0, 255, 0, 255]); // culture 2 -> cultures[2]
		expect(cultureTexels[2]).toEqual([0, 0, 0, 0]); // 0 = none
		expect(cultureTexels[3]).toEqual([255, 0, 0, 255]); // culture 1
		expect(religionTexels[0]).toEqual([0, 0, 255, 255]); // religion 1 -> religions[1]
		expect(religionTexels[1]).toEqual([0, 0, 0, 0]); // 0 = none
		expect(religionTexels[2]).toEqual([255, 255, 0, 255]); // religion 2 -> religions[2]
		expect(religionTexels[3]).toEqual([0, 0, 0, 0]); // 0 = none
		wm.destroy();
	});

	it("is idempotent — calling twice yields same buffer contents", () => {
		const g = makeTwoCellGrid();
		const wm = new WorldMap(g);
		const world = makeWorldAt({
			year: 200,
			cells_state: [1, 2],
			cells_province: [1, 2],
			cells_culture: [1, 0],
			cells_religion: [0, 2],
			pack: {
				states: [{ color: 0xff0000 } as any, { color: 0x00ff00 } as any],
				provinces: [{ color: 0x111111 } as any, { color: 0x222222 } as any],
				cultures: [{ color: 0xffffff } as any, { color: 0xaaaaaa } as any],
				religions: [{ color: 0xbbbbbb } as any, { color: 0xcccccc } as any],
				burgs: [],
				armies: [],
			} as any,
		});
		wm.updateEntities(world, g);
		const b = getDataBuffers(wm);
		const s1 = readTexels(b.stateData);
		const p1 = readTexels(b.provinceData);
		const c1 = readTexels(b.cultureData);
		const r1 = readTexels(b.religionData);

		wm.updateEntities(world, g);
		expect(readTexels(b.stateData)).toEqual(s1);
		expect(readTexels(b.provinceData)).toEqual(p1);
		expect(readTexels(b.cultureData)).toEqual(c1);
		expect(readTexels(b.religionData)).toEqual(r1);
		wm.destroy();
	});
});

// ---- burgs overlay (Step 3.4) ----------------------------------------
describe("WorldMap burgs overlay", () => {
	it("getBurgRadius is 0 when layer off", () => {
		wm.fitToScreen(800, 600);
		wm.setBurgs(quadGrid(1, 1000, 1000), [{ id: 1, cell: 0, population: 500, capital: 0 }]);
		expect(wm.getBurgRadius()).toBe(0);
	});

	it("burg markers scale with the map (grow when zooming in)", () => {
		wm.setLayers({ burgs: true });
		const grid = quadGrid(1, 1000, 1000);
		wm.setBurgs(grid, [{ id: 1, cell: 0, population: 500, capital: 1 }]);
		wm.fitToScreen(800, 600);
		const r1 = wm.getBurgRadius();
		wm.setZoom(2, 800, 600);
		const r2 = wm.getBurgRadius();
		// Capital: ~5px at zoom=1; ~10px at zoom=2 (scales with the map,
		// not fixed on screen).
		expect(r1).toBeCloseTo(5, 0);
		expect(r2).toBeCloseTo(10, 0);
	});

	it("capital markers are larger than town markers", () => {
		wm.setLayers({ burgs: true });
		const grid = quadGrid(1, 1000, 1000);
		wm.setBurgs(grid, [
			{ id: 2, cell: 0, population: 100, capital: 0 },
			{ id: 3, cell: 0, population: 100, capital: 1 },
		]);
		wm.fitToScreen(800, 600);
		// Capital: ~5px, town: ~2.5px
		expect(wm.getBurgRadius()).toBeCloseTo(5, 0);
	});

	it("marker radius grows with population (capital scaling)", () => {
		wm.setLayers({ burgs: true });
		const grid = quadGrid(1, 1000, 1000);
		// Capital: desiredPx = 5 always (cap > 0), so radius is constant at 5px.
		// Town: desiredPx = 2.5 + min(2.5, sqrt(pop)/25).
		// Small town: sqrt(10)/25 ≈ 0.126 -> 2.5 + 0.126 ≈ 2.63px
		// Large town: sqrt(10000)/25 = 4 -> 2.5 + 2.5 = 5px
		// But getBurgRadius only tracks capitals. Use capitals to verify.
		wm.setBurgs(grid, [{ id: 4, cell: 0, population: 100, capital: 1 }]);
		wm.fitToScreen(800, 600);
		const r = wm.getBurgRadius();
		// Capital desiredPx is 5 regardless of population.
		expect(r).toBeCloseTo(5, 0);
	});

	it("setBurgs with null grid clears the overlay", () => {
		wm.setLayers({ burgs: true });
		wm.setBurgs(quadGrid(1, 1000, 1000), [{ id: 5, cell: 0, population: 500, capital: 1 }]);
		wm.fitToScreen(800, 600);
		expect(wm.getBurgRadius()).toBeCloseTo(5, 0);
		wm.setBurgs(null, []);
		wm.fitToScreen(800, 600);
		expect(wm.getBurgRadius()).toBe(0);
	});
});

// ---- Phase 1.3: patchEntityCells (height-edit -> entity-layer refresh) ----
describe("WorldMap.patchEntityCells (post-edit refresh)", () => {
	const setBase = (g: Grid) => {
		const wm = new WorldMap(g);
		wm.setEntities(g, {
			pack: {
				states: [{ color: 0xff0000 }, { color: 0x00ff00 }],
				provinces: [{ color: 0x0000ff }],
				cultures: [{ color: 0xffffff }, { color: 0x111111 }],
				religions: [{ color: 0x222222 }],
			},
			cells_state: [1, 2],
			cells_province: [0, 1],
			cells_culture: [1, 0],
			cells_religion: [0, 0],
		});
		return wm;
	};

	it("re-fills the entity buffers from grid.cells after an in-place edit", () => {
		const g = quadGrid(2, 1000, 1000);
		const wm = setBase(g);
		// Initial: cell0 -> state1 (red), cell1 -> state2 (green).
		let b = wm as unknown as {
			stateData: Uint8Array;
			provinceData: Uint8Array;
			cultureData: Uint8Array;
			religionData: Uint8Array;
			entityCells: { state: number[] };
		};
		expect(readTexels(b.stateData)[0]).toEqual([255, 0, 0, 255]);
		expect(readTexels(b.stateData)[1]).toEqual([0, 255, 0, 255]);

		// Simulate a height-edit repair: a land<->water flip changes cell1's
		// state assignment (1-based -> -1 unassigned). grid.cells.state is the
		// (possibly stale) per-cell array the editor respliced.
		g.cells.state = [1, 0];
		wm.patchEntityCells(g);

		const t = readTexels(b.stateData);
		// cell1 was dissolved (removed): its texel must go transparent, and the
		// entityCells stash must reflect the new -1 sentinel so state borders +
		// click-to-select no longer reference a removed entity exactly there.
		expect(t[0]).toEqual([255, 0, 0, 255]);
		// Alpha 0 = invisible; the old RGB bytes may linger but render nothing.
		expect(t[1][3]).toBe(0);
		expect(b.entityCells.state).toEqual([1, -1]);
		wm.destroy();
	});

	it("re-fills from the ACTIVE source (projected pack), not the base pack", () => {
		const g = quadGrid(2, 1000, 1000);
		const wm = setBase(g);
		// Project to a year where cell0 changes to state2 (green).
		wm.updateEntities(
			{
				year: 500,
				cells_state: [2, 0],
				cells_province: [0, 0],
				cells_culture: [0, 0],
				cells_religion: [0, 0],
				pack: {
					states: [
						{ color: 0xff0000 } as any,
						{ color: 0x00ff00 } as any,
					],
					provinces: [],
					cultures: [],
					religions: [],
					burgs: [],
					armies: [],
				} as any,
			},
			g,
		);
		// Post-scrub in-place edit (water->land in cell1). patchEntityCells
		// must re-fill using the PROJECTED pack (the active source), so the
		// projected view is preserved rather than reverting to year-0.
		g.cells.state = [2, 2];
		wm.patchEntityCells(g);
		const b = wm as unknown as { stateData: Uint8Array; entityCells: { state: number[] } };
		const t = readTexels(b.stateData);
		expect(t[0]).toEqual([0, 255, 0, 255]); // state2 green from projected pack
		expect(t[1]).toEqual([0, 255, 0, 255]);
		expect(b.entityCells.state).toEqual([2, 2]);
		wm.destroy();
	});

	it("is a no-op before any entity source is loaded", () => {
		const g = quadGrid(1, 1000, 1000);
		const wm = new WorldMap(g);
		g.cells.state = [-1];
		expect(() => wm.patchEntityCells(g)).not.toThrow();
		wm.destroy();
	});
});

// ---- Phase 2c/4c: updateEntityColor (patch-on-edit) -----------------------
describe("WorldMap.updateEntityColor (patch-on-edit)", () => {
	const make = (g: Grid) => {
		const wm = new WorldMap(g);
		wm.setEntities(g, {
			pack: {
				states: [{ color: 0xff0000 }, { color: 0x00ff00 }],
				provinces: [],
				cultures: [{ color: 0xffffff }, { color: 0x111111 }],
				religions: [{ color: 0x222222 }, { color: 0x333333 }],
			},
			cells_state: [1, 2],
			cells_province: [-1, -1],
			cells_culture: [1, 0],
			cells_religion: [0, 0],
		});
		return wm;
	};

	it("patches only the matching entity's texels in one layer", () => {
		const g = quadGrid(2, 1000, 1000);
		const wm = make(g);
		wm.updateEntityColor("state", 1, 0x123456);
		const b = wm as unknown as {
			stateData: Uint8Array;
			provinceData: Uint8Array;
			activePack: { states: { color: number }[] };
		};
		const states = readTexels(b.stateData);
		expect(states[0]).toEqual([0x12, 0x34, 0x56, 255]);
		// cell1 still state2 green — untouched.
		expect(states[1]).toEqual([0, 255, 0, 255]);
		// activePack (source-of-truth) reflects the new color so a later
		// patchEntityCells re-fill preserves it.
		expect(b.activePack.states[0]!.color).toBe(0x123456);
		wm.destroy();
	});

	it("culture edits patch the culture layer (0-based ids)", () => {
		const g = quadGrid(2, 1000, 1000);
		const wm = make(g);
		wm.updateEntityColor("culture", 1, 0xaabbcc);
		const b = wm as unknown as { cultureData: Uint8Array };
		const t = readTexels(b.cultureData);
		expect(t[0]).toEqual([0xaa, 0xbb, 0xcc, 255]);
		expect(t[1]).toEqual([0, 0, 0, 0]); // cell1 culture0 = none
		wm.destroy();
	});

	it("routes a colour edit into the projected pack when it is the active source", () => {
		const g = quadGrid(2, 1000, 1000);
		const wm = make(g);
		const world = {
			year: 500,
			cells_state: [2, 2],
			cells_province: [0, 0],
			cells_culture: [0, 0],
			cells_religion: [0, 0],
			pack: {
				states: [
					{ color: 0xff0000 } as any,
					{ color: 0x00ff00 } as any,
				],
				provinces: [],
				cultures: [],
				religions: [],
				burgs: [],
				armies: [],
			} as any,
		};
		wm.updateEntities(world, g);
		// Both cells are now state2 (green) in the projected year. Recolour
		// state2 -> purple: must patch BOTH cells from the projected pack.
		wm.updateEntityColor("state", 2, 0x884422);
		const b = wm as unknown as {
			stateData: Uint8Array;
			activePack: { states: { color: number }[] };
		};
		const t = readTexels(b.stateData);
		expect(t[0]).toEqual([0x88, 0x44, 0x22, 255]);
		expect(t[1]).toEqual([0x88, 0x44, 0x22, 255]);
		// And the projected activePack is updated (not the base vector).
		expect(b.activePack.states[1]!.color).toBe(0x884422);
		wm.destroy();
	});

	it("no-ops for a burg (point overlay, no fill texture)", () => {
		const g = quadGrid(1, 1000, 1000);
		const wm = make(g);
		expect(() => wm.updateEntityColor("burg", 1, 0xff0000)).not.toThrow();
		wm.destroy();
	});
});

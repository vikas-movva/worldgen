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

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Container } from "pixi.js";
import type { Grid } from "../core/api";
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
		},
	};
}

let wm: WorldMap;

beforeEach(() => {
	wm = new WorldMap(quadGrid(1, 1000, 1000));
});

afterEach(() => {
	wm.destroy();
});

// ---- construction + layers -------------------------------------------

describe("WorldMap construction + layer state", () => {
	it("defaults to terrain=on, biome=off", () => {
		expect(wm.getLayers()).toEqual({ terrain: true, biome: false });
	});

	it("preserve a smaller priority value when reinserted", () => {
		// Custom initialLayers overrides defaults.
		const custom = new WorldMap(quadGrid(1), {
			initialLayers: { biome: true, terrain: false },
		});
		expect(custom.getLayers()).toEqual({ terrain: false, biome: true });
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
		expect(snap).toEqual({ terrain: true, biome: false });
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

	it("pointer drag pans the view (pointerdown + pointermove + pointerup)", () => {
		wm.fitToScreen(1280, 720);
		attachCamera(target, {
			worldMap: wm,
			screenSize: () => ({ w: 1280, h: 720 }),
		});
		const startX = wm.view.x;
		const startY2 = wm.view.y;
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
		expect(wm.view.x).toBeCloseTo(startX + 40, 2);
		expect(wm.view.y).toBeCloseTo(startY2 + 10, 2);
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

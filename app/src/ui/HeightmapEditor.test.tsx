// Step 2.5.5 unit tests — HeightmapEditor component + tool mapping logic.
//
// These exercise the editor's static configuration (TOOL_TO_MODE, BRUSH_TOOLS),
// its render-time behavior (tool palette, brush controls visibility, Reset
// button), the Reset handler's entity-clearing logic, and the spacebar
// pan-suppression — all WITHOUT a real Web Worker or WASM, using the same
// FakeWorker harness as CellInspector.test.tsx and api.test.ts.
//
// Coverage:
//   - TOOL_TO_MODE: every EditorTool maps to the expected EditMode.
//   - BRUSH_TOOLS: contains exactly the 4 brush tools, excludes macro/select.
//   - Render: null when no grid; tool palette groups render all 12 tools;
//     brush controls (radius + strength sliders) visible only for brush tools.
//   - Tool selection: clicking a tool button calls store.setEditorTool.
//   - Reset: clears entity fields (state/province/culture/religion to -1,
//     burg to 0) when a HeightmapPatch is returned; resets selectedCellId to -1.
//   - Spacebar: Space keydown is preventDefault'd and suppresses pointerdown
//     painting (no edit_heightmap message posted while Space is held).
//   - New-mesh guard: stale lastEditGrid from a prior mesh is cleared when a
//     new grid with a different mesh arrives.

import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
	__setWorkerForTest,
	type Grid,
	type HeightmapPatch,
} from "../core/api";
import type { WorldMap } from "../render/layers";
import { useHeightmapEditor } from "../state/heightmapEditorStore";
import type { EditorTool } from "../state/worldgenStore";
import { useWorldgenStore } from "../state/worldgenStore";
import { BRUSH_TOOLS, HeightmapEditor, TOOL_TO_MODE } from "./HeightmapEditor";

// ---- fake worker harness -------------------------------------------------

type AnyReq = { kind: string; reqId: number; [k: string]: unknown };

class FakeWorker {
	public lastMessage: AnyReq | null = null;
	public onmessage: ((e: MessageEvent) => void) | null = null;
	public messages: AnyReq[] = [];

	postMessage(msg: AnyReq) {
		this.messages.push(msg);
		this.lastMessage = msg;
	}

	reply(result: unknown, kind?: string) {
		const req = this.lastMessage!;
		const evt = {
			data: { kind: kind ?? req.kind, reqId: req.reqId, ok: true, result },
		} as unknown as MessageEvent;
		this.onmessage?.(evt);
	}

	replyTo(req: AnyReq, result: unknown) {
		this.lastMessage = req;
		this.reply(result);
	}

	replyError(msg: string) {
		const req = this.lastMessage!;
		const evt = {
			// The api.ts Res<T> type uses `message` (not `error`) for failures.
			data: { kind: req.kind, reqId: req.reqId, ok: false, message: msg },
		} as unknown as MessageEvent;
		this.onmessage?.(evt);
	}
}

let fake: FakeWorker;

// ---- fake WorldMap -------------------------------------------------------

function makeFakeWorldMap(zoom = 2): WorldMap {
	return {
		view: { x: 0, y: 0, scale: { x: 1, y: 1 } },
		getZoom: () => zoom,
		screenToWorld: (sx: number, sy: number) => ({ x: sx, y: sy }),
		setSelected: vi.fn(),
	} as unknown as WorldMap;
}

// ---- fixtures -------------------------------------------------------------

function fakeGrid(n: number, seed: number): Grid {
	return {
		seed,
		mesh: {
			points: Array.from({ length: n }, (_, i) => [i, 0] as [number, number]),
			cells: {
				v: [],
				c: [],
				i: [],
				b: [],
				spacing: [],
				cells_x: 0,
				cells_y: 0,
			},
			vertices: { p: [] },
			world_w: 10_000,
			world_h: 8_000,
		},
		cells: {
			h: new Array(n).fill(50),
			temp: new Array(n).fill(20),
			prec: new Array(n).fill(50),
			biome: new Array(n).fill(4),
			state: new Array(n).fill(2),
			province: new Array(n).fill(5),
			culture: new Array(n).fill(3),
			religion: new Array(n).fill(1),
			burg: new Array(n).fill(7),
			fl: new Array(n).fill(0),
			r: new Array(n).fill(0),
			conf: new Array(n).fill(0),
		},
	};
}

beforeEach(() => {
	fake = new FakeWorker();
	__setWorkerForTest(fake as unknown as Worker);
	useWorldgenStore.setState({
		grid: null,
		mesh: null,
		climate: null,
		generation: null,
		layerEnabled: { terrain: true, biome: false, rivers: false, lakes: false },
		editorTool: "raise",
		brushRadius: 30,
		brushStrength: 0.5,
		selectedCellId: -1,
	});
	useHeightmapEditor.setState({
		recomputePending: false,
		lastDependentResult: null,
		lastError: null,
	});
});

afterEach(() => {
	__setWorkerForTest(null);
	act(() => {
		useHeightmapEditor.getState().clearPending();
	});
});

// ---- render helper --------------------------------------------------------

function renderEditor(
	worldMap: WorldMap | null = makeFakeWorldMap(),
	canvasEl: HTMLCanvasElement | null = document.createElement("canvas"),
): { container: HTMLElement; unmount: () => void } {
	const container = document.createElement("div");
	document.body.appendChild(container);
	const root = createRoot(container);
	act(() => {
		root.render(<HeightmapEditor worldMap={worldMap} canvasEl={canvasEl} />);
	});
	return {
		container,
		unmount: () => {
			act(() => {
				root.unmount();
			});
			container.remove();
		},
	};
}

function setGrid(grid: Grid | null) {
	act(() => useWorldgenStore.setState({ grid }));
}

// ---- TOOL_TO_MODE mapping tests ------------------------------------------

describe("TOOL_TO_MODE", () => {
	const allTools: EditorTool[] = [
		"raise",
		"lower",
		"flatten",
		"smooth",
		"range",
		"trough",
		"strait",
		"mask",
		"invert",
		"add",
		"multiply",
		"select",
	];

	it("has an entry for every EditorTool", () => {
		for (const tool of allTools) {
			expect(TOOL_TO_MODE[tool]).toBeDefined();
		}
		expect(Object.keys(TOOL_TO_MODE)).toHaveLength(allTools.length);
	});

	it("maps brush tools to their capitalized EditMode names", () => {
		expect(TOOL_TO_MODE.raise).toBe("Raise");
		expect(TOOL_TO_MODE.lower).toBe("Lower");
		expect(TOOL_TO_MODE.flatten).toBe("Flatten");
		expect(TOOL_TO_MODE.smooth).toBe("Smooth");
	});

	it("maps macro tools to their capitalized EditMode names", () => {
		expect(TOOL_TO_MODE.range).toBe("Range");
		expect(TOOL_TO_MODE.trough).toBe("Trough");
		expect(TOOL_TO_MODE.strait).toBe("Strait");
		expect(TOOL_TO_MODE.mask).toBe("Mask");
		expect(TOOL_TO_MODE.invert).toBe("Invert");
		expect(TOOL_TO_MODE.add).toBe("Add");
		expect(TOOL_TO_MODE.multiply).toBe("Multiply");
	});

	it("maps 'select' to 'Raise' (placeholder — select is handled differently)", () => {
		// The select tool doesn't edit the heightmap; TOOL_TO_MODE gives it
		// a dummy EditMode that's never actually sent to the worker.
		expect(TOOL_TO_MODE.select).toBe("Raise");
	});
});

// ---- BRUSH_TOOLS set tests ------------------------------------------------

describe("BRUSH_TOOLS", () => {
	it("contains exactly the 4 brush tools", () => {
		expect(BRUSH_TOOLS.size).toBe(4);
		expect(BRUSH_TOOLS.has("raise")).toBe(true);
		expect(BRUSH_TOOLS.has("lower")).toBe(true);
		expect(BRUSH_TOOLS.has("flatten")).toBe(true);
		expect(BRUSH_TOOLS.has("smooth")).toBe(true);
	});

	it("does not contain macro tools", () => {
		expect(BRUSH_TOOLS.has("range")).toBe(false);
		expect(BRUSH_TOOLS.has("trough")).toBe(false);
		expect(BRUSH_TOOLS.has("strait")).toBe(false);
		expect(BRUSH_TOOLS.has("mask")).toBe(false);
		expect(BRUSH_TOOLS.has("invert")).toBe(false);
		expect(BRUSH_TOOLS.has("add")).toBe(false);
		expect(BRUSH_TOOLS.has("multiply")).toBe(false);
	});

	it("does not contain 'select'", () => {
		expect(BRUSH_TOOLS.has("select")).toBe(false);
	});
});

// ---- component render tests ----------------------------------------------

describe("HeightmapEditor render", () => {
	it("renders nothing when grid is null", () => {
		setGrid(null);
		const { container, unmount } = renderEditor();
		expect(
			container.querySelector("[data-testid='heightmap-editor']"),
		).toBeNull();
		unmount();
	});

	it("renders the editor container when a grid is loaded", () => {
		setGrid(fakeGrid(100, 42));
		const { container, unmount } = renderEditor();
		expect(
			container.querySelector("[data-testid='heightmap-editor']"),
		).toBeTruthy();
		unmount();
	});

	it("renders all three tool group labels", () => {
		setGrid(fakeGrid(100, 42));
		const { container, unmount } = renderEditor();
		const text = container.textContent ?? "";
		expect(text).toMatch(/Brush/);
		expect(text).toMatch(/Macro/);
		expect(text).toMatch(/Inspect/);
		unmount();
	});

	it("renders 12 tool buttons total", () => {
		setGrid(fakeGrid(100, 42));
		const { container, unmount } = renderEditor();
		const buttons = container.querySelectorAll("button[aria-pressed]");
		expect(buttons.length).toBe(12);
		unmount();
	});

	it("shows brush controls (radius + strength sliders) when a brush tool is active", () => {
		setGrid(fakeGrid(100, 42));
		// default editorTool is "raise" (a brush tool)
		const { container, unmount } = renderEditor();
		const rangeInputs = container.querySelectorAll('input[type="range"]');
		// 2 range inputs: radius + strength
		expect(rangeInputs.length).toBe(2);
		unmount();
	});

	it("hides brush controls when a macro tool is active", () => {
		setGrid(fakeGrid(100, 42));
		act(() => useWorldgenStore.setState({ editorTool: "range" }));
		const { container, unmount } = renderEditor();
		const rangeInputs = container.querySelectorAll('input[type="range"]');
		// No brush radius/strength sliders for macro tools
		expect(rangeInputs.length).toBe(0);
		unmount();
	});

	it("hides brush controls when 'select' is active", () => {
		setGrid(fakeGrid(100, 42));
		act(() => useWorldgenStore.setState({ editorTool: "select" }));
		const { container, unmount } = renderEditor();
		const rangeInputs = container.querySelectorAll('input[type="range"]');
		expect(rangeInputs.length).toBe(0);
		unmount();
	});

	it("marks the active tool button as pressed", () => {
		setGrid(fakeGrid(100, 42));
		act(() => useWorldgenStore.setState({ editorTool: "smooth" }));
		const { container, unmount } = renderEditor();
		const pressed = container.querySelector("button[aria-pressed='true']");
		expect(pressed?.textContent).toBe("smooth");
		unmount();
	});
});

// ---- tool selection tests -------------------------------------------------

describe("HeightmapEditor tool selection", () => {
	it("clicking a tool button calls setEditorTool on the store", () => {
		setGrid(fakeGrid(100, 42));
		const { container, unmount } = renderEditor();
		const buttons = container.querySelectorAll("button[aria-pressed]");
		// Find the "trough" button
		const trough = Array.from(buttons).find(
			(b) => b.textContent === "trough",
		) as HTMLButtonElement;
		expect(trough).toBeTruthy();
		act(() => {
			trough.dispatchEvent(new MouseEvent("click", { bubbles: true }));
		});
		expect(useWorldgenStore.getState().editorTool).toBe("trough");
		unmount();
	});

	it("changing from a brush tool to a macro tool hides the brush controls", () => {
		setGrid(fakeGrid(100, 42));
		// Start with "raise" (brush tool)
		const { container, unmount } = renderEditor();
		expect(container.querySelectorAll('input[type="range"]').length).toBe(2);

		// Click the "mask" macro tool
		const buttons = container.querySelectorAll("button[aria-pressed]");
		const mask = Array.from(buttons).find(
			(b) => b.textContent === "mask",
		) as HTMLButtonElement;
		act(() => {
			mask.dispatchEvent(new MouseEvent("click", { bubbles: true }));
		});
		// Brush controls should now be hidden
		expect(container.querySelectorAll('input[type="range"]').length).toBe(0);
		unmount();
	});
});

// ---- Reset button tests ---------------------------------------------------

describe("HeightmapEditor Reset", () => {
	it("clears entity fields (state/province/culture/religion to -1, burg to 0) on HeightmapPatch result", async () => {
		const grid = fakeGrid(50, 1);
		// Pre-populate entity fields with non-reset values
		grid.cells.state = new Array(50).fill(3);
		grid.cells.province = new Array(50).fill(4);
		grid.cells.culture = new Array(50).fill(2);
		grid.cells.religion = new Array(50).fill(1);
		grid.cells.burg = new Array(50).fill(8);
		setGrid(grid);
		act(() => useWorldgenStore.setState({ selectedCellId: 7 }));

		const { container, unmount } = renderEditor();

		// Click the Reset button.
		const resetBtn = Array.from(container.querySelectorAll("button")).find(
			(b) => b.textContent === "Reset Heightmap",
		) as HTMLButtonElement;
		expect(resetBtn).toBeTruthy();

		await act(async () => {
			resetBtn.dispatchEvent(new MouseEvent("click", { bubbles: true }));
		});

		// Find the reset_heightmap request and reply with a HeightmapPatch.
		const resetReq = fake.messages.find((m) => m.kind === "reset_heightmap")!;
		expect(resetReq).toBeTruthy();
		const patch: HeightmapPatch = { h: new Uint8Array(50).fill(50) };

		await act(async () => {
			fake.replyTo(resetReq, patch);
		});

		// Verify entity fields are reset in the store grid.
		const cur = useWorldgenStore.getState().grid!;
		expect(cur.cells.state.every((v) => v === -1)).toBe(true);
		expect(cur.cells.province.every((v) => v === -1)).toBe(true);
		expect(cur.cells.culture.every((v) => v === -1)).toBe(true);
		expect(cur.cells.religion.every((v) => v === -1)).toBe(true);
		expect(cur.cells.burg.every((v) => v === 0)).toBe(true);
		// Height array is replaced by the patch
		expect(cur.cells.h.every((v) => v === 50)).toBe(true);
		// selectedCellId is reset to -1
		expect(useWorldgenStore.getState().selectedCellId).toBe(-1);

		unmount();
	});

	it("shows 'Resetting...' text and disables the button while reset is in flight", async () => {
		setGrid(fakeGrid(50, 1));
		const { container, unmount } = renderEditor();

		const resetBtn = Array.from(container.querySelectorAll("button")).find(
			(b) => b.textContent === "Reset Heightmap",
		) as HTMLButtonElement;

		// Click but don't reply yet — button should show "Resetting..."
		await act(async () => {
			resetBtn.dispatchEvent(new MouseEvent("click", { bubbles: true }));
		});

		const resettingBtn = Array.from(container.querySelectorAll("button")).find(
			(b) => b.textContent === "Resetting...",
		) as HTMLButtonElement;
		expect(resettingBtn).toBeTruthy();
		expect(resettingBtn.disabled).toBe(true);

		// Reply to complete the reset
		const resetReq = fake.messages.find((m) => m.kind === "reset_heightmap")!;
		await act(async () => {
			fake.replyTo(resetReq, { h: new Uint8Array(50).fill(50) });
		});

		// Button should go back to "Reset Heightmap"
		const afterBtn = Array.from(container.querySelectorAll("button")).find(
			(b) => b.textContent === "Reset Heightmap",
		) as HTMLButtonElement;
		expect(afterBtn).toBeTruthy();
		expect(afterBtn.disabled).toBe(false);

		unmount();
	});

	it("shows an error message when reset fails", async () => {
		setGrid(fakeGrid(50, 1));
		const { container, unmount } = renderEditor();

		const resetBtn = Array.from(container.querySelectorAll("button")).find(
			(b) => b.textContent === "Reset Heightmap",
		) as HTMLButtonElement;

		await act(async () => {
			resetBtn.dispatchEvent(new MouseEvent("click", { bubbles: true }));
		});

		const resetReq = fake.messages.find((m) => m.kind === "reset_heightmap")!;
		await act(async () => {
			fake.lastMessage = resetReq;
			fake.replyError("wasm panic: reset failed");
		});

		// Status div should show the error message
		const status = container.querySelector("[data-testid='editor-status']");
		expect(status?.textContent).toMatch(/Reset error/i);
		expect(status?.textContent).toMatch(/reset failed/);

		// Button should be re-enabled (not stuck in "Resetting...")
		const afterBtn = Array.from(container.querySelectorAll("button")).find(
			(b) => b.textContent === "Reset Heightmap",
		);
		expect(afterBtn).toBeTruthy();

		unmount();
	});
});

// ---- spacebar pan-suppression tests --------------------------------------

describe("HeightmapEditor spacebar suppression", () => {
	it("calls preventDefault on Space keydown", () => {
		setGrid(fakeGrid(100, 42));
		const { unmount } = renderEditor();

		const spy = vi.spyOn(KeyboardEvent.prototype, "preventDefault");
		act(() => {
			window.dispatchEvent(new KeyboardEvent("keydown", { code: "Space" }));
		});
		expect(spy).toHaveBeenCalled();
		spy.mockRestore();
		unmount();
	});

	it("does not call preventDefault on non-Space keys", () => {
		setGrid(fakeGrid(100, 42));
		const { unmount } = renderEditor();

		const spy = vi.spyOn(KeyboardEvent.prototype, "preventDefault");
		act(() => {
			window.dispatchEvent(new KeyboardEvent("keydown", { code: "KeyA" }));
		});
		expect(spy).not.toHaveBeenCalled();
		spy.mockRestore();
		unmount();
	});

	it("suppressed pointerdown painting while Space is held (no edit_heightmap posted)", () => {
		setGrid(fakeGrid(100, 42));
		const canvas = document.createElement("canvas");
		// getBoundingClientRect: stub so pointerToCell's screen->world math works
		canvas.getBoundingClientRect = () =>
			({ left: 0, top: 0, width: 800, height: 600 }) as DOMRect;
		const worldMap = makeFakeWorldMap();
		const { unmount } = renderEditor(worldMap, canvas);

		// Hold Space
		act(() => {
			window.dispatchEvent(new KeyboardEvent("keydown", { code: "Space" }));
		});

		// Attempt a pointerdown on the canvas. jsdom may not have PointerEvent,
		// so use MouseEvent (which PointerEvent extends) with pointerId.
		const editBefore = fake.messages.filter(
			(m) => m.kind === "edit_heightmap",
		).length;
		const PointerEventCtor = (globalThis as any).PointerEvent ?? MouseEvent;
		act(() => {
			canvas.dispatchEvent(
				new PointerEventCtor("pointerdown", {
					clientX: 400,
					clientY: 300,
					pointerId: 1,
					bubbles: true,
				}) as Event,
			);
		});

		const editAfter = fake.messages.filter(
			(m) => m.kind === "edit_heightmap",
		).length;
		expect(editAfter).toBe(editBefore); // no new edit posted

		// Release Space
		act(() => {
			window.dispatchEvent(new KeyboardEvent("keyup", { code: "Space" }));
		});

		unmount();
	});

	it("cleans up keydown/keyup listeners on unmount", () => {
		setGrid(fakeGrid(100, 42));
		const { unmount } = renderEditor();
		unmount();

		// After unmount, Space should NOT be preventDefault'd (listener removed)
		const spy = vi.spyOn(KeyboardEvent.prototype, "preventDefault");
		act(() => {
			window.dispatchEvent(new KeyboardEvent("keydown", { code: "Space" }));
		});
		expect(spy).not.toHaveBeenCalled();
		spy.mockRestore();
	});
});

// ---- new-mesh guard test --------------------------------------------------

describe("HeightmapEditor new-mesh guard", () => {
	it("the component updates when a new grid with a different mesh arrives (no crash)", () => {
		setGrid(fakeGrid(100, 42));
		const { container, unmount } = renderEditor();
		expect(
			container.querySelector("[data-testid='heightmap-editor']"),
		).toBeTruthy();

		// Replace with a grid on a different mesh (different seed).
		const grid2 = fakeGrid(200, 99);
		act(() => useWorldgenStore.setState({ grid: grid2 }));

		// Editor should still be rendered (no crash from stale lastEditGrid).
		expect(
			container.querySelector("[data-testid='heightmap-editor']"),
		).toBeTruthy();

		unmount();
	});
});

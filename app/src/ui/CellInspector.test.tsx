// Step 2.5.5 unit tests — CellInspector + per-cell edit pipeline.
//
// These exercise the inspector's render readout and the single-cell height
// edit path WITHOUT a real Web Worker or WASM:
//   - the "select a cell" hint when no cell is selected.
//   - the readout (cell id / h / temp / prec / biome name) for a selected cell.
//   - the height slider fires `editHeightmap` with an `Add` op (single-cell,
//     strength = (target - current)/100) then `recomputeTempBiomeLocal` for
//     that cell, and splices the result into the store grid so the store's
//     `cells.h[id]` updates.
//   - the ±nudge buttons go through the same path.
//   - a land/water crossing (set h to below sea level) flips the Type readout.
//
// A fake `Worker` captures the `postMessage` payload; we drive `onmessage` by
// hand to simulate the worker's reply (same harness as `api.test.ts`). The
// CellInspector is rendered into a jsdom container via `react-dom/client`.

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { act } from "react";
import { createRoot } from "react-dom/client";
import {
	__setWorkerForTest,
	type EditOp,
	type Grid,
	type HeightmapPatch,
} from "../core/api";
import { useHeightmapEditor } from "../state/heightmapEditorStore";
import { useWorldgenStore } from "../state/worldgenStore";
import { CellInspector } from "./CellInspector";

// ---- fake worker harness -------------------------------------------------

type AnyReq = { kind: string; reqId: number; [k: string]: unknown };

class FakeWorker {
	public lastMessage: AnyReq | null = null;
	public onmessage: ((e: MessageEvent) => void) | null = null;

	postMessage(msg: AnyReq) {
		this.lastMessage = msg;
	}

	/** Reply to the current `lastMessage` with a success payload. */
	reply(result: unknown, kind?: string) {
		const req = this.lastMessage!;
		const evt = {
			data: { kind: kind ?? req.kind, reqId: req.reqId, ok: true, result },
		} as unknown as MessageEvent;
		this.onmessage?.(evt);
	}

	/** Reply to a specific captured request (for interleaved calls). The
	 * fake only stores the most recent `lastMessage`, so the caller saves the
	 * request to reply to and passes it here. */
	replyTo(req: AnyReq, result: unknown) {
		this.lastMessage = req;
		this.reply(result);
	}
}

let fake: FakeWorker;

beforeEach(() => {
	fake = new FakeWorker();
	__setWorkerForTest(fake as unknown as Worker);
	// Reset stores.
	useWorldgenStore.setState({
		grid: null,
		mesh: null,
		climate: null,
		generation: null,
		layerEnabled: { terrain: true, biome: false },
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
	// Cancel any pending debounced recompute so a dangling 300ms timer
	// doesn't fire a store setState in a later test. Wrap in act so the
	// clearPending setState flushes within this test's act scope.
	act(() => {
		useHeightmapEditor.getState().clearPending();
	});
});

// ---- fixture -------------------------------------------------------------

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
			biome: new Array(n).fill(4), // Grassland
			state: new Array(n).fill(-1),
			province: new Array(n).fill(-1),
			culture: new Array(n).fill(-1),
			religion: new Array(n).fill(-1),
			burg: new Array(n).fill(0),
			fl: new Array(n).fill(0),
			r: new Array(n).fill(0),
			conf: new Array(n).fill(0),
		},
	};
}

function renderInspector(): { container: HTMLElement; unmount: () => void } {
	const container = document.createElement("div");
	document.body.appendChild(container);
	const root = createRoot(container);
	act(() => {
		root.render(<CellInspector worldMap={null} />);
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

/** Simulate a range-input `change` event on the inspector height slider. */
function setSliderValue(container: HTMLElement, value: number) {
	const slider = container.querySelector(
		'[data-testid="cell-height-slider"]',
	) as HTMLInputElement;
	expect(slider).toBeTruthy();
	const setter = Object.getOwnPropertyDescriptor(
		HTMLInputElement.prototype,
		"value",
	)?.set;
	act(() => {
		setter?.call(slider, String(value));
		slider.dispatchEvent(new Event("input", { bubbles: true }));
		slider.dispatchEvent(new Event("change", { bubbles: true }));
	});
}

/** Click a nudge button by test id (e.g. `cell-height-delta-p1` = +1). */
function clickNudge(container: HTMLElement, testId: string) {
	const btn = container.querySelector(
		`[data-testid="${testId}"]`,
	) as HTMLButtonElement;
	expect(btn).toBeTruthy();
	act(() => {
		btn.dispatchEvent(new MouseEvent("click", { bubbles: true }));
	});
}

// ---- tests ---------------------------------------------------------------

describe("CellInspector readout", () => {
	it("renders the 'select a cell' hint when no cell is selected", () => {
		const grid = fakeGrid(100, 42);
		act(() => useWorldgenStore.setState({ grid, selectedCellId: -1 }));
		const { container, unmount } = renderInspector();

		const el = container.querySelector('[data-testid="cell-inspector"]');
		expect(el?.textContent).toMatch(/Select a cell/i);

		unmount();
	});

	it("renders the cell id / height / type / biome readout for a selected cell", () => {
		const grid = fakeGrid(100, 42);
		grid.cells.h[7] = 80;
		grid.cells.temp[7] = 12;
		grid.cells.prec[7] = 33;
		grid.cells.biome[7] = 6; // Temperate deciduous forest
		act(() => useWorldgenStore.setState({ grid, selectedCellId: 7 }));

		const { container, unmount } = renderInspector();
		const text = container.textContent ?? "";
		// The readout table renders label+value in adjacent <td>s so the
		// textContent collapses to e.g. "Cell7Height...". Match the id
		// value immediately before the "Height" label row.
		expect(text).toMatch(/Cell7Height/);
		expect(text).toMatch(/80\s*\/\s*100/);
		expect(text).toMatch(/Land/);
		expect(text).toMatch(/12°C/);
		expect(text).toMatch(/33\s*mm/);
		expect(text).toMatch(/6\s*·\s*Temperate deciduous forest/);

		unmount();
	});

	it("shows 'Water' when the selected cell's height is below sea level (20)", () => {
		const grid = fakeGrid(50, 1);
		grid.cells.h[3] = 10;
		grid.cells.biome[3] = 0; // Marine (matches the water height)
		act(() => useWorldgenStore.setState({ grid, selectedCellId: 3 }));
		const { container, unmount } = renderInspector();

		expect(container.textContent).toMatch(/Water/);
		expect(container.textContent).toMatch(/Marine/);

		unmount();
	});
});

describe("CellInspector per-cell height edit", () => {
	it("fires editHeightmap with an Add op (single-cell, strength = delta/100)", async () => {
		const grid = fakeGrid(100, 42);
		grid.cells.h[5] = 50;
		act(() => useWorldgenStore.setState({ grid, selectedCellId: 5 }));

		const editReq: AnyReq[] = [];
		const origPost = fake.postMessage.bind(fake);
		fake.postMessage = (msg: AnyReq) => {
			editReq.push(msg);
			origPost(msg);
		};

		const { container, unmount } = renderInspector();

		// Set height 50 -> 75 (+25): strength = 25/100 = 0.25.
		await act(async () => {
			setSliderValue(container, 75);
		});

		// First wire message should be edit_heightmap.
		const eh = editReq.find((m) => m.kind === "edit_heightmap");
		expect(eh).toBeTruthy();
		const ops = eh!.ops as EditOp[];
		expect(ops).toHaveLength(1);
		expect(ops[0].mode).toBe("Add");
		expect(ops[0].cells).toEqual([5]);
		expect(ops[0].strength).toBeCloseTo(0.25, 5);
		expect(ops[0].center_cell).toBe(5);

		unmount();
	});

	it("splices the h patch into the store grid after the edit resolves", async () => {
		const grid = fakeGrid(100, 42);
		grid.cells.h[5] = 50;
		act(() => useWorldgenStore.setState({ grid, selectedCellId: 5 }));

		// Capture the edit_heightmap request so we can reply to it, since the
		// fake only tracks the last message.
		const editReqPromise = new Promise<AnyReq>((resolveEdit) => {
			const orig = fake.postMessage.bind(fake);
			fake.postMessage = (msg: AnyReq) => {
				if (msg.kind === "edit_heightmap") resolveEdit(msg);
				orig(msg);
			};
		});

		const { container, unmount } = renderInspector();
		await act(async () => {
			setSliderValue(container, 75);
		});

		const editReq = await editReqPromise;

		// Reply to edit_heightmap with a thin HeightmapPatch { h: Uint8Array }.
		const newH = new Array(100).fill(50);
		newH[5] = 75;
		const patch: HeightmapPatch = { h: new Uint8Array(newH) };

		// Then recomputeTempBiomeLocal will fire; capture its request too.
		const localReqPromise = new Promise<AnyReq>((resolveLocal) => {
			const prev = fake.postMessage.bind(fake);
			fake.postMessage = (msg: AnyReq) => {
				if (msg.kind === "recompute_temp_biome_local") resolveLocal(msg);
				prev(msg);
			};
		});

		await act(async () => {
			fake.replyTo(editReq, patch);
		});

		// Reply to the local recompute with a single-entry temp/biome patch.
		const localReq = await localReqPromise;
		await act(async () => {
			fake.replyTo(localReq, {
				temp: new Int8Array([8]),
				biome: new Uint8Array([11]), // Glacier (high altitude)
			});
		});

		// The store grid should now reflect the spliced h + temp + biome.
		const cur = useWorldgenStore.getState().grid!;
		expect(cur.cells.h[5]).toBe(75);
		expect(cur.cells.temp[5]).toBe(8);
		expect(cur.cells.biome[5]).toBe(11);

		// A debounced recomputeDependents is scheduled (fires after 300ms).
		const depReqPromise = new Promise<AnyReq>((resolveDep) => {
			const prev = fake.postMessage.bind(fake);
			fake.postMessage = (msg: AnyReq) => {
				if (msg.kind === "recompute_dependents") resolveDep(msg);
				prev(msg);
			};
		});
		// Advance the 300ms debounce so the request is posted.
		await act(async () => {
			await new Promise((r) => setTimeout(r, 320));
		});
		// Drain the pending recompute so it doesn't leak state updates across
		// tests (the promise rejects via the store's clearPending). Wrap in
		// act so any consequent store setState flushes synchronously.
		await act(async () => {
			useHeightmapEditor.getState().clearPending();
		});
		const depReq = await depReqPromise;
		expect(depReq).toBeTruthy();

		unmount();
	});

	it("nudge +5 button raises the selected cell's height by 5 via the Add op", async () => {
		const grid = fakeGrid(100, 42);
		grid.cells.h[5] = 40;
		act(() => useWorldgenStore.setState({ grid, selectedCellId: 5 }));

		const editReq: AnyReq[] = [];
		const orig = fake.postMessage.bind(fake);
		fake.postMessage = (msg: AnyReq) => {
			editReq.push(msg);
			orig(msg);
		};

		const { container, unmount } = renderInspector();
		await act(async () => {
			clickNudge(container, "cell-height-delta-p5");
		});

		const eh = editReq.find((m) => m.kind === "edit_heightmap");
		expect(eh).toBeTruthy();
		const op = (eh!.ops as EditOp[])[0];
		expect(op.mode).toBe("Add");
		expect(op.cells).toEqual([5]);
		// 40 -> 45, delta = 5, strength = 5/100 = 0.05.
		expect(op.strength).toBeCloseTo(0.05, 5);

		unmount();
	});

	it("does not send an editHeightmap op when the value is unchanged", async () => {
		const grid = fakeGrid(100, 42);
		grid.cells.h[5] = 40;
		act(() => useWorldgenStore.setState({ grid, selectedCellId: 5 }));

		const messages: AnyReq[] = [];
		const orig = fake.postMessage.bind(fake);
		fake.postMessage = (msg: AnyReq) => {
			messages.push(msg);
			orig(msg);
		};

		const { container, unmount } = renderInspector();
		await act(async () => {
			setSliderValue(container, 40); // same as current
		});

		expect(messages.find((m) => m.kind === "edit_heightmap")).toBeUndefined();

		unmount();
	});

	it("clamps the target height to [0, 100]", async () => {
		const grid = fakeGrid(20, 1);
		grid.cells.h[1] = 95;
		act(() => useWorldgenStore.setState({ grid, selectedCellId: 1 }));

		const editReq: AnyReq[] = [];
		const orig = fake.postMessage.bind(fake);
		fake.postMessage = (msg: AnyReq) => {
			editReq.push(msg);
			orig(msg);
		};

		const { container, unmount } = renderInspector();
		// +5 nudge on a 95 cell -> the component asks for 100 (clamped from 100).
		await act(async () => {
			clickNudge(container, "cell-height-delta-p5");
		});

		const eh = editReq.find((m) => m.kind === "edit_heightmap");
		expect(eh).toBeTruthy();
		const op = (eh!.ops as EditOp[])[0];
		// 95 -> 100 (clamped), delta = 5, strength = 0.05.
		expect(op.strength).toBeCloseTo(0.05, 5);

		unmount();
	});

	it("a land→water edit (set below sea level) flips the readout Type after the store splices", async () => {
		const grid = fakeGrid(100, 42);
		grid.cells.h[5] = 50;
		grid.cells.biome[5] = 4; // Grassland
		act(() => useWorldgenStore.setState({ grid, selectedCellId: 5 }));

		const editReqPromise = new Promise<AnyReq>((resolveEdit) => {
			const orig = fake.postMessage.bind(fake);
			fake.postMessage = (msg: AnyReq) => {
				if (msg.kind === "edit_heightmap") resolveEdit(msg);
				orig(msg);
			};
		});

		const { container, unmount } = renderInspector();
		await act(async () => {
			setSliderValue(container, 10); // below sea level (20)
		});
		const editReq = await editReqPromise;

		await act(async () => {
			fake.replyTo(editReq, {
				h: new Uint8Array(new Array(100).fill(10)),
			});
		});
		// Reply to the local recompute (marine biome 0).
		await act(async () => {
			// Wait one tick for recomputeTempBiomeLocal message to register.
			await new Promise((r) => setTimeout(r, 0));
			fake.reply({
				temp: new Int8Array([18]),
				biome: new Uint8Array([0]), // Marine
			});
		});

		// Squash the pending debounced recompute so it doesn't leak across
		// tests (the store timer fires after 300ms). Wrap in act so the
		// consequent setState flushes before the assertion.
		await act(async () => {
			useHeightmapEditor.getState().clearPending();
		});

		// Re-render the inspector to pick up the spliced store grid.
		const text = container.textContent ?? "";
		// The store grid splice may not have completed the dependent step
		// (debounced), but the local h/temp/biome splice already landed, so
		// the readout reflects Water/Marine.
		expect(text).toMatch(/Water/);
		expect(text).toMatch(/Marine/);

		unmount();
	});
});

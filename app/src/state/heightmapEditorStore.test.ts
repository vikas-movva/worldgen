// Step 2.5.3 unit tests — the heightmap editor store debounce logic.
//
// Tests the `useHeightmapEditor` zustand slice:
//   T1. Initial state: not pending, no result, no error.
//   T2. `scheduleDependentRecompute` sets `recomputePending` immediately and
//       fires `recomputeDependents` after the debounce window (300ms).
//   T3. Rapid successive calls coalesce: only the latest call's promise
//       resolves; earlier ones are superseded (rejected).
//   T4. `clearPending` cancels the timer and rejects the in-flight promise.
//   T5. Worker error sets `lastError` and rejects the promise.
//   T6. `lastDependentResult` is stored after a successful recompute.
//
// Uses `vi.useFakeTimers` so the 300ms debounce is instant in tests.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DependentResult, Grid } from "../core/api";
import { coreApi } from "../core/api";
import {
	RECOMPUTE_DEBOUNCE_MS,
	useHeightmapEditor,
} from "./heightmapEditorStore";

// Mock `coreApi.recomputeDependents` — the store calls this after the debounce.
vi.mock("../core/api", () => ({
	coreApi: {
		recomputeDependents: vi.fn(),
	},
}));

const mockedRecompute = vi.mocked(coreApi.recomputeDependents);

function makeFakeGrid(n = 100): Grid {
	return {
		seed: 42,
		mesh: {
			points: Array.from({ length: n }, () => [0, 0] as [number, number]),
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
			world_w: 10000,
			world_h: 8000,
		},
		cells: {
			h: new Array(n).fill(50),
			temp: new Array(n).fill(10),
			prec: new Array(n).fill(50),
			biome: new Array(n).fill(5),
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

function fakeDependentResult(n: number): DependentResult {
	return {
		temp: new Int8Array(n),
		prec: new Uint8Array(n),
		biome: new Uint8Array(n),
		state: new Int32Array(n),
		province: new Int32Array(n),
		burg: new Int16Array(n),
		fl: new Uint16Array(n),
		r: new Uint16Array(n),
		conf: new Uint16Array(n),
		coastline: new Uint8Array(n),
		removed_burgs: [],
		dissolved_states: [],
		rivers: [],
		lakes: [],
	};
}

beforeEach(() => {
	vi.useFakeTimers();
	useHeightmapEditor.setState({
		recomputePending: false,
		lastDependentResult: null,
		lastError: null,
	});
	mockedRecompute.mockReset();
});

afterEach(() => {
	vi.useRealTimers();
});

describe("useHeightmapEditor initial state", () => {
	it("starts not pending with no result and no error", () => {
		const s = useHeightmapEditor.getState();
		expect(s.recomputePending).toBe(false);
		expect(s.lastDependentResult).toBeNull();
		expect(s.lastError).toBeNull();
	});
});

describe("scheduleDependentRecompute", () => {
	it("sets recomputePending immediately and fires recomputeDependents after debounce", async () => {
		const grid = makeFakeGrid();
		const result = fakeDependentResult(100);
		mockedRecompute.mockResolvedValue(result);

		const p = useHeightmapEditor.getState().scheduleDependentRecompute(grid);
		expect(useHeightmapEditor.getState().recomputePending).toBe(true);
		expect(mockedRecompute).not.toHaveBeenCalled();

		await vi.advanceTimersByTimeAsync(RECOMPUTE_DEBOUNCE_MS);

		const res = await p;
		expect(res).toBe(result);
		expect(useHeightmapEditor.getState().recomputePending).toBe(false);
		expect(useHeightmapEditor.getState().lastDependentResult).toBe(result);
	});

	it("coalesces rapid calls: only the latest promise resolves", async () => {
		const grid = makeFakeGrid();
		const result = fakeDependentResult(100);
		mockedRecompute.mockResolvedValue(result);

		const p1 = useHeightmapEditor.getState().scheduleDependentRecompute(grid);
		await vi.advanceTimersByTimeAsync(100);
		const p2 = useHeightmapEditor.getState().scheduleDependentRecompute(grid);

		// p1 should reject (superseded). p2 should resolve.
		await expect(p1).rejects.toThrow(/superseded/);

		await vi.advanceTimersByTimeAsync(RECOMPUTE_DEBOUNCE_MS);
		const res2 = await p2;
		expect(res2).toBe(result);
		// Only one worker call (the second).
		expect(mockedRecompute).toHaveBeenCalledTimes(1);
	});

	it("clearPending cancels the timer and rejects the in-flight promise", async () => {
		const grid = makeFakeGrid();
		const p = useHeightmapEditor.getState().scheduleDependentRecompute(grid);
		expect(useHeightmapEditor.getState().recomputePending).toBe(true);

		useHeightmapEditor.getState().clearPending();

		await expect(p).rejects.toThrow(/cleared/);
		expect(useHeightmapEditor.getState().recomputePending).toBe(false);
		// No worker call should have fired.
		expect(mockedRecompute).not.toHaveBeenCalled();
	});

	it("worker error sets lastError and rejects the promise", async () => {
		const grid = makeFakeGrid();
		mockedRecompute.mockRejectedValueOnce(new Error("wasm panic: bad grid"));

		const p = useHeightmapEditor.getState().scheduleDependentRecompute(grid);
		// Suppress unhandled rejection from the mock before we await.
		p.catch(() => {});
		await vi.advanceTimersByTimeAsync(RECOMPUTE_DEBOUNCE_MS);

		await expect(p).rejects.toThrow(/bad grid/);
		expect(useHeightmapEditor.getState().recomputePending).toBe(false);
		expect(useHeightmapEditor.getState().lastError).toBe(
			"wasm panic: bad grid",
		);
	});

	it("stores lastDependentResult after a successful recompute", async () => {
		const grid = makeFakeGrid();
		const result = fakeDependentResult(100);
		// Add a river so we can verify the full shape is preserved.
		result.rivers = [
			{
				id: 1,
				source: 5,
				mouth: 10,
				discharge: 42,
				cells: [5, 6, 7],
				points: [
					[0, 0],
					[1, 1],
				],
			},
		];
		mockedRecompute.mockResolvedValue(result);

		const p = useHeightmapEditor.getState().scheduleDependentRecompute(grid);
		await vi.advanceTimersByTimeAsync(RECOMPUTE_DEBOUNCE_MS);
		const res = await p;

		const stored = useHeightmapEditor.getState().lastDependentResult;
		expect(stored).toBe(res);
		expect(stored?.rivers.length).toBe(1);
		expect(stored?.rivers[0].id).toBe(1);
	});
});

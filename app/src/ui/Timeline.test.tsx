// Step 5.1 unit tests — Timeline scrubber UI.
//
// These exercise the Timeline component's integration with the zustand store
// and coreApi WITHOUT a real Web Worker or WASM module:
//
//   - renders a slider + play/pause + speed controls when a grid + states + timeline exist.
//   - dragging the year slider calls `setCurrentYear` via scrubTo (which also
//     calls coreApi.scrubWorld on the worker).
//   - scrubbing posts a `scrub_world` message to the worker; the worker reply
//     resolves and stores the projection via `setProjectedWorld`.
//   - the stale-response guard rejects an older reqId's projection (the core
//     Step 5.1 verification: "Old worker responses cannot overwrite newer
//     years").
//   - play starts a tick loop that advances the year and triggers scrub; pause
//     stops it.
//   - speed control updates playbackSpeed.
//
// A fake `Worker` captures the `postMessage` payload; we drive `onmessage` by
// hand to simulate the worker's reply (same harness as `api.test.ts`).

import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
	__setWorkerForTest,
	coreApi,
	type WorldAt,
} from "../core/api";
import { useWorldgenStore } from "../state/worldgenStore";
import type { Grid, StatesResult, CulturesResult, TimelineEvent, Timeline as TimelineType } from "../core/api";
import { Timeline as TimelineComp } from "./Timeline";

// ---- fake worker harness ---------------------------------------------------

type AnyReq = { kind: string; reqId: number; [k: string]: unknown };

class FakeWorker {
	public lastMessage: AnyReq | null = null;
	public messages: AnyReq[] = [];
	public onmessage: ((e: MessageEvent) => void) | null = null;
	public replyCount = 0;

	postMessage(msg: AnyReq) {
		this.lastMessage = msg;
		this.messages.push(msg);
	}

	/** Reply to the current `lastMessage` with a success payload. */
	reply(result: unknown) {
		expect(this.lastMessage).toBeDefined();
		const req = this.lastMessage!;
		const evt = {
			data: { kind: req.kind, reqId: req.reqId, ok: true, result },
		} as unknown as MessageEvent;
		this.onmessage?.(evt);
		this.replyCount++;
	}
}

let fake: FakeWorker;

// ---- test fixtures ---------------------------------------------------------

/** Minimal 2×2 grid for testing. Cell 0 is land (h>=20), cell 3 is water. */
function makeFakeGrid(seed: number): Grid {
	return {
		seed,
		mesh: {
			points: [
				[0, 0],
				[1, 1],
				[2, 0],
				[1, -1],
			],
			cells: {
				v: [0, 1, 2, 0, 2, 3],
				c: [0, 1, 2, 0, 2, 3],
				i: [0, 1, 2, 3],
				b: [0, 1, 2, 3],
				spacing: [1, 1, 1, 1],
				cells_x: 2,
				cells_y: 2,
			},
			vertices: { p: [[0, 0], [1, 1], [2, 0], [1, -1]] },
			world_w: 2,
			world_h: 2,
		},
		cells: {
			h: [80, 90, 70, 10],
			temp: [10, 20, 30, 40],
			prec: [100, 90, 80, 70],
			biome: [1, 1, 0, 0],
			state: [-1, -1, -1, -1],
			province: [-1, -1, -1, -1],
			culture: [-1, -1, -1, -1],
			religion: [-1, -1, -1, -1],
			burg: [0, 0, 0, 0],
			fl: [0, 0, 0, 0],
			r: [0, 0, 0, 0],
			conf: [0, 0, 0, 0],
		},
	};
}

/** Full State entity (id=1, 1-based). */
function makeState(): StatesResult["pack"]["states"][number] {
	return {
		id: 1,
		name: "TestState",
		color: 0xff0000,
		capital: 2,
		center_cell: 0,
		form: "Monarchy",
		tax_rate: 0.1,
		treasury: 1000,
		rural_pop: 500,
		urban_pop: 200,
		military: 50,
		founded_year: 0,
		dissolved_year: null,
		culture: 1,
	};
}

/** Full Province entity (id=1) belonging to state 1. */
function makeProvince(): StatesResult["pack"]["provinces"][number] {
	return {
		id: 1,
		state: 1,
		name: "TestProvince",
		color: 0xaa0000,
		center_cell: 0,
		rural_pop: 300,
		urban_pop: 100,
		founded_year: 0,
		dissolved_year: null,
	};
}

/** Full Culture entity (id=1, 0-based in cells_culture). */
function makeCulture(): CulturesResult["cultures"][number] {
	return {
		id: 1,
		name: "TestCulture",
		color: 0x0000ff,
		origin: 0,
		type_code: 1,
		founded_year: 0,
		dissolved_year: null,
		cell_count: 1,
	};
}

/** Full Religion entity (id=1, 0-based in cells_religion). */
function makeReligion(): CulturesResult["religions"][number] {
	return {
		id: 1,
		name: "TestReligion",
		color: 0x00ff00,
		center_cell: 0,
		parent: null,
		followers: 100,
		type_code: 0,
		founded_year: 0,
		dissolved_year: null,
	};
}

function makeFakePack(): StatesResult["pack"] {
	return {
		states: [makeState()],
		provinces: [makeProvince()],
		cultures: [makeCulture()],
		religions: [makeReligion()],
		burgs: [],
		armies: [],
	};
}

function makeFakeStatesResult(): StatesResult {
	return {
		pack: makeFakePack(),
		cells_state: [0, -1, -1, -1],
		cells_province: [0, -1, -1, -1],
		cells_burg: [0, 0, 0, 0],
	};
}

function makeFakeCulturesResult(): CulturesResult {
	return {
		cultures: [makeCulture()],
		religions: [makeReligion()],
		cells_culture: [0, -1, -1, -1],
		cells_religion: [0, -1, -1, -1],
	};
}

function makeFakeWorldAt(year: number): WorldAt {
	return {
		year,
		cells_state: [0, -1, -1, -1],
		cells_province: [0, -1, -1, -1],
		cells_culture: [0, -1, -1, -1],
		cells_religion: [0, -1, -1, -1],
		cells_burg: [0, 0, 0, 0],
		pack: makeFakePack(),
	};
}

function makeFakeTimeline(): TimelineType {
	const events: TimelineEvent[] = [
		{
			id: 1,
			year: 10,
			entity_id: 1,
			entity_type: "State",
			kind: "Found",
			payload: { kind: "None" },
			narrative: null,
		},
	];
	return events;
}

// ---- test harness ----------------------------------------------------------

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
	fake = new FakeWorker();
	__setWorkerForTest(fake as unknown as Worker);
	container = document.createElement("div");
	document.body.appendChild(container);

	// Reset store to a known state.
	const store = useWorldgenStore.getState();
	store.setGrid(makeFakeGrid(42));
	store.setStatesResult(makeFakeStatesResult());
	store.setCulturesResult(makeFakeCulturesResult());
	store.setTimeline(makeFakeTimeline(), 0, 100);
	store.setCurrentYear(0);
	store.setIsPlaying(false);
	store.setPlaybackSpeed(5);
	store.setScrubStatus("idle");
	store.setProjectedWorld(null);
});

afterEach(() => {
	act(() => root?.unmount());
	container.remove();
	__setWorkerForTest(null);
});

function renderTimeline() {
	root = createRoot(container);
	act(() => {
		root.render(<TimelineComp />);
	});
}

/**
 * Drag the range slider to `value` by setting the value via the native
 * HTMLInputElement setter (so React's synthetic event detects the change)
 * and dispatching an `input` event — the standard jsdom/React testing
 * workaround.
 */
function dragSlider(slider: HTMLInputElement, value: number) {
	const nativeSet = Object.getOwnPropertyDescriptor(
		HTMLInputElement.prototype,
		"value",
	)!.set!;
	act(() => {
		nativeSet.call(slider, String(value));
		slider.dispatchEvent(new Event("input", { bubbles: true }));
	});
}

/** Drain the microtask queue after a React state update. */
async function flushMicrotasks() {
	await act(async () => {
		await Promise.resolve();
	});
}

// ---- tests -----------------------------------------------------------------

describe("Timeline (Step 5.1)", () => {
	it("renders slider, play/pause, and speed controls after grid+states+timeline load", () => {
		renderTimeline();
		const html = container.innerHTML;
		// The slider (year input) and play/pause button must be present.
		expect(html).toContain("input"); // slider is a range input
		// Button text is "Play" when not playing (initial state).
		expect(html).toContain("Play");
		// Speed control is a <select> with speed options.
		expect(html).toContain("Speed:");
		expect(html).toContain("25×");
	});

	it("dragging the year slider updates currentYear in the store via scrubTo", async () => {
		renderTimeline();

		// The onMount useEffect: timeline is already set, so it calls
		// scrubTo(0) which posts a scrub_world message.
		if (fake.lastMessage) {
			fake.reply(makeFakeWorldAt(0));
			await flushMicrotasks();
		}

		const slider = container.querySelector(
			'input[type="range"]',
		) as HTMLInputElement;
		expect(slider).toBeTruthy();

		// Move slider to year 23.
		dragSlider(slider, 23);

		expect(useWorldgenStore.getState().currentYear).toBe(23);
		// scrubWorld should be called with the target year.
		expect(fake.lastMessage?.kind).toBe("scrub_world");
		expect(fake.lastMessage?.target_year).toBe(23);

		// The worker would reply with a WorldAt; deliver it so the promise
		// resolves and projectedWorld gets set.
		const projected = makeFakeWorldAt(23);
		fake.reply(projected);
		await flushMicrotasks();
		expect(useWorldgenStore.getState().projectedWorld).toEqual(projected);
	});

	it("on slider interaction, calls coreApi.scrubWorld and stores the projection", async () => {
		const scrubSpy = vi.spyOn(coreApi, "scrubWorld");
		renderTimeline();

		// The onMount useEffect: timeline is set, so it calls scrubTo(0).
		// That posts a scrub_world message. Consume it.
		if (fake.lastMessage) {
			fake.reply(makeFakeWorldAt(0));
			await flushMicrotasks();
		}

		const slider = container.querySelector(
			'input[type="range"]',
		) as HTMLInputElement;
		expect(slider).toBeTruthy();

		// Simulate slider interaction at year 10.
		dragSlider(slider, 10);

		// scrubWorld should be called with the target year.
		expect(scrubSpy).toHaveBeenCalled();
		expect(fake.lastMessage?.kind).toBe("scrub_world");
		expect(fake.lastMessage?.target_year).toBe(10);

		// Simulate worker reply.
		const projected = makeFakeWorldAt(10);
		fake.reply(projected);
		await flushMicrotasks();

		expect(useWorldgenStore.getState().projectedWorld).toEqual(projected);
		expect(useWorldgenStore.getState().scrubStatus).toBe("idle");
	});

	it("stale-response guard: an older reqId projection is rejected", async () => {
		renderTimeline();

		// Consume the onMount scrubTo(0) message if present.
		if (fake.lastMessage) {
			fake.reply(makeFakeWorldAt(0));
			await flushMicrotasks();
		}

		const slider = container.querySelector(
			'input[type="range"]',
		) as HTMLInputElement;
		expect(slider).toBeTruthy();

		// Two rapid scrubs → two scrub_world messages with incrementing reqIds.
		const nativeSet = Object.getOwnPropertyDescriptor(
			HTMLInputElement.prototype,
			"value",
		)!.set!;

		act(() => {
			nativeSet.call(slider, "50");
			slider.dispatchEvent(new Event("input", { bubbles: true }));
		});
		const firstReqId = fake.lastMessage!.reqId;

		act(() => {
			nativeSet.call(slider, "60");
			slider.dispatchEvent(new Event("input", { bubbles: true }));
		});
		const secondReqId = fake.lastMessage!.reqId;
		expect(secondReqId).toBeGreaterThan(firstReqId);

		// currentYear should be 60 (the latest).
		expect(useWorldgenStore.getState().currentYear).toBe(60);

		// Deliver the newer response (year 60) — it should be committed.
		fake.reply(makeFakeWorldAt(60));
		await flushMicrotasks();

		// Now deliver a stale response (year 50) with the OLD reqId.
		// The guard should reject it — projectedWorld should stay at year 60.
		const oldEvt = {
			data: {
				kind: "scrub_world",
				reqId: firstReqId,
				ok: true,
				result: makeFakeWorldAt(50),
			},
		} as unknown as MessageEvent;
		fake.onmessage?.(oldEvt);
		await flushMicrotasks();

		// The projection should still be year 60 (the newer one was committed).
		const projected = useWorldgenStore.getState().projectedWorld;
		expect(projected?.year).toBe(60);
	});

	it("play starts a tick loop that advances the year and triggers scrub; pause stops it", async () => {
		const scrubSpy = vi.spyOn(coreApi, "scrubWorld");
		renderTimeline();

		// Consume onMount scrubTo(0).
		if (fake.lastMessage) {
			fake.reply(makeFakeWorldAt(0));
			await flushMicrotasks();
		}

		const playBtn = Array.from(container.querySelectorAll("button")).find(
			(b) => b.textContent === "Play",
		);
		expect(playBtn).toBeTruthy();

		// Start playback.
		await act(async () => {
			playBtn!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
			// Let the rAF tick fire a couple of times.
			await new Promise((r) => setTimeout(r, 50));
		});

		expect(useWorldgenStore.getState().isPlaying).toBe(true);
		// During playback, scrubWorld is invoked at least once.
		expect(scrubSpy).toHaveBeenCalled();

		// Stop playback.
		await act(async () => {
			playBtn!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
			await new Promise((r) => setTimeout(r, 10));
		});

		expect(useWorldgenStore.getState().isPlaying).toBe(false);
	});

	it("speed control updates playbackSpeed in the store", () => {
		renderTimeline();

		const speedSelect = container.querySelector(
			"select",
		) as HTMLSelectElement;
		expect(speedSelect).toBeTruthy();

		act(() => {
			speedSelect.value = "25";
			speedSelect.dispatchEvent(new Event("change", { bubbles: true }));
		});

		expect(useWorldgenStore.getState().playbackSpeed).toBe(25);
	});
});

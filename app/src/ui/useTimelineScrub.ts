// Phase 5.1: shared timeline scrubbing logic.
//
// Extracted from Timeline.tsx so that both the Timeline scrubber UI and the
// EntityInspector History tab can trigger a projection to a specific year
// without duplicating the stale-response-guard / worker-envelope plumbing.
//
// `scrubTo` posts a `scrub_world` message to the worker via `coreApi.scrubWorld`,
// resolves the resulting `WorldAt`, and commits it to the store (which MapCanvas
// picks up to live-morph entity data textures).
//
// Design — single-flight + latest-wins coalescing:
//
// Playback / scrubbing can call `scrubTo` many times a frame (the rAF play loop),
// and a `scrub_world` projection takes tens of ms on the worker. Worker
// `onmessage` handlers run serially in an event queue, so firing one request per
// call would pile stale projections onto the worker's backlog — it would never
// catch up to the current year and the map would appear frozen ("the worker gets
// stuck"). To keep the worker caught up we allow at most ONE `scrub_world` request
// in flight. While one is pending, later `scrubTo` calls only record the NEWEST
// target year (coalescing) and that single target is dispatched once the in-flight
// request finishes, skipping a burst of intermediate dispatches.
//
// The stale-response guard is keyed to the LATEST DISPATCHED request id, not to
// every `scrubTo` call. Because coalesced calls don't dispatch, they don't advance
// the guard — so the response to the currently in-flight (latest dispatched)
// request is always committed, and the map advances monotonically through
// playback. (Since scrubbing is monotonic forward during plays, commits never
// regress the displayed year; a manually-stale out-of-order reply is dropped
// because its reqId no longer matches a live dispatch.)
//
// All world inputs (pack / timeline / cells / era bounds) are read from the store
// at dispatch time rather than while rendering, so a scrub issued by a stale
// closure (e.g. right after async timeline generation resolves) still targets the
// current world instead of silently dropping.

import { useRef } from "react";
import type { WorldAt } from "../core/api";
import { coreApi } from "../core/api";
import { useWorldgenStore } from "../state/worldgenStore";

export function useTimelineScrub(): (targetYear: number) => void {
	// id of the most recent *dispatched* `scrub_world` request. A worker response
	// is committed only when its reqId still matches this (the stale-response
	// guard). Coalesced (non-dispatched) `scrubTo` calls do NOT advance it.
	const scrubReqIdRef = useRef(0);
	// Last *committed* projected year — the forward-vs-jump hint for the worker.
	const lastProjectedYearRef = useRef<number | null>(null);

	// Coalescing state. `inFlightRef` holds the reqId of the request currently
	// awaiting a worker reply (0 = none in flight). `pendingYearRef` holds the
	// newest target year that arrived while a request was in flight (null = none
	// pending). Refs (not component state) are required because the scrub flow
	// runs across async boundaries, not during render.
	const inFlightRef = useRef(0);
	const pendingYearRef = useRef<number | null>(null);

	/** Dispatch one scrub_world request for `targetYear`. Reads the freshest
	 *  world inputs from the store at call time. `reqId` must already equal
	 *  `scrubReqIdRef.current` (this is the latest dispatched request). */
	const dispatch = (
		reqId: number,
		targetYear: number,
		fromYear: number,
		prevWorld?: WorldAt,
	) => {
		const s = useWorldgenStore.getState();
		const pack = s.statesResult?.pack;
		const cells_state = s.statesResult?.cells_state ?? [];
		const cells_culture = s.culturesResult?.cells_culture ?? [];
		const cells_religion = s.culturesResult?.cells_religion ?? [];
		const cells_burg = s.statesResult?.cells_burg ?? [];
		const tl = s.timeline;

		if (!pack || !tl) {
			inFlightRef.current = 0;
			pendingYearRef.current = null;
			s.setScrubStatus("idle");
			return;
		}

		s.setCurrentYear(targetYear);
		s.setScrubStatus("loading");
		inFlightRef.current = reqId;

		coreApi
			.scrubWorld(
				pack,
				cells_state,
				cells_culture,
				cells_religion,
				cells_burg,
				tl,
				fromYear,
				targetYear,
				prevWorld,
			)
			.then((world: WorldAt) => {
				// Commit this projection. Because forward scrubs are monotonic,
				// committing each dispatched response advances the map smoothly
				// without ever regressing. Guard against a stale out-of-order
				// reply: only commit the response matching the latest dispatch.
				if (reqId !== scrubReqIdRef.current) return;
				lastProjectedYearRef.current = world.year;
				useWorldgenStore.getState().setProjectedWorld(world);
				useWorldgenStore.getState().setScrubStatus("idle");
			})
			.catch(() => {
				if (reqId !== scrubReqIdRef.current) return;
				useWorldgenStore.getState().setScrubStatus("error");
			})
			.finally(() => {
				// Only the request currently holding the in-flight slot may drain
				// the coalesced pending target (a newer dispatch owns the slot).
				if (inFlightRef.current !== reqId) return;
				inFlightRef.current = 0;
				const pending = pendingYearRef.current;
				pendingYearRef.current = null;
				if (pending != null && pending !== lastProjectedYearRef.current) {
					const nextReqId = ++scrubReqIdRef.current;
					const prevWorld =
						lastProjectedYearRef.current != null
							? (useWorldgenStore.getState().projectedWorld ?? undefined)
							: undefined;
					dispatch(
						nextReqId,
						pending,
						lastProjectedYearRef.current ?? useWorldgenStore.getState().eraStart,
						prevWorld,
					);
				}
			});
	};

	return (targetYear: number) => {
		const s = useWorldgenStore.getState();
		s.setCurrentYear(targetYear);

		if (!s.statesResult?.pack || !s.timeline) {
			s.setScrubStatus("idle");
			return;
		}

		// Coalesce: if a scrub is already in flight, don't queue another worker
		// request — just record the newest target and hand off when the in-flight
		// request finishes (its .finally() performs the latest-wins dispatch).
		if (inFlightRef.current !== 0) {
			pendingYearRef.current = targetYear;
			return;
		}

		const reqId = ++scrubReqIdRef.current;
		const fromYear = lastProjectedYearRef.current ?? s.eraStart;
		const prevWorld =
			lastProjectedYearRef.current != null
				? (s.projectedWorld ?? undefined)
				: undefined;
		dispatch(reqId, targetYear, fromYear, prevWorld);
	};
}
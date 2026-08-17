// Phase 5.1: shared timeline scrubbing logic.
//
// Extracted from Timeline.tsx so that both the Timeline scrubber UI and the
// EntityInspector History tab can trigger a projection to a specific year
// without duplicating the stale-response-guard / worker-envelope plumbing.
//
// `scrubTo` posts a `scrub_world` message to the worker via `coreApi.scrubWorld`,
// resolves the resulting `WorldAt`, and commits it to the store (which MapCanvas
// picks up to live-morph entity data textures). The stale reqId guard ensures a
// faster earlier request can't overwrite a slower later one.

import { useRef } from "react";
import { coreApi } from "../core/api";
import type { WorldAt } from "../core/api";
import { useWorldgenStore } from "../state/worldgenStore";

export function useTimelineScrub(): (targetYear: number) => void {
	const statesResult = useWorldgenStore((s) => s.statesResult);
	const culturesResult = useWorldgenStore((s) => s.culturesResult);
	const timeline = useWorldgenStore((s) => s.timeline);
	const eraStart = useWorldgenStore((s) => s.eraStart);
	const setCurrentYear = useWorldgenStore((s) => s.setCurrentYear);
	const setScrubStatus = useWorldgenStore((s) => s.setScrubStatus);
	const setProjectedWorld = useWorldgenStore((s) => s.setProjectedWorld);

	// Monotonically increasing reqId for the most recent scrub call.
	const scrubReqIdRef = useRef(0);
	// Last projected year, so we know which direction (forward vs jump) to
	// request from the worker.
	const lastProjectedYearRef = useRef<number | null>(null);

	return (targetYear: number) => {
		const reqId = ++scrubReqIdRef.current;
		setCurrentYear(targetYear);
		setScrubStatus("loading");

		const pack = statesResult?.pack;
		const cells_state = statesResult?.cells_state ?? [];
		const cells_culture = culturesResult?.cells_culture ?? [];
		const cells_religion = culturesResult?.cells_religion ?? [];
		const cells_burg = statesResult?.cells_burg ?? [];
		const tl = timeline;

		if (!pack || !tl) {
			setScrubStatus("idle");
			return;
		}

		const fromYear = lastProjectedYearRef.current ?? eraStart;
		const prevWorld =
			lastProjectedYearRef.current != null
				? useWorldgenStore.getState().projectedWorld ?? undefined
				: undefined;

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
				// Stale guard: only commit if this is the most recent request.
				if (reqId !== scrubReqIdRef.current) return;
				lastProjectedYearRef.current = world.year;
				setProjectedWorld(world);
				setScrubStatus("idle");
			})
			.catch(() => {
				if (reqId !== scrubReqIdRef.current) return;
				setScrubStatus("error");
			});
	};
}

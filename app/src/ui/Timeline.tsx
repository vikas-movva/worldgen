// Phase 5.1: timeline scrubber UI (Step 5.1).
//
// Renders a year slider + play/pause + speed control. Drives the worker's
// `scrub_world` endpoint to project the world at `currentYear` and stores the
// resulting `WorldAt` in the zustand store (which MapCanvas/layers.ts reads to
// update entity textures without rebuilding geometry).
//
// Stale-response guard: each scrub call tags its `reqId`; the store tracks
// `scrubReqId`. When the response arrives, we only commit the projection if its
// reqId still matches — a faster earlier request can't overwrite a slower later
// one (design §5.1 verification: "Old worker responses cannot overwrite newer
// years").

import { useEffect } from "react";
import { coreApi } from "../core/api";
import { useWorldgenStore } from "../state/worldgenStore";
import { useTimelineScrub } from "./useTimelineScrub";

/** Playback speed options in years/sec. */
const SPEED_OPTIONS = [1, 5, 10, 25, 50];

export function Timeline(): React.ReactElement | null {
	const grid = useWorldgenStore((s) => s.grid);
	const statesResult = useWorldgenStore((s) => s.statesResult);
	const culturesResult = useWorldgenStore((s) => s.culturesResult);
	const timeline = useWorldgenStore((s) => s.timeline);
	const eraStart = useWorldgenStore((s) => s.eraStart);
	const eraEnd = useWorldgenStore((s) => s.eraEnd);
	const currentYear = useWorldgenStore((s) => s.currentYear);
	const isPlaying = useWorldgenStore((s) => s.isPlaying);
	const playbackSpeed = useWorldgenStore((s) => s.playbackSpeed);
	const scrubStatus = useWorldgenStore((s) => s.scrubStatus);
	const setTimeline = useWorldgenStore((s) => s.setTimeline);
	const setCurrentYear = useWorldgenStore((s) => s.setCurrentYear);
	const setIsPlaying = useWorldgenStore((s) => s.setIsPlaying);
	const setPlaybackSpeed = useWorldgenStore((s) => s.setPlaybackSpeed);
	const setScrubStatus = useWorldgenStore((s) => s.setScrubStatus);

	// Shared scrub function (extracted to useTimelineScrub.ts) used by both
	// the slider and the play/pause tick below.
	const scrubTo = useTimelineScrub();
	useEffect(() => {
		if (!grid || !statesResult || !culturesResult) {
			setTimeline(null, 0, 1000);
			return;
		}
		if (timeline) return; // already generated

		const pack = statesResult.pack;
		const cells_state = statesResult.cells_state;
		const cells_culture = culturesResult.cells_culture;
		const cells_religion = culturesResult.cells_religion;
		const cells_burg = statesResult.cells_burg;

		const eraStart = 0;
		const eraEnd = 1000;
		coreApi
			.generateTimeline(
				pack,
				cells_state,
				cells_culture,
				cells_religion,
				cells_burg,
				grid.cells.h,
				grid.cells.province,
				grid.mesh,
				grid.seed,
				eraStart,
				eraEnd,
				{},
			)
			.then((tl) => {
				setTimeline(tl, eraStart, eraEnd);
				// Project to the era start (year 0 baseline) so the map shows
				// the initial state immediately.
				scrubTo(eraStart);
				setScrubStatus("idle");
			})
			.catch(() => {
				setScrubStatus("error");
			});
	}, [grid, statesResult, culturesResult, timeline, setTimeline]);

	// Playback tick: advance year by speed * dt each rAF.
	useEffect(() => {
		if (!isPlaying || !timeline) return;
		let raf: number;
		let lastT = 0;
		const tick = (t: number) => {
			if (lastT === 0) lastT = t;
			const dt = (t - lastT) / 1000; // seconds
			lastT = t;
			const deltaYears = playbackSpeed * dt;
			const next = Math.min(eraEnd, currentYear + deltaYears);
			if (next >= eraEnd) {
				setIsPlaying(false);
				setCurrentYear(eraEnd);
				scrubTo(eraEnd);
				return;
			}
			setCurrentYear(next);
			scrubTo(next);
			raf = requestAnimationFrame(tick);
		};
		raf = requestAnimationFrame(tick);
		return () => cancelAnimationFrame(raf);
	}, [isPlaying, currentYear, playbackSpeed]);

	if (!grid || !timeline) return null;

	return (
		<div
			data-testid="timeline-scrubber"
			style={{
				display: "flex",
				flexDirection: "column",
				gap: "0.35rem",
				padding: "0.5rem",
				borderTop: "1px solid #30363d",
				fontSize: "0.8rem",
				color: "#e6edf3",
			}}
		>
			<div style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}>
				<button
					type="button"
					onClick={() => {
						if (isPlaying) {
							setIsPlaying(false);
						} else {
							setIsPlaying(true);
						}
					}}
					style={{
						padding: "0.2rem 0.5rem",
						fontSize: "0.85rem",
						cursor: "pointer",
						border: "1px solid #30363d",
						background: "#161b22",
						color: "#e6edf3",
						borderRadius: "4px",
					}}
				>
					{isPlaying ? "Pause" : "Play"}
				</button>
				<label style={{ fontSize: "0.75rem", color: "#8b949e" }}>
					Speed:{" "}
					<select
						value={playbackSpeed}
						onChange={(e) => setPlaybackSpeed(Number(e.target.value))}
						style={{
							fontSize: "0.75rem",
							background: "#0d1117",
							color: "#e6edf3",
							border: "1px solid #30363d",
							borderRadius: "3px",
							padding: "0.1rem 0.3rem",
						}}
					>
						{SPEED_OPTIONS.map((s) => (
							<option key={s} value={s}>
								{s}×
							</option>
						))}
					</select>
				</label>
				<span style={{ fontSize: "0.75rem", color: "#8b949e", marginLeft: "auto" }}>
					{scrubStatus === "loading" ? "loading…" : scrubStatus === "error" ? "error" : ""}
				</span>
			</div>
			<div style={{ display: "flex", alignItems: "center", gap: "0.4rem" }}>
				<span style={{ fontSize: "0.72rem", color: "#8b949e", minWidth: "3ch" }}>
					{eraStart}
				</span>
				<input
					type="range"
					min={eraStart}
					max={eraEnd}
					value={currentYear}
					onChange={(e) => {
						const y = Number(e.target.value);
						if (isPlaying) setIsPlaying(false);
						scrubTo(y);
					}}
					style={{
						flex: "1",
						accentColor: "#2f81f7",
						height: "4px",
					}}
				/>
				<span style={{ fontSize: "0.72rem", color: "#8b949e", minWidth: "3ch" }}>
					{eraEnd}
				</span>
			</div>
		</div>
	);
}

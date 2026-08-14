// Step 2.5.3: heightmap-editor state slice + debounced `recomputeDependents`.
//
// The editor flow is:
//   pointermove → `editHeightmap` (instant, Rust core) + local temp/biome patch
//                 (`recomputeTempBiomeLocal`, Step 2.5.2) → live recolor.
//   stroke end / ≥300ms idle → `recomputeDependents` (this module) → full
//                 drainage + climate + biome recompute → texture swap.
//
// This module exposes:
//   - `useHeightmapEditor` — a zustand slice tracking the debounce timer, the
//     pending flag, and the last `DependentResult` (for the renderer / toasts).
//   - `scheduleDependentRecompute(grid, opts)` — the debounce entry point.
//     Called after each brush stroke end (or any heightmap edit that changes
//     land/water boundaries). Fires `recomputeDependents` after 300ms of idle,
//     coalescing rapid successive edits into one recompute.
//
// The 300ms gate keeps the UI responsive: the user can drag a brush
// continuously and the expensive full recompute (drainage + climate + biomes,
// ~50ms at 10k / ~400ms at 60k) runs once when they pause, not on every move.

import { create } from "zustand";
import type { DependentResult, Grid } from "../core/api";
import { coreApi } from "../core/api";

/** Debounce window (ms). FMG uses ~300ms for the same purpose. */
export const RECOMPUTE_DEBOUNCE_MS = 300;

export type HeightmapEditorState = {
	/** True when a recompute is pending (debounce timer running). */
	recomputePending: boolean;
	/** The most recent dependent recompute result (null before first run). */
	lastDependentResult: DependentResult | null;
	/** Error from the last recompute, if any (shown as a toast). */
	lastError: string | null;
};

export type HeightmapEditorActions = {
	/**
	 * Schedule a debounced `recomputeDependents`. Call after each brush stroke
	 * end or heightmap edit. Coalesces rapid edits within `RECOMPUTE_DEBOUNCE_MS`.
	 * Returns a promise that resolves with the `DependentResult` (or rejects
	 * on worker error). The promise from a superseded call (same timer slot)
	 * never resolves — the latest caller's promise wins.
	 *
	 * Serde fix: pass null for grid to use the Rust-held grid handle (no Grid
	 * on the wire, avoiding the 13.5MB serde round-trip). Pass a Grid only for
	 * backward compat or an explicitly-loaded grid.
	 */
	scheduleDependentRecompute: (
		grid: Grid | null,
		opts?: unknown,
	) => Promise<DependentResult>;
	/** Clear the pending timer and reset error (called on unmount / world regen). */
	clearPending: () => void;
};

type InternalState = {
	debounceTimer: ReturnType<typeof setTimeout> | null;
	resolveCurrent: ((r: DependentResult) => void) | null;
	rejectCurrent: ((e: Error) => void) | null;
};

const internal: InternalState = {
	debounceTimer: null,
	resolveCurrent: null,
	rejectCurrent: null,
};

export const useHeightmapEditor = create<
	HeightmapEditorState & HeightmapEditorActions
>()((set) => ({
	recomputePending: false,
	lastDependentResult: null,
	lastError: null,

	scheduleDependentRecompute: (grid: Grid | null, opts?) => {
		// Clear any pending timer + reject the previous caller's promise.
		if (internal.debounceTimer !== null) {
			clearTimeout(internal.debounceTimer);
			internal.debounceTimer = null;
		}
		if (internal.rejectCurrent) {
			internal.rejectCurrent(
				new Error("superseded by a newer recompute request"),
			);
			internal.rejectCurrent = null;
		}
		if (internal.resolveCurrent) {
			internal.resolveCurrent = null;
		}

		set({ recomputePending: true, lastError: null });

		return new Promise<DependentResult>((resolve, reject) => {
			internal.resolveCurrent = resolve;
			internal.rejectCurrent = reject;

			internal.debounceTimer = setTimeout(async () => {
				internal.debounceTimer = null;
				const resolver = internal.resolveCurrent;
				const rejecter = internal.rejectCurrent;
				internal.resolveCurrent = null;
				internal.rejectCurrent = null;
				try {
					// Serde fix: when grid is null, use the Rust-held grid
					// handle (no Grid on the wire). When grid is provided,
					// pass it explicitly (backward compat).
					const result = grid
						? await coreApi.recomputeDependents(opts, grid)
						: await coreApi.recomputeDependents(opts);
					set({
						recomputePending: false,
						lastDependentResult: result,
						lastError: null,
					});
					if (resolver) resolver(result);
				} catch (err) {
					const msg = err instanceof Error ? err.message : String(err);
					set({ recomputePending: false, lastError: msg });
					if (rejecter) {
						rejecter(err instanceof Error ? err : new Error(msg));
					}
				}
			}, RECOMPUTE_DEBOUNCE_MS);
		});
	},

	clearPending: () => {
		if (internal.debounceTimer !== null) {
			clearTimeout(internal.debounceTimer);
			internal.debounceTimer = null;
		}
		if (internal.rejectCurrent) {
			internal.rejectCurrent(new Error("cleared"));
			internal.rejectCurrent = null;
		}
		if (internal.resolveCurrent) {
			internal.resolveCurrent = null;
		}
		set({ recomputePending: false, lastError: null });
	},
}));

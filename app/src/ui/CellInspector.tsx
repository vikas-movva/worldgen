// Step 2.5.5: cell inspector + per-cell height edit.
//
// Sits beside the HeightmapEditor in the sidebar. Reads the currently
// selected cell from the zustand store (set by the editor's "select" tool)
// and renders a readout of its height / temperature / precipitation / biome
// plus a direct edit control (slider + ±1 / ±5 buttons) that pushes a
// single-cell `EditOp` through the worker.
//
// Per-cell edit pipeline (mirrors the brush stroke-hot path, design §3.6):
//   1. `coreApi.editHeightmap([{ mode: "Add", cells: [cellId], strength:
//      (target - current) / 100, ... }])` — `Add` with an explicit `cells`
//      set iterates just that cell and clamps via the Rust `lim` ([0,100]),
//      so it doubles as a "set height to value" op without a new mode.
//   2. Splice the returned `HeightmapPatch.h` into a new Grid object so the
//      MapCanvas store subscription fires `updateHeight` (height texture
//      re-upload) — the same splice the brush path uses.
//   3. `coreApi.recomputeTempBiomeLocal([cellId])` — Tier-1 local recompute
//      of the edited cell's temp + biome from the new `h` (altitude lapse +
//      neighbor mean). Splice the returned single-entry arrays into the
//      Grid so `updateBiome` fires. <16ms, same path as the brush live patch.
//   4. `scheduleDependentRecompute(null)` — the debounced full recompute
//      (rivers/lakes/coastline/precip/full-biome/entity-repair) so that a
//      land→water or water→land flip via the inspector drives the same
//      repair cascade as a brush stroke-end. The 300ms debounce coalesces
//      rapid slider drags into one recompute.
//
// The inspector only renders a cell when one is selected (`selectedCellId >= 0`)
// and the grid is loaded. It is intentionally a separate component from the
// HeightmapEditor so the readout/numeric edit UI doesn't clutter the brush
// palette; both share the store + the same edit/recompute pipeline.

import { useCallback, useState } from "react";
import {
	coreApi,
	type DependentResult,
	type EditOp,
	type Grid,
	type HeightmapPatch,
	spliceDependentResult,
} from "../core/api";
import { BIOME_NAMES } from "../render/palette";
import { useHeightmapEditor } from "../state/heightmapEditorStore";
import { useWorldgenStore } from "../state/worldgenStore";

type CellInspectorProps = {
	/**
	 * Current WorldMap (for re-drawing the selection outline after an edit
	 * touches the grid, so the yellow outline stays on the same cell).
	 */
	worldMap: import("../render/layers").WorldMap | null;
};

/** Sea level — matches the Rust `SEA_LEVEL` (h >= 20 is land). */
const SEA_LEVEL = 20;

export function CellInspector({
	worldMap,
}: CellInspectorProps): React.ReactElement | null {
	const grid = useWorldgenStore((s) => s.grid);
	const selectedCellId = useWorldgenStore((s) => s.selectedCellId);
	const setGrid = useWorldgenStore((s) => s.setGrid);

	const scheduleDependentRecompute = useHeightmapEditor(
		(s) => s.scheduleDependentRecompute,
	);
	const recomputePending = useHeightmapEditor((s) => s.recomputePending);
	const lastError = useHeightmapEditor((s) => s.lastError);

	const [statusMsg, setStatusMsg] = useState<string>("");

	// IDs: -1 means "no selection" or "grid unloaded".
	const id = selectedCellId;
	const hasSelection = grid !== null && id >= 0 && id < grid.cells.h.length;
	// Snapshot the live cell values from the current grid. Reading inside the
	// render means a stale grid reference would show stale numbers; but the
	// store grid is updated on every edit, so the inspector re-renders with the
	// new values immediately after the splice.
	const h = hasSelection ? grid.cells.h[id] : null;
	const temp = hasSelection ? (grid.cells.temp?.[id] ?? null) : null;
	const prec = hasSelection ? (grid.cells.prec?.[id] ?? null) : null;
	const biomeId = hasSelection ? (grid.cells.biome?.[id] ?? 0) : 0;
	const biomeName =
		biomeId >= 0 && biomeId < BIOME_NAMES.length
			? BIOME_NAMES[biomeId]
			: `Biome ${biomeId}`;
	const isWater = hasSelection ? (h ?? 0) < SEA_LEVEL : false;

	/** Push a single-cell height-set edit + local temp/biome recompute. */
	const setCellHeight = useCallback(
		async (target: number) => {
			if (!grid || !hasSelection) return;
			const clamped = Math.max(0, Math.min(100, Math.round(target)));
			const cur = grid.cells.h[id];
			if (clamped === cur) return; // no-op, no recompute churn

			// `Add` with an explicit `cells: [id]` set + strength = delta/100
			// applies `lim(h[id] + delta)` = `lim(clamped)` = `clamped`.
			// radius/0 are ignored for Add (it iterates `cells` directly).
			const op: EditOp = {
				mode: "Add",
				center_cell: id,
				target_cell: id,
				radius: 0,
				strength: (clamped - cur) / 100,
				cells: [id],
			};

			setStatusMsg("");
			try {
				const result = await coreApi.editHeightmap([op]);
				// Splice the h patch into a new Grid so subscribers
				// (MapCanvas `updateHeight`) fire.
				let nextGrid: Grid;
				if (
					result &&
					typeof result === "object" &&
					"h" in result &&
					(result as HeightmapPatch).h instanceof Uint8Array
				) {
					const patch = result as HeightmapPatch;
					const newH = Array.from(patch.h);
					nextGrid = {
						...grid,
						cells: { ...grid.cells, h: newH },
					};
				} else {
					nextGrid = result as Grid;
				}

				// Tier-1 local temp/biome recompute for just this cell.
				const local = await coreApi.recomputeTempBiomeLocal([id], {});
				// Splice the single-entry patch into temp/biome so
				// `updateBiome` fires for the biome texture.
				if (local && local.temp?.length === 1 && local.biome?.length === 1) {
					const tempArr = nextGrid.cells.temp
						? nextGrid.cells.temp.slice()
						: [];
					const biomeArr = nextGrid.cells.biome
						? nextGrid.cells.biome.slice()
						: [];
					tempArr[id] = local.temp[0];
					biomeArr[id] = local.biome[0];
					nextGrid = {
						...nextGrid,
						cells: { ...nextGrid.cells, temp: tempArr, biome: biomeArr },
					};
				}

				setGrid(nextGrid);
				// Keep the yellow selection outline on the same cell (the
				// selection overlay reads grid meshes, which are unchanged,
				// so a redraw is harmless but cheap).
				worldMap?.setSelected(nextGrid, id);

				// Debounced full recompute — reconciles rivers/lakes/precip/
				// biome entity repair on land/water flips, same as brush
				// stroke-end. Coalesces rapid slider drags into one run.
				// The store's `scheduleDependentRecompute` handles dedup:
				// it rejects the prior promise when a new request supersedes
				// it, so the old `.then` is routed to `.catch` and skips the
				// splice. No local guard needed here.
				scheduleDependentRecompute(null)
					.then((dep: DependentResult) => {
						// Splice the full recompute arrays back into the
						// store grid so temp/prec/biome/entity arrays reflect
						// the post-edit drainage + repair. Same splice as the
						// brush stroke-end handler in HeightmapEditor.
						const cur2 = useWorldgenStore.getState().grid;
						if (cur2) {
							const spliced = spliceDependentResult(cur2, dep);
							setGrid(spliced);
							worldMap?.setSelected(spliced, id);
						}
						setStatusMsg(
							`Recompute done: ${dep.rivers?.length ?? 0} rivers, ` +
								`${dep.lakes?.length ?? 0} lakes`,
						);
					})
					.catch(() => {
						// Error is surfaced via `lastError` from the store.
					});
			} catch (err) {
				setStatusMsg(
					`Edit error: ${err instanceof Error ? err.message : String(err)}`,
				);
			}
		},
		[grid, hasSelection, id, scheduleDependentRecompute, setGrid, worldMap],
	);

	if (!grid) return null;

	if (!hasSelection) {
		return (
			<div
				data-testid="cell-inspector"
				style={{
					padding: "0.5rem",
					borderTop: "1px solid #21262d",
					fontSize: "0.8rem",
					color: "#6e7681",
				}}
			>
				Select a cell (Inspect → click) to inspect it.
			</div>
		);
	}

	return (
		<div
			data-testid="cell-inspector"
			style={{
				display: "flex",
				flexDirection: "column",
				gap: "0.45rem",
				padding: "0.5rem",
				borderTop: "1px solid #30363d",
				fontSize: "0.82rem",
				color: "#e6edf3",
			}}
		>
			<span
				style={{
					fontSize: "0.72rem",
					color: "#8b949e",
					textTransform: "uppercase",
					letterSpacing: "0.05em",
				}}
			>
				Cell inspector
			</span>

			{/* Readout */}
			<table
				style={{
					width: "100%",
					borderCollapse: "collapse",
					fontSize: "0.78rem",
					fontFamily: "monospace",
				}}
			>
				<tbody>
					<ReadoutRow label="Cell" value={`${id}`} />
					<ReadoutRow
						label="Height"
						value={h !== null ? `${h} / 100` : "?"}
						accent={isWater ? "#4f6c8a" : "#8fbf5f"}
					/>
					<ReadoutRow
						label="Type"
						value={isWater ? "Water" : "Land"}
						accent={isWater ? "#4f6c8a" : "#8fbf5f"}
					/>
					<ReadoutRow label="Temp" value={temp !== null ? `${temp}°C` : "?"} />
					<ReadoutRow
						label="Precip"
						value={prec !== null ? `${prec} mm` : "?"}
					/>
					<ReadoutRow label="Biome" value={`${biomeId} · ${biomeName}`} />
				</tbody>
			</table>

			{/* Height edit (land/water crossing). */}
			<label
				style={{
					display: "flex",
					flexDirection: "column",
					gap: "0.2rem",
				}}
			>
				<span style={{ fontSize: "0.75rem", color: "#8b949e" }}>
					Set height: {h ?? 0} / 100
					{isWater ? " (water)" : " (land)"}
				</span>
				<input
					type="range"
					min={0}
					max={100}
					step={1}
					value={h ?? 0}
					onChange={(e) => {
						const v = Number(e.target.value);
						void setCellHeight(v);
					}}
					style={{ accentColor: "#2f81f7" }}
					data-testid="cell-height-slider"
				/>
			</label>

			{/* Quick +/- nudge buttons. */}
			<div
				style={{
					display: "flex",
					gap: "0.25rem",
					flexWrap: "wrap",
				}}
			>
				{[1, 5, -1, -5].map((delta) => (
					<button
						key={delta}
						type="button"
						onClick={() => void setCellHeight((h ?? 0) + delta)}
						style={{
							padding: "0.25rem 0.5rem",
							fontSize: "0.78rem",
							cursor: "pointer",
							border: delta > 0 ? "1px solid #2ea043" : "1px solid #da3633",
							background: "transparent",
							color: delta > 0 ? "#3fb950" : "#f85149",
							borderRadius: "4px",
						}}
						data-testid={`cell-height-delta-${delta > 0 ? "p" : "m"}${Math.abs(delta)}`}
					>
						{delta > 0 ? `+${delta}` : delta}
					</button>
				))}
				<button
					type="button"
					onClick={() => void setCellHeight(SEA_LEVEL)}
					style={{
						padding: "0.25rem 0.5rem",
						fontSize: "0.78rem",
						cursor: "pointer",
						border: "1px solid #30363d",
						background: "transparent",
						color: "#8b949e",
						borderRadius: "4px",
					}}
					title="Set to sea level (20)"
					data-testid="cell-height-sea"
				>
					Sea
				</button>
			</div>

			{/* Status line. */}
			<div
				data-testid="inspector-status"
				style={{
					fontSize: "0.72rem",
					color: "#8b949e",
					minHeight: "1.5em",
					lineHeight: 1.4,
					whiteSpace: "pre-wrap",
					wordBreak: "break-word",
				}}
			>
				{recomputePending ? "Recomputing dependents..." : ""}
				{lastError ? `Error: ${lastError}` : ""}
				{statusMsg || ""}
			</div>
		</div>
	);
}

function ReadoutRow({
	label,
	value,
	accent,
}: {
	label: string;
	value: string;
	accent?: string;
}): React.ReactElement {
	return (
		<tr>
			<td
				style={{
					color: "#8b949e",
					paddingRight: "0.5rem",
					verticalAlign: "top",
				}}
			>
				{label}
			</td>
			<td
				style={{
					color: accent ?? "#e6edf3",
					fontWeight: 500,
					overflowWrap: "anywhere",
				}}
			>
				{value}
			</td>
		</tr>
	);
}

export default CellInspector;

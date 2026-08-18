// Step 2.5.4: Heightmap editor toolbar + canvas painting bridge.
//
// This component renders the editor controls (tool palette, brush radius/strength
// sliders, Reset button, cell info) and wires canvas pointer events to the
// Rust core's `edit_heightmap` + `recomputeTempBiomeLocal` + debounced
// `recomputeDependents` pipeline.
//
// Painting flow (design section 5):
//   pointerdown  -> start stroke, determine center_cell via `pickCell`
//   pointermove  -> for each move, build an EditOp with the current tool +
//                   center_cell, call `editHeightmap` (updates h in the grid),
//                   then `recomputeTempBiomeLocal` for the edited cells ->
//                   live recolor. The expensive drainage/climate/biome full
//                   recompute is debounced (300ms) via `scheduleDependentRecompute`.
//   pointerup    -> end stroke, flush the debounced recompute immediately.
//
// The editor reads the current tool/radius/strength from `useWorldgenStore`
// and the debounce state from `useHeightmapEditor`. It needs a reference to
// the `WorldMap` instance (via the `MapCanvasHandle`) to do screen->world
// coordinate conversion for `pickCell`.

import { useCallback, useEffect, useRef, useState } from "react";
import {
	coreApi,
	type DependentResult,
	type EditMode,
	type Grid,
	type HeightmapPatch,
	spliceDependentResult,
} from "../core/api";
import type { WorldMap } from "../render/layers";
import { useHeightmapEditor } from "../state/heightmapEditorStore";
import type { EditorTool } from "../state/worldgenStore";
import { useWorldgenStore } from "../state/worldgenStore";
import { useIsMobile } from "./useIsMobile";

// Map editor tool names to Rust EditMode variants.
export const TOOL_TO_MODE: Record<EditorTool, EditMode> = {
	raise: "Raise",
	lower: "Lower",
	flatten: "Flatten",
	smooth: "Smooth",
	range: "Range",
	trough: "Trough",
	strait: "Strait",
	mask: "Mask",
	invert: "Invert",
	add: "Add",
	multiply: "Multiply",
	select: "Raise", // select is handled differently (no edit)
	pan: "Raise", // pan is handled differently (camera only, no edit)
};

// Tools that use the brush radius/strength (continuous painting tools).
export const BRUSH_TOOLS = new Set<EditorTool>([
	"raise",
	"lower",
	"flatten",
	"smooth",
]);

// Macro tools that operate over a brush-radius neighbourhood (they gather the
// radius-bounded cell set around a click — the Rust `apply_macro` gathers from
// `center_cell`+`radius` when `cells` is empty). A single click applies them.
export const AREA_MACRO_TOOLS = new Set<EditorTool>([
	"strait",
	"mask",
	"invert",
	"add",
	"multiply",
]);

// Macro tools that build a ridge path between TWO clicks: the first sets the
// start cell, the second sets the `target_cell` endpoint (FMG addRange/
// addTrough UX).
export const TARGET_MACRO_TOOLS = new Set<EditorTool>(["range", "trough"]);

export const MACRO_TOOLS = new Set<EditorTool>([
	...AREA_MACRO_TOOLS,
	...TARGET_MACRO_TOOLS,
]);

// Compact glyph icons for each tool button (see `toolIcon`).
export const TOOL_ICONS: Record<EditorTool, string> = {
	pan: "✋",
	select: "↖",
	raise: "▲",
	lower: "▼",
	flatten: "▬",
	smooth: "∿",
	range: "⛰",
	trough: "▽",
	strait: "≈",
	mask: "◐",
	invert: "⇅",
	add: "⊕",
	multiply: "×",
};

type HeightmapEditorProps = {
	/** The WorldMap instance from the MapCanvas handle, for screen->world transform. */
	worldMap: WorldMap | null;
	/** The Pixi app's canvas element, for attaching pointer listeners. */
	canvasEl: HTMLCanvasElement | null;
};

export function HeightmapEditor({
	worldMap,
	canvasEl,
}: HeightmapEditorProps): React.ReactElement | null {
	const grid = useWorldgenStore((s) => s.grid);
	const editorTool = useWorldgenStore((s) => s.editorTool);
	const brushRadius = useWorldgenStore((s) => s.brushRadius);
	const brushStrength = useWorldgenStore((s) => s.brushStrength);
	const selectedCellId = useWorldgenStore((s) => s.selectedCellId);
	const setEditorTool = useWorldgenStore((s) => s.setEditorTool);
	const setBrushRadius = useWorldgenStore((s) => s.setBrushRadius);
	const setBrushStrength = useWorldgenStore((s) => s.setBrushStrength);
	const setSelectedCellId = useWorldgenStore((s) => s.setSelectedCellId);
	const setGrid = useWorldgenStore((s) => s.setGrid);

	// Mobile: the editor lives in a full-width slide-over drawer on phones, so
	// tool buttons need comfortably large (≥44px) touch targets and the brush
	// sliders benefit from a touch-friendly taller range.
	const isMobile = useIsMobile();

	const scheduleDependentRecompute = useHeightmapEditor(
		(s) => s.scheduleDependentRecompute,
	);
	const recomputePending = useHeightmapEditor((s) => s.recomputePending);
	const lastError = useHeightmapEditor((s) => s.lastError);

	// Stroke state refs (don't trigger re-renders on every pointermove).
	const isPainting = useRef(false);
	const lastEditGrid = useRef<Grid | null>(null);
	const editedCellIds = useRef<Set<number>>(new Set());
	// For the two-click Range/Trough tools, the cell where the gesture started
	// (the `target_cell` for the ridge walk endpoint lands on the second click).
	const targetMacroStart = useRef<number>(-1);
	// Spacebar-to-pan: when Space is held, the camera pans and the editor
	// must NOT paint. Tracked independently of attachCamera's own spaceDown
	// (both listen to the same window keydown/keyup events).
	const spaceDown = useRef(false);
	const [statusMsg, setStatusMsg] = useState<string>("");
	const [isResetting, setIsResetting] = useState(false);

	// Track Space (the pan modifier) so the editor can suppress painting
	// during a pan — prevents the heightmap from being edited while panning.
	useEffect(() => {
		const onKeyDown = (e: KeyboardEvent) => {
			if (e.code === "Space" && !spaceDown.current) {
				spaceDown.current = true;
				e.preventDefault();
			}
		};
		const onKeyUp = (e: KeyboardEvent) => {
			if (e.code === "Space") {
				spaceDown.current = false;
			}
		};
		window.addEventListener("keydown", onKeyDown);
		window.addEventListener("keyup", onKeyUp);
		return () => {
			window.removeEventListener("keydown", onKeyDown);
			window.removeEventListener("keyup", onKeyUp);
		};
	}, []);

	// Clear stroke state when a new world (different mesh) is generated.
	// Prevents stale lastEditGrid from spreading an old mesh into a new grid.
	useEffect(() => {
		if (
			grid &&
			lastEditGrid.current &&
			grid.mesh !== lastEditGrid.current.mesh
		) {
			lastEditGrid.current = null;
			editedCellIds.current.clear();
		}
	}, [grid]);

	// Convert a pointer event to world coordinates via the WorldMap's
	// screenToWorld, then call pickCell to find the nearest cell. Uses the
	// worker-held grid handle (no grid on the wire) for the hot path.
	const pointerToCell = useCallback(
		async (e: PointerEvent | React.PointerEvent): Promise<number> => {
			if (!worldMap || !canvasEl) return -1;
			const rect = canvasEl.getBoundingClientRect();
			const screenX = e.clientX - rect.left;
			const screenY = e.clientY - rect.top;
			const { x, y } = worldMap.screenToWorld(screenX, screenY);
			const cellId = await coreApi.pickCell(x, y);
			return cellId;
		},
		[worldMap, canvasEl],
	);

	// Apply a single edit op + local temp/biome recompute for live recolor.
	// Uses the worker-held grid handle (no grid on the wire) so the hot drag
	// path avoids the ~28ms serde round-trip per pointermove.
	//
	// `targetCell` is used by the two-click Range/Trough tools: it becomes the
	// `target_cell` endurance for the ridge walk built in the Rust core.
	const applyEdit = useCallback(
		async (_g: Grid, centerCell: number, tool: EditorTool, targetCell?: number) => {
			if (centerCell < 0) return _g;

			const mode = TOOL_TO_MODE[tool];
			// Convert brushRadius (UI units ~1-100) to world-space distance using
			// the mesh's average cell spacing. Brush tools use it for continuous
			// falloff painting; AREA_MACRO_TOOLS use it because the Rust
			// `apply_macro` gathers the radius-bounded cell set (when `cells` is
			// empty) — without a radius, a macro click would affect a single cell.
			const avgSpacing =
				_g.mesh.world_w / Math.sqrt(_g.mesh.points.length) || 1;
			const wantsRadius =
				BRUSH_TOOLS.has(tool) || AREA_MACRO_TOOLS.has(tool);
			const worldRadius = wantsRadius ? brushRadius * avgSpacing * 0.5 : 0;
			// Brush tools use continuous brushStrength. Range/Trough scale a ridge
			// height in Rust so they want a stronger fixed value; other area macros
			// use a fixed value so a single click is visibly impactful rather than
			// the near-zero 0.05 default.
			const strength = BRUSH_TOOLS.has(tool)
				? brushStrength
				: TARGET_MACRO_TOOLS.has(tool)
					? 0.6
					: 0.4;

			try {
				// No grid arg → worker uses Rust-held grid (serde fix).
				// Returns a thin { h: Uint8Array } patch, not a full Grid.
				const result = await coreApi.editHeightmap([
					{
						mode,
						center_cell: centerCell,
						target_cell: targetCell ?? centerCell,
						radius: worldRadius,
						strength,
						cells: [],
					},
				]);

				// Splice the h patch into a NEW grid object so React/zustand
				// subscribers detect the change via reference inequality. The
				// mesh + other cell arrays are unchanged by a heightmap edit,
				// so we shallow-copy `cells` and swap only `h`.
				let updatedGrid: Grid;
				if (
					result &&
					typeof result === "object" &&
					"h" in result &&
					result.h instanceof Uint8Array
				) {
					const patch = result as HeightmapPatch;
					const base = lastEditGrid.current ?? _g;
					updatedGrid = {
						...base,
						cells: {
							...base.cells,
							h: Array.from(patch.h),
						},
					};
				} else {
					// Full Grid return (backward compat / explicit grid arg).
					updatedGrid = result as Grid;
				}

				lastEditGrid.current = updatedGrid;

				// Live recolor: local temp/biome patch on edited cells.
				if (BRUSH_TOOLS.has(tool)) {
					editedCellIds.current.add(centerCell);
					if (editedCellIds.current.size > 0) {
						const cellIds = Array.from(editedCellIds.current);
						await coreApi.recomputeTempBiomeLocal(cellIds, {});
					}
				}

				return updatedGrid;
			} catch (err) {
				setStatusMsg(
					`Edit error: ${err instanceof Error ? err.message : String(err)}`,
				);
				return _g;
			}
		},
		[brushRadius, brushStrength],
	);

	// Flush the debounced full dependent recompute (drainage + climate + biome
	// + entity repair) and splice the result + fresh river/lake geometry back
	// into the store. Extracted so one-shot macro edits (which never reach
	// pointerup) re-render rivers/lakes/biomes exactly like a brush-stroke end.
	const flushRecompute = useCallback(() => {
		void scheduleDependentRecompute(null)
			.then((result: DependentResult) => {
				const cur = useWorldgenStore.getState().grid;
				if (cur) {
					const next = spliceDependentResult(cur, result);
					setGrid(next);
					lastEditGrid.current = next;
				}
				useWorldgenStore
					.getState()
					.setDrainageGeometry(result.rivers ?? [], result.lakes ?? []);
				setStatusMsg(
					`Recompute done: ${result.rivers?.length ?? 0} rivers, ` +
						`${result.lakes?.length ?? 0} lakes`,
				);
			})
			// A rejected recompute (superseded / cleared while the debounce is
			// still pending) is surfaced via the editor store's `lastError`;
			// swallow the promise rejection here so it's never an unhandled
			// rejection in tests or console.
			.catch(() => {});
	}, [scheduleDependentRecompute, setGrid]);

	// Handle a stroke start.
	const onPointerDown = useCallback(
		(e: PointerEvent) => {
			if (!grid || !worldMap || !canvasEl) return;
			// Suppress painting while Space is held (pan mode).
			if (spaceDown.current) return;
			// The Pan tool is a pure camera gesture — the camera owns the
			// pointer and the editor must not paint or select.
			if (editorTool === "pan") return;
			if (editorTool === "select") {
				// Min-zoom gate (Step 2.5.5 #7): at 60k cells a cell is sub-pixel
				// when zoomed out, so a click selects an essentially arbitrary
				// cell near the cursor. Require the user to be zoomed in to at
				// least ~1.5x the fit-to-screen base scale before picking. Below
				// that we surface a hint instead of a misleading selection.
				const minSelectZoom = 1.5;
				if (worldMap.getZoom() < minSelectZoom) {
					setSelectedCellId(-1);
					worldMap.setSelected(grid, -1);
					setStatusMsg(`Zoom in to select a cell (need ≥${minSelectZoom}x).`);
					return;
				}
				// Select mode: pick cell and draw selection outline.
				void (async () => {
					const cellId = await pointerToCell(e);
					setSelectedCellId(cellId);
					worldMap.setSelected(grid, cellId);
					setStatusMsg(
						cellId >= 0
							? `Cell ${cellId}: h=${grid.cells.h[cellId]} temp=${grid.cells.temp?.[cellId] ?? "?"} biome=${grid.cells.biome?.[cellId] ?? "?"}`
							: "No cell selected",
					);
				})();
				return;
			}

			// Macro tools are one-shot edits (no continuous painting). Range/
			// Trough use a two-click gesture: first click stores the start cell,
			// second click supplies the endpoint.
			if (MACRO_TOOLS.has(editorTool)) {
				if (TARGET_MACRO_TOOLS.has(editorTool)) {
					if (targetMacroStart.current < 0) {
						void (async () => {
							const cellId = await pointerToCell(e);
							targetMacroStart.current = cellId;
							setStatusMsg(
								cellId >= 0
									? `${editorTool}: start ${cellId} — click the endpoint.`
									: "No start cell",
							);
						})();
						return;
					}
					void (async () => {
						const end = await pointerToCell(e);
						const start = targetMacroStart.current;
						targetMacroStart.current = -1;
						if (start >= 0 && end >= 0) {
							const g = await applyEdit(grid, start, editorTool, end);
							setGrid(g);
							flushRecompute();
						}
					})();
					return;
				}
				// Area macro: apply once across the brush-radius neighborhood.
				void (async () => {
					const cellId = await pointerToCell(e);
					if (cellId >= 0) {
						const g = await applyEdit(grid, cellId, editorTool);
						setGrid(g);
						flushRecompute();
					}
				})();
				return;
			}

			// Brush tools: continuous painting stroke.
			isPainting.current = true;
			editedCellIds.current = new Set();
			canvasEl.setPointerCapture(e.pointerId);
			setStatusMsg("");

			void (async () => {
				const cellId = await pointerToCell(e);
				if (cellId >= 0) {
					const g = await applyEdit(grid, cellId, editorTool);
					setGrid(g);
				}
			})();
		},
		[
			grid,
			worldMap,
			canvasEl,
			editorTool,
			pointerToCell,
			applyEdit,
			flushRecompute,
			setGrid,
			setSelectedCellId,
		],
	);

	// Handle painting move.
	const onPointerMove = useCallback(
		(e: PointerEvent) => {
			if (!isPainting.current || !lastEditGrid.current) return;
			// Suppress painting while Space is held (pan mode).
			if (spaceDown.current) return;
			if (!BRUSH_TOOLS.has(editorTool)) return;

			const g = lastEditGrid.current;
			void (async () => {
				const cellId = await pointerToCell(e);
				if (cellId >= 0) {
					const newGrid = await applyEdit(g, cellId, editorTool);
					setGrid(newGrid);
				}
			})();
		},
		[editorTool, pointerToCell, applyEdit, setGrid],
	);

	// Handle stroke end: flush debounced recompute.
	const onPointerUp = useCallback(
		(e: PointerEvent) => {
			if (!isPainting.current) return;
			isPainting.current = false;
			canvasEl?.releasePointerCapture(e.pointerId);
			editedCellIds.current = new Set();
			// Flush the debounced recompute. Pass null for grid to use the
			// Rust-held grid handle (serde fix — no 13.5MB Grid on the wire).
			flushRecompute();
		},
		[flushRecompute, canvasEl],
	);

	// Attach pointer listeners to the canvas element.
	useEffect(() => {
		if (!canvasEl) return;
		canvasEl.addEventListener("pointerdown", onPointerDown);
		canvasEl.addEventListener("pointermove", onPointerMove);
		canvasEl.addEventListener("pointerup", onPointerUp);
		return () => {
			canvasEl.removeEventListener("pointerdown", onPointerDown);
			canvasEl.removeEventListener("pointermove", onPointerMove);
			canvasEl.removeEventListener("pointerup", onPointerUp);
		};
	}, [canvasEl, onPointerDown, onPointerMove, onPointerUp]);

	// Reset button handler: regenerate h from seed. Uses the worker handle.
	const onReset = useCallback(async () => {
		if (isResetting) return;
		setIsResetting(true);
		setStatusMsg("Resetting heightmap...");
		try {
			const result = await coreApi.resetHeightmap();
			let resetGrid: Grid;
			if (
				result &&
				typeof result === "object" &&
				"h" in result &&
				result.h instanceof Uint8Array
			) {
				const patch = result as HeightmapPatch;
				const base = lastEditGrid.current ?? grid;
				if (base) {
					resetGrid = {
						...base,
						cells: {
							...base.cells,
							h: Array.from(patch.h),
							state: base.cells.state.map(() => -1),
							province: base.cells.province.map(() => -1),
							culture: base.cells.culture.map(() => -1),
							religion: base.cells.religion.map(() => -1),
							burg: base.cells.burg.map(() => 0),
						},
					};
				} else {
					// No base grid (shouldn't happen — editor requires grid).
					return;
				}
			} else {
				resetGrid = result as Grid;
			}
			setGrid(resetGrid);
			lastEditGrid.current = resetGrid;
			setSelectedCellId(-1);
			worldMap?.setSelected(resetGrid, -1);
			setStatusMsg("Heightmap reset — recomputing rivers/lakes/biomes...");
			// The reset only rewrites `h` (and clears entity arrays); the
			// temperature / biome textures and the river/lake overlays would go
			// stale against the new terrain. Run the full dependent recompute so
			// rivers, lakes and biomes regen from the reset heightmap.
			flushRecompute();
		} catch (err) {
			setStatusMsg(
				`Reset error: ${err instanceof Error ? err.message : String(err)}`,
			);
		} finally {
			setIsResetting(false);
		}
	}, [isResetting, setGrid, setSelectedCellId, worldMap, grid, flushRecompute]);

	// Don't render the editor if no grid is loaded.
	if (!grid) return null;

	const toolGroups: { label: string; tools: EditorTool[] }[] = [
		{ label: "Navigate", tools: ["pan", "select"] },
		{ label: "Brush", tools: ["raise", "lower", "flatten", "smooth"] },
		{
			label: "Macro",
			tools: ["range", "trough", "strait", "mask", "invert", "add", "multiply"],
		},
	];
	// Brush radius/strength controls apply to both brush and macro editing
	// tools (pan/select are camera + inspection tools — no brush).
	const isEditingTool = BRUSH_TOOLS.has(editorTool) || MACRO_TOOLS.has(editorTool);

	return (
		<div
			data-testid="heightmap-editor"
			style={{
				display: "flex",
				flexDirection: "column",
				gap: "0.5rem",
				padding: "0.6rem",
				background: "#161b22",
				border: "1px solid #30363d",
				borderRadius: "6px",
				fontSize: "0.85rem",
				color: "#e6edf3",
				width: "100%",
				maxWidth: "100%",
				minWidth: 0,
				minHeight: 220,
				maxHeight: "100%",
				overflowY: "auto",
				WebkitOverflowScrolling: "touch",
			}}
		>
			{/* Tool palette */}
			{toolGroups.map((group) => (
				<div
					key={group.label}
					style={{ display: "flex", flexDirection: "column", gap: "0.3rem" }}
				>
					<span
						style={{
							fontSize: "0.7rem",
							color: "#8b949e",
							textTransform: "uppercase",
							letterSpacing: "0.05em",
						}}
					>
						{group.label}
					</span>
					<div
						style={{
							display: "flex",
							flexWrap: "wrap",
							gap: "0.25rem",
						}}
					>
						{group.tools.map((tool) => {
							const active = editorTool === tool;
							return (
								<button
									key={tool}
									type="button"
									aria-pressed={active}
									onClick={() => {
										setEditorTool(tool);
										// Cancel a half-started Range/Trough gesture when
										// switching tools mid-flow.
										targetMacroStart.current = -1;
									}}
									title={tool}
									data-testid={`tool-${tool}`}
									style={{
										display: "inline-flex",
										alignItems: "center",
										gap: "0.3rem",
										padding: "0.35rem 0.5rem",
										fontSize: "0.8rem",
										cursor: "pointer",
										// ≥44px touch targets on mobile (finger-friendly);
										// 32px on desktop for a denser toolbar.
										minHeight: isMobile ? 44 : 32,
										minWidth: isMobile ? 44 : 32,
										justifyContent: "center",
										border: active
											? "1px solid #2f81f7"
											: "1px solid #30363d",
										background: active ? "#1f6feb" : "transparent",
										color: active ? "#fff" : "#8b949e",
										borderRadius: "4px",
										textTransform: "capitalize",
									}}
								>
									<span
										aria-hidden="true"
										style={{ fontSize: "0.95rem", lineHeight: 1 }}
									>
										{TOOL_ICONS[tool]}
									</span>
									<span>{tool}</span>
								</button>
							);
						})}
					</div>
				</div>
			))}

			{/* Brush controls (brush + macro editing tools) */}
			{isEditingTool && (
				<div
					style={{ display: "flex", flexDirection: "column", gap: "0.4rem" }}
				>
					<label
						style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}
					>
						<span style={{ fontSize: "0.72rem", color: "#8b949e" }}>
							Brush radius: {brushRadius}
						</span>
						<input
							type="range"
							min={1}
							max={100}
							value={brushRadius}
							onChange={(e) => setBrushRadius(Number(e.target.value))}
							style={{
								accentColor: "#2f81f7",
								touchAction: "auto",
								width: "100%",
								height: isMobile ? 44 : "auto",
							}}
							data-testid="brush-radius"
						/>
					</label>
					<label
						style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}
					>
						<span style={{ fontSize: "0.72rem", color: "#8b949e" }}>
							Brush strength: {brushStrength.toFixed(2)}
						</span>
						<input
							type="range"
							min={0}
							max={1}
							step={0.05}
							value={brushStrength}
							onChange={(e) => setBrushStrength(Number(e.target.value))}
							style={{
								accentColor: "#2f81f7",
								touchAction: "auto",
								width: "100%",
								height: isMobile ? 44 : "auto",
							}}
							data-testid="brush-strength"
						/>
					</label>
				</div>
			)}

			{/* Reset button */}
			<button
				type="button"
				onClick={onReset}
				disabled={isResetting}
				style={{
					padding: "0.45rem 0.6rem",
					fontSize: "0.85rem",
					cursor: isResetting ? "wait" : "pointer",
					minHeight: isMobile ? 44 : 36,
					border: "1px solid #da3633",
					background: isResetting ? "#3d1010" : "transparent",
					color: "#f85149",
					borderRadius: "4px",
				}}
			>
				{isResetting ? "Resetting..." : "Reset Heightmap"}
			</button>

			{/* Status / cell info */}
			<div
				data-testid="editor-status"
				style={{
					fontSize: "0.75rem",
					color: "#8b949e",
					minHeight: "2.5em",
					lineHeight: 1.4,
					whiteSpace: "pre-wrap",
					wordBreak: "break-word",
				}}
			>
				{recomputePending ? "Recomputing dependents..." : ""}
				{lastError ? `Error: ${lastError}` : ""}
				{statusMsg || ""}
				{targetMacroStart.current >= 0
					? " — now click the Range/Trough endpoint"
					: ""}
				{!recomputePending && !lastError && !statusMsg && selectedCellId >= 0
					? `Cell ${selectedCellId} selected`
					: ""}
			</div>

			{/* Active tool indicator */}
			<div
				style={{
					fontSize: "0.72rem",
					color: "#6e7681",
					borderTop: "1px solid #21262d",
					paddingTop: "0.4rem",
				}}
			>
				Tool:{" "}
				<span style={{ color: "#e6edf3", textTransform: "capitalize" }}>
					{editorTool}
				</span>
			</div>
		</div>
	);
}

export default HeightmapEditor;

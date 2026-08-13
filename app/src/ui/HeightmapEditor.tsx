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
import { coreApi, type DependentResult, type EditMode, type Grid } from "../core/api";
import { WorldMap } from "../render/layers";
import { useHeightmapEditor } from "../state/heightmapEditorStore";
import type { EditorTool } from "../state/worldgenStore";
import { useWorldgenStore } from "../state/worldgenStore";

// Map editor tool names to Rust EditMode variants.
const TOOL_TO_MODE: Record<EditorTool, EditMode> = {
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
};

// Tools that use the brush radius/strength (continuous painting tools).
const BRUSH_TOOLS = new Set<EditorTool>(["raise", "lower", "flatten", "smooth"]);

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

  const scheduleDependentRecompute = useHeightmapEditor(
    (s) => s.scheduleDependentRecompute,
  );
  const recomputePending = useHeightmapEditor((s) => s.recomputePending);
  const lastError = useHeightmapEditor((s) => s.lastError);

  // Stroke state refs (don't trigger re-renders on every pointermove).
  const isPainting = useRef(false);
  const lastEditGrid = useRef<Grid | null>(null);
  const editedCellIds = useRef<Set<number>>(new Set());
  const [statusMsg, setStatusMsg] = useState<string>("");
  const [isResetting, setIsResetting] = useState(false);

  // Convert a pointer event to world coordinates via the WorldMap's
  // screenToWorld, then call pickCell to find the nearest cell.
  const pointerToCell = useCallback(
    async (
      e: PointerEvent | React.PointerEvent,
      g: Grid,
    ): Promise<number> => {
      if (!worldMap || !canvasEl) return -1;
      const rect = canvasEl.getBoundingClientRect();
      const screenX = e.clientX - rect.left;
      const screenY = e.clientY - rect.top;
      const { x, y } = worldMap.screenToWorld(screenX, screenY);
      const cellId = await coreApi.pickCell(g, x, y);
      return cellId;
    },
    [worldMap, canvasEl],
  );

  // Apply a single edit op + local temp/biome recompute for live recolor.
  const applyEdit = useCallback(
    async (g: Grid, centerCell: number, tool: EditorTool) => {
      if (centerCell < 0 || centerCell >= g.cells.h.length) return g;

      const mode = TOOL_TO_MODE[tool];
      // For brush tools, convert brushRadius (UI units ~1-100) to world-space
      // distance. The mesh's average cell spacing gives us the scale.
      // For macro tools, radius is typically 0 or irrelevant.
      const avgSpacing =
        g.mesh.world_w / Math.sqrt(g.mesh.points.length) || 1;
      const worldRadius = BRUSH_TOOLS.has(tool)
        ? brushRadius * avgSpacing * 0.5
        : 0;
      const strength = BRUSH_TOOLS.has(tool) ? brushStrength : 0.5;

      try {
        const newGrid = await coreApi.editHeightmap(g, [
          {
            mode,
            center_cell: centerCell,
            target_cell: centerCell,
            radius: worldRadius,
            strength,
            cells: [],
          },
        ]);

        // Track edited cells for the local temp/biome patch.
        // The Rust core auto-gathers cells in radius; we approximate the
        // edited set by recording the center cell. The local recompute
        // also handles neighbors, so this is a lower bound.
        editedCellIds.current.add(centerCell);

        // Live recolor: local temp/biome patch on edited cells.
        if (editedCellIds.current.size > 0 && BRUSH_TOOLS.has(tool)) {
          const cellIds = Array.from(editedCellIds.current);
          await coreApi.recomputeTempBiomeLocal(newGrid, cellIds, {});
        }

        lastEditGrid.current = newGrid;
        return newGrid;
      } catch (err) {
        setStatusMsg(`Edit error: ${err instanceof Error ? err.message : String(err)}`);
        return g;
      }
    },
    [brushRadius, brushStrength],
  );

  // Handle a stroke start.
  const onPointerDown = useCallback(
    (e: PointerEvent) => {
      if (!grid || !worldMap || !canvasEl) return;
      if (editorTool === "select") {
        // Select mode: pick cell and draw selection outline.
        void (async () => {
          const cellId = await pointerToCell(e, grid);
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

      isPainting.current = true;
      editedCellIds.current = new Set();
      canvasEl.setPointerCapture(e.pointerId);
      setStatusMsg("");

      void (async () => {
        const cellId = await pointerToCell(e, grid);
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
      setGrid,
      setSelectedCellId,
    ],
  );

  // Handle painting move.
  const onPointerMove = useCallback(
    (e: PointerEvent) => {
      if (!isPainting.current || !lastEditGrid.current) return;
      if (!BRUSH_TOOLS.has(editorTool)) return;

      const g = lastEditGrid.current;
      void (async () => {
        const cellId = await pointerToCell(e, g);
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

      // Flush the debounced recompute with the latest grid.
      if (lastEditGrid.current) {
        void scheduleDependentRecompute(lastEditGrid.current).then(
          (result: DependentResult) => {
            setStatusMsg(
              `Recompute done: ${result.rivers?.length ?? 0} rivers, ` +
                `${result.lakes?.length ?? 0} lakes`,
            );
          },
        );
      }
      editedCellIds.current = new Set();
    },
    [scheduleDependentRecompute],
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

  // Reset button handler: regenerate h from seed.
  const onReset = useCallback(async () => {
    if (!grid || isResetting) return;
    setIsResetting(true);
    setStatusMsg("Resetting heightmap...");
    try {
      const newGrid = await coreApi.resetHeightmap(grid);
      setGrid(newGrid);
      lastEditGrid.current = newGrid;
      setSelectedCellId(-1);
      worldMap?.setSelected(newGrid, -1);
      setStatusMsg("Heightmap reset to seeded state.");
    } catch (err) {
      setStatusMsg(
        `Reset error: ${err instanceof Error ? err.message : String(err)}`,
      );
    } finally {
      setIsResetting(false);
    }
  }, [grid, isResetting, setGrid, setSelectedCellId, worldMap]);

  // Don't render the editor if no grid is loaded.
  if (!grid) return null;

  const toolGroups: { label: string; tools: EditorTool[] }[] = [
    { label: "Brush", tools: ["raise", "lower", "flatten", "smooth"] },
    { label: "Macro", tools: ["range", "trough", "strait", "mask", "invert", "add", "multiply"] },
    { label: "Inspect", tools: ["select"] },
  ];

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
        minWidth: 220,
        maxHeight: "100%",
        overflowY: "auto",
      }}
    >
      {/* Tool palette */}
      {toolGroups.map((group) => (
        <div key={group.label} style={{ display: "flex", flexDirection: "column", gap: "0.3rem" }}>
          <span style={{ fontSize: "0.75rem", color: "#8b949e", textTransform: "uppercase", letterSpacing: "0.05em" }}>
            {group.label}
          </span>
          <div style={{ display: "flex", flexWrap: "wrap", gap: "0.25rem" }}>
            {group.tools.map((tool) => (
              <button
                key={tool}
                type="button"
                onClick={() => setEditorTool(tool)}
                aria-pressed={editorTool === tool}
                title={tool}
                style={{
                  padding: "0.3rem 0.55rem",
                  fontSize: "0.8rem",
                  cursor: "pointer",
                  border: editorTool === tool ? "1px solid #2f81f7" : "1px solid #30363d",
                  background: editorTool === tool ? "#1f6feb" : "transparent",
                  color: editorTool === tool ? "#fff" : "#8b949e",
                  borderRadius: "4px",
                  textTransform: "capitalize",
                }}
              >
                {tool}
              </button>
            ))}
          </div>
        </div>
      ))}

      {/* Brush controls (only for brush tools) */}
      {BRUSH_TOOLS.has(editorTool) && (
        <div style={{ display: "flex", flexDirection: "column", gap: "0.4rem" }}>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            <span style={{ fontSize: "0.75rem", color: "#8b949e" }}>
              Brush radius: {brushRadius}
            </span>
            <input
              type="range"
              min={1}
              max={100}
              value={brushRadius}
              onChange={(e) => setBrushRadius(Number(e.target.value))}
              style={{ accentColor: "#2f81f7" }}
            />
          </label>
          <label style={{ display: "flex", flexDirection: "column", gap: "0.2rem" }}>
            <span style={{ fontSize: "0.75rem", color: "#8b949e" }}>
              Brush strength: {brushStrength.toFixed(2)}
            </span>
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={brushStrength}
              onChange={(e) => setBrushStrength(Number(e.target.value))}
              style={{ accentColor: "#2f81f7" }}
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
          padding: "0.4rem 0.6rem",
          fontSize: "0.85rem",
          cursor: isResetting ? "wait" : "pointer",
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
        {!recomputePending && !lastError && !statusMsg && selectedCellId >= 0
          ? `Cell ${selectedCellId} selected`
          : ""}
      </div>

      {/* Active tool indicator */}
      <div style={{ fontSize: "0.72rem", color: "#6e7681", borderTop: "1px solid #21262d", paddingTop: "0.4rem" }}>
        Tool: <span style={{ color: "#e6edf3", textTransform: "capitalize" }}>{editorTool}</span>
      </div>
    </div>
  );
}

export default HeightmapEditor;

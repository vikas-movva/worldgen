// PixiJS canvas host — Step 2.2.
//
// Creates a Pixi `Application` (WebGL2 preference) inside a sized ref div, draws a
// placeholder background, auto-resizes with the container, and tears down cleanly
// on unmount. Under React 19 StrictMode the effect runs twice (mount → unmount →
// mount); the destroy guard in cleanup ensures no second WebGL context leaks.
//
// This step renders a placeholder rectangle only — terrain/biome mesh rendering
// is Step 2.3. The component reads the current `Grid` from the zustand store so
// the renderer is wired to state from the start (design §5, tech-req §7: the
// canvas holds the current Grid; the store owns state, not Pixi objects).

import { Application, Container, Graphics } from "pixi.js";
import { useEffect, useRef, useState } from "react";
import type { Grid } from "../core/api";
import { coreApi } from "../core/api";
import { useGrid, useWorldgenStore } from "../state/worldgenStore";
import { attachCamera, WorldMap } from "./layers";

/**
 * Step 3.4: push the combined Phase-3 entity payload to a `WorldMap`. Builds
 * the `setEntities` argument by merging the states pack + per-cell arrays from
 * `statesResult` with the culture/religion vectors + per-cell arrays from
 * `culturesResult`. No-op if either result is missing (the entity layers stay
 * transparent). The renderer reads `pack.states[i].color` etc. to color each
 * cell; unassigned cells render transparent so terrain shows through.
 */
function pushEntities(
	worldMap: WorldMap,
	grid: Grid,
	st: {
		statesResult: {
			pack: {
				states: { color: number }[];
				provinces: { color: number }[];
				cultures: { color: number }[];
				religions: { color: number }[];
			};
			cells_state: number[];
			cells_province: number[];
			cells_burg: number[];
		} | null;
		culturesResult: {
			cultures: { color: number }[];
			religions: { color: number }[];
			cells_culture: number[];
			cells_religion: number[];
		} | null;
	},
): void {
	if (!st.statesResult || !st.culturesResult) return;
	worldMap.setEntities(grid, {
		pack: {
			states: st.statesResult.pack.states,
			provinces: st.statesResult.pack.provinces,
			cultures: st.culturesResult.cultures,
			religions: st.culturesResult.religions,
		},
		cells_state: st.statesResult.cells_state,
		cells_province: st.statesResult.cells_province,
		cells_culture: st.culturesResult.cells_culture,
		cells_religion: st.culturesResult.cells_religion,
	});
}

// Dark map background (matches the app theme) so the canvas is visible before
// any geometry is drawn. The placeholder text is drawn with Pixi primitives so
// we don't pull in @pixi/text for Step 2.2 (avoid font-loading edge cases in the
// WebGL2 mounting test; a real label is a Step 2.3 concern).
const BG_COLOR = 0x0d1117;
const PLACEHOLDER_COLOR = 0x1f2c3d;

export type MapCanvasHandle = {
	/** The Pixi Application, or null before init / after destroy. */
	app: Application | null;
	/** Root container for layers added by later steps (terrain, biome, ...). */
	worldLayer: Container | null;
	/** Step 2.5.4: the WorldMap instance (for screen->world transform + selection). */
	worldMap: WorldMap | null;
	/** Step 2.5.4: the canvas DOM element (for attaching pointer listeners). */
	canvasEl: HTMLCanvasElement | null;
};

export type MapCanvasProps = {
	/**
	 * Optional callback fired once the Pixi Application has initialised. The
	 * caller receives the app + the root `worldLayer` container it should add
	 * its layers to. Used by tests and by later steps to mount terrain/biome
	 * meshes without re-querying the ref.
	 */
	onReady?: (handle: MapCanvasHandle) => void;
	/**
	 * Step 2.5.4: fired whenever the `WorldMap` instance is (re)built or
	 * destroyed. The editor uses this to get the current `WorldMap` for
	 * screen->world transform + cell selection overlay.
	 */
	onWorldMapChange?: (
		worldMap: WorldMap | null,
		canvasEl: HTMLCanvasElement | null,
	) => void;
};

/**
 * Mount a PixiJS Application inside a div and keep it sized to that div.
 *
 * Lifecycle (the StrictMode-safe part):
 * - `initApp()` constructs the `Application`, awaits `app.init({ preference:
 *   'webgl', resizeTo: containerRef })`, appends `app.canvas`, draws the
 *   placeholder, and stores the handle in a ref.
 * - The cleanup function guards with a `destroyedRef` flag so that a re-entrant
 *   StrictMode unmount→mount cycle cannot create two live Applications: the
 *   first unmount destroys the app and sets the flag; the second mount creates
 *   a fresh one. `app.destroy({ removeView: true }, { context: true })` forces
 *   WebGL context loss so the GPU context is actually released, not just the
 *   canvas node.
 */
export function MapCanvas({
	onReady,
	onWorldMapChange,
}: MapCanvasProps = {}): React.ReactElement {
	const containerRef = useRef<HTMLDivElement | null>(null);
	const appRef = useRef<Application | null>(null);
	const worldLayerRef = useRef<Container | null>(null);
	const destroyedRef = useRef(false);
	const onReadyRef = useRef(onReady);
	onReadyRef.current = onReady;
	const onWorldMapChangeRef = useRef(onWorldMapChange);
	onWorldMapChangeRef.current = onWorldMapChange;
	const worldMapRef = useRef<WorldMap | null>(null);
	const unsubRef = useRef<(() => void) | null>(null);
	const cleanupListenersRef = useRef<Array<() => void>>([]);
	const resizeObserverRef = useRef<ResizeObserver | null>(null);

	// Surface a minimal status string for debugging + tests. The component is
	// intentionally side-effect-only for rendering; this state does not drive
	// Pixi logic.
	const [status, setStatus] = useState<string>("initialising");

	// Read the current Grid from the store. Step 2.2 only renders a placeholder,
	// but we subscribe now so the wiring (store → renderer) is real from day one.
	// A non-null grid flips the placeholder label from "no world" to "world ready".
	const grid: Grid | null = useGrid();

	useEffect(() => {
		const container = containerRef.current;
		if (!container) return;

		let cancelled = false;
		destroyedRef.current = false;

		const host = container; // non-null (early-return above)

		async function initApp() {
			//_preference: 'webgl' forces WebGL2 (PixiJS v8 auto-detects WebGL2);
			// resizeTo: container makes the renderer track the div, not the window,
			// so the canvas lives inside the React layout rather than filling it.
			const app = new Application();
			try {
				await app.init({
					preference: "webgl",
					backgroundColor: BG_COLOR,
					resizeTo: host,
					antialias: true,
					autoDensity: true,
					resolution: Math.min(window.devicePixelRatio || 1, 2),
					powerPreference: "high-performance",
				});
			} catch (err) {
				setStatus(`init failed: ${String(err)}`);
				return;
			}

			// If a re-entrant unmount already ran while we were awaiting init,
			// destroy the app we just built and bail — do not attach it.
			if (cancelled || destroyedRef.current) {
				app.destroy({ removeView: true }, { context: true });
				return;
			}

			host.appendChild(app.canvas);

			// worldLayer is the root container for terrain/biome/entity layers
			// added by later steps. Keeping it separate from app.stage means
			// Step 2.3 can clear+rebuild just the world without touching overlays.
			const worldLayer = new Container();
			worldLayer.label = "worldLayer";
			app.stage.addChild(worldLayer);

			// Debug hook for verify scripts — set early so diagnostics can
			// inspect even before the world is built.
			(window as unknown as Record<string, unknown>).__WORLDFORGE_DEBUG__ = {
				app,
				worldLayer,
				worldMapRef,
			};

			// Placeholder: a centred filled rect so the canvas is visibly alive
			// before Step 2.3 draws real polygons. Replaced when terrain lands.
			const placeholder = new Graphics();
			placeholder.label = "placeholder";
			const drawPlaceholder = () => {
				const w = app.screen.width;
				const h = app.screen.height;
				const pw = Math.min(w * 0.6, 360);
				const ph = Math.min(h * 0.18, 64);
				placeholder.clear();
				placeholder
					.rect((w - pw) / 2, (h - ph) / 2, pw, ph)
					.fill({ color: PLACEHOLDER_COLOR });
			};
			drawPlaceholder();
			worldLayer.addChild(placeholder);

			// Build the terrain + biome render from the current Grid, and keep it
			// in sync with the store (regeneration / layer toggles) without
			// re-mounting the canvas.
			let worldMap: WorldMap | null = null;
			let detachCamera: (() => void) | null = null;
			const buildMap = (grid: Grid | null) => {
				if (!grid) return;
				worldMap = new WorldMap(grid, {
					initialLayers: useWorldgenStore.getState().layerEnabled,
				});
				worldMapRef.current = worldMap;
				// Step 3.5: mirror map click-selections back into the store
				// so the EntityPanel highlights the clicked entity. The
				// store->map subscription then re-applies the selection to
				// the map, but selectEntity's value guard prevents a loop.
				worldMap.onSelectEntity = (kind, id) => {
					const cur = useWorldgenStore.getState().selectedEntity;
					if (cur && cur.kind === kind && cur.id === id) return;
					useWorldgenStore.getState().selectEntity({ kind, id });
				};
				// Fit the normalized [0,1]^2 geometry into the canvas pixel space.
				// Without this the entire world renders in a ~1px area at the
				// top-left corner — the root cause of the blank-canvas bug.
				worldMap.fitToScreen(app.screen.width, app.screen.height);
				worldLayer.addChild(worldMap.view);
				detachCamera = attachCamera(app.canvas, {
					worldMap,
					screenSize: () => ({ w: app.screen.width, h: app.screen.height }),
				});
				if (placeholder.parent) worldLayer.removeChild(placeholder);
				setStatus("world ready");
				// Step 2.5.6: push the current river/lake geometry (if any)
				// to the freshly-built overlay so a regenerated world does
				// not arrive without its drainage drawn. The store updates
				// first (App sets grid); buildMap runs next; this seeds
				// the new WorldMap with whatever drainage is current.
				const st = useWorldgenStore.getState();
				if (st.rivers.length || st.lakes.length) {
					worldMap.setRiversLakes(grid, st.rivers, st.lakes);
				}
				// Step 3.4: push entity layers (states/provinces/cultures/
				// religions) if the Phase-3 results are already present.
				pushEntities(worldMap, grid, st);
				// Step 2.5.4: notify the editor that a new WorldMap is available.
				onWorldMapChangeRef.current?.(worldMap, app.canvas);
			};

			// Step 3.4: click-to-select an entity. A genuine click (pointer
			// down + up without much movement) picks the cell under the cursor
			// and asks the WorldMap to outline every cell belonging to that
			// cell's entity (state/culture/province/religion, whichever entity
			// layer is currently on top). Drags (camera pan) don't trigger it
			// because we require the press to end within a small pixel radius
			// of where it started.
			let downX = 0;
			let downY = 0;
			let downT = 0;
			const onCanvasDown = (e: PointerEvent) => {
				downX = e.clientX;
				downY = e.clientY;
				downT = performance.now();
			};
			const onCanvasUp = async (e: PointerEvent) => {
				const moved = Math.hypot(e.clientX - downX, e.clientY - downY);
				const dt = performance.now() - downT;
				if (moved > 6 || dt > 600) return; // it was a drag/hold, not a click
				const wm = worldMapRef.current;
				const g = useWorldgenStore.getState().grid;
				if (!wm || !g) return;
				const rect = app.canvas.getBoundingClientRect();
				const { x, y } = wm.screenToWorld(
					e.clientX - rect.left,
					e.clientY - rect.top,
				);
				try {
					const cellId = await coreApi.pickCell(x, y);
					wm.setSelectedEntity(g, cellId);
					// setSelectedEntity selects an entity OR a single cell.
					// Mirror the resulting selection into the store.
					const sel = wm.getSelectedEntity();
					useWorldgenStore.getState().selectEntity(sel);
				} catch {
					/* pick failed; ignore */
				}
			};
			app.canvas.addEventListener("pointerdown", onCanvasDown);
			app.canvas.addEventListener("pointerup", onCanvasUp);
			// Store the removers so cleanup detaches them. Flush any prior
			// leftover listeners (e.g. from a previous buildMap) first so
			// regenerate never stacks duplicate pointer handlers.
			while (cleanupListenersRef.current.length) {
				cleanupListenersRef.current.pop()?.();
			}
			cleanupListenersRef.current.push(() => {
				app.canvas.removeEventListener("pointerdown", onCanvasDown);
				app.canvas.removeEventListener("pointerup", onCanvasUp);
			});
			const rebuildMap = (grid: Grid | null) => {
				if (worldMap) {
					detachCamera?.();
					worldLayer.removeChild(worldMap.view);
					worldMap.destroy();
					worldMap = null;
					worldMapRef.current = null;
					onWorldMapChangeRef.current?.(null, app.canvas);
				}
				buildMap(grid);
			};
			buildMap(useWorldgenStore.getState().grid);

			// One subscription covers grid regeneration + layer toggles.
			// Height edits (same mesh, only cells.h changed) use the fast
			// `updateHeight` texture-update path — no geometry/mesh rebuild.
			// Only a full world regeneration (new mesh reference) triggers
			// the expensive `rebuildMap` (destroy + recreate WorldMap).
			const unsub = useWorldgenStore.subscribe((state, prev) => {
				if (state.grid !== prev.grid) {
					if (prev.grid && state.grid && state.grid.mesh === prev.grid.mesh) {
						// Same mesh → height/temp/biome edit. Update textures in place.
						worldMap?.updateHeight(state.grid);
						worldMap?.updateBiome(state.grid);
					} else {
						rebuildMap(state.grid);
					}
				}
				// Step 2.5.6: river/lake geometry change (new world or post-edit
				// recompute). Push the fresh polylines + polygons to the overlay.
				if (state.rivers !== prev.rivers || state.lakes !== prev.lakes) {
					worldMap?.setRiversLakes(state.grid, state.rivers, state.lakes);
				}
				if (state.layerEnabled !== prev.layerEnabled)
					worldMap?.setLayers(state.layerEnabled);
				// Step 3.4: entity results changed (or first arrived). Push the
				// combined payload to the WorldMap so the entity layers fill.
				if (
					state.statesResult !== prev.statesResult ||
					state.culturesResult !== prev.culturesResult
				) {
					if (worldMap && state.grid) {
					pushEntities(worldMap, state.grid, state);
				}
				}
				// Step 5.2: projected world changed (timeline scrub). Live-morph
				// the entity data textures + borders without rebuilding geometry.
				if (state.projectedWorld !== prev.projectedWorld) {
					if (worldMap && state.projectedWorld && state.grid) {
					worldMap.updateEntities(state.projectedWorld, state.grid);
				}
				}
				// Step 3.5: a panel-driven entity selection. Mirror the store
				// selection onto the map (highlights the entity's cells; for
				// a state on the Provinces layer, also draws its border).
				if (state.selectedEntity !== prev.selectedEntity) {
					if (worldMap && state.grid && state.selectedEntity) {
					const sel = state.selectedEntity;
					worldMap.selectEntity(state.grid, sel.kind, sel.id);
				} else if (worldMap && state.grid) {
				worldMap.setSelected(state.grid, -1);
				}
				}
			});
			unsubRef.current = unsub;

			// Keep the placeholder centred when the container size changes.
			// Pixi's resizeTo handles the renderer/canvas size; we redraw the
			// placeholder so it stays visually centred. Also re-fit the world
			// map so the terrain stays visible after a window resize.
			const onResize = () => {
				drawPlaceholder();
				// Re-fit the world map on resize. fitToScreen re-applies the
				// current zoom multiplier, so a resize doesn't reset zoom.
				if (worldMap) {
					worldMap.fitToScreen(app.screen.width, app.screen.height);
				}
			};
			app.renderer.on("resize", onResize);

			// Belt-and-suspenders: Pixi's `resizeTo` uses a ResizeObserver
			// internally, but in some headless / flex-layout scenarios the
			// initial measurement fires before layout settles, leaving the
			// canvas undersized (508px vs 1216px container). This explicit
			// observer forces a renderer resize to the real container size
			// whenever it changes.
			const resizeObserver = new ResizeObserver((entries) => {
				const entry = entries[0];
				if (!entry) return;
				const w = Math.round(entry.contentRect.width);
				const h = Math.round(entry.contentRect.height);
				if (
					w > 0 &&
					h > 0 &&
					(w !== app.screen.width || h !== app.screen.height)
				) {
					app.renderer.resize(w, h);
				}
			});
			resizeObserver.observe(host);
			resizeObserverRef.current = resizeObserver;

			appRef.current = app;
			worldLayerRef.current = worldLayer;
			setStatus("canvas ready");

			onReadyRef.current?.({
				app,
				worldLayer,
				worldMap: null,
				canvasEl: app.canvas,
			});
		}

		initApp();

		return () => {
			destroyedRef.current = true;
			cancelled = true;
			unsubRef.current?.();
			unsubRef.current = null;
			// Step 3.4: detach any click-to-select listeners.
			while (cleanupListenersRef.current.length) {
				cleanupListenersRef.current.pop()?.();
			}
			resizeObserverRef.current?.disconnect();
			resizeObserverRef.current = null;
			const wm = worldMapRef.current;
			if (wm) {
				wm.destroy();
				worldMapRef.current = null;
			}
			const app = appRef.current;
			if (app) {
				// Fully tear down: remove the canvas from the DOM and force a
				// WebGL context loss so StrictMode's double-mount never leaks a
				// second live context. `removeView: true` detaches the canvas;
				// `context: true` calls `gl.getExtension('WEBGL_lose_context')`
				// and `.loseContext()` so the GPU actually frees the context.
				app.destroy({ removeView: true }, { context: true });
			}
			appRef.current = null;
			worldLayerRef.current = null;
			delete (window as unknown as Record<string, unknown>)
				.__WORLDFORGE_DEBUG__;
		};
		// Init effect only re-runs on mount/unmount. A separate effect below
		// reflects `grid` presence into the status string without re-mounting.
	}, []);

	// Reflect grid presence in the status line without re-mounting the canvas.
	useEffect(() => {
		setStatus(grid ? "world ready" : "canvas ready");
	}, [grid]);

	return (
		<div
			ref={containerRef}
			style={{
				position: "relative",
				width: "100%",
				height: "100%",
				minHeight: 320,
				overflow: "hidden",
				background: "#0d1117",
			}}
			data-testid="pixi-container"
			data-pixi-status={status}
		>
			{/* PixiJS appends its <canvas> here on init. No inline text to avoid
			    interfering with Pixi's pointer hit-testing on the canvas surface. */}
		</div>
	);
}

export default MapCanvas;

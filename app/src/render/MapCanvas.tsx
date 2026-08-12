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
import { useGrid, useWorldgenStore } from "../state/worldgenStore";
import { attachCamera, WorldMap } from "./layers";

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
};

export type MapCanvasProps = {
	/**
	 * Optional callback fired once the Pixi Application has initialised. The
	 * caller receives the app + the root `worldLayer` container it should add
	 * its layers to. Used by tests and by later steps to mount terrain/biome
	 * meshes without re-querying the ref.
	 */
	onReady?: (handle: MapCanvasHandle) => void;
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
}: MapCanvasProps = {}): React.ReactElement {
	const containerRef = useRef<HTMLDivElement | null>(null);
	const appRef = useRef<Application | null>(null);
	const worldLayerRef = useRef<Container | null>(null);
	const destroyedRef = useRef(false);
	const onReadyRef = useRef(onReady);
	onReadyRef.current = onReady;
	const worldMapRef = useRef<WorldMap | null>(null);
	const unsubRef = useRef<(() => void) | null>(null);
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
			};
			const rebuildMap = (grid: Grid | null) => {
				if (worldMap) {
					detachCamera?.();
					worldLayer.removeChild(worldMap.view);
					worldMap.destroy();
					worldMap = null;
					worldMapRef.current = null;
				}
				buildMap(grid);
			};
			buildMap(useWorldgenStore.getState().grid);

			// One subscription covers grid regeneration + layer toggles.
			const unsub = useWorldgenStore.subscribe((state, prev) => {
				if (state.grid !== prev.grid) rebuildMap(state.grid);
				if (state.layerEnabled !== prev.layerEnabled)
					worldMap?.setLayers(state.layerEnabled);
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
			});
		}

		initApp();

		return () => {
			destroyedRef.current = true;
			cancelled = true;
			unsubRef.current?.();
			unsubRef.current = null;
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

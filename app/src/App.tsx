import { useEffect, useRef, useState } from "react";
import {
	type CulturesResult,
	coreApi,
	type Grid,
	type StatesResult,
} from "./core/api";
import type { WorldMap } from "./render/layers";
import { MapCanvas, type MapCanvasHandle } from "./render/MapCanvas";
import { useWorldgenStore } from "./state/worldgenStore";
import { CellInspector } from "./ui/CellInspector";
import { EntityPanel } from "./ui/EntityPanel";
import { HeightmapEditor } from "./ui/HeightmapEditor";
import EntityInspector from "./ui/EntityInspector";
import { Timeline } from "./ui/Timeline";

function App() {
	const [result, setResult] = useState<string>("Loading WASM...");
	const [busy, setBusy] = useState(false);
	const [seed, setSeed] = useState(42);
	// Stopwatch counter — main-thread liveness indicator. If the worker blocks
	// the main thread, the rAF tick stream freezes during generation.
	const [tickCount, setTickCount] = useState(0);
	const tickRef = useRef(0);

	const grid = useWorldgenStore((s) => s.grid);
	const setGrid = useWorldgenStore((s) => s.setGrid);
	const setGenerationMeta = useWorldgenStore((s) => s.setGenerationMeta);
	const setDrainageGeometry = useWorldgenStore((s) => s.setDrainageGeometry);
	const setStatesResult = useWorldgenStore((s) => s.setStatesResult);
	const setCulturesResult = useWorldgenStore((s) => s.setCulturesResult);
	const layerEnabled = useWorldgenStore((s) => s.layerEnabled);
	const toggleLayer = useWorldgenStore((s) => s.toggleLayer);
	const toggleEntityLayer = useWorldgenStore((s) => s.toggleEntityLayer);

	const canvasHandleRef = useRef<MapCanvasHandle | null>(null);
	const [worldMap, setWorldMap] = useState<WorldMap | null>(null);
	const [canvasEl, setCanvasEl] = useState<HTMLCanvasElement | null>(null);

	useEffect(() => {
		let raf = 0;
		const loop = () => {
			tickRef.current += 1;
			setTickCount(tickRef.current);
			raf = requestAnimationFrame(loop);
		};
		raf = requestAnimationFrame(loop);
		return () => cancelAnimationFrame(raf);
	}, []);

	useEffect(() => {
		async function runAll() {
			// Step 2.1 regression suite via the worker bridge.
			async function runAdd() {
				try {
					const sum = await coreApi.add(2, 3);
					setResult(`add(2, 3) = ${sum} ${sum === 5 ? "PASS" : "FAIL"}`);
				} catch (err) {
					setResult(`Error: ${String(err)}`);
				}
			}
			async function runMesh() {
				try {
					const mesh = await coreApi.generateMesh(1000, 42);
					const pointCount = mesh.points?.length ?? 0;
					const cellCount = mesh.cells?.i?.length ? mesh.cells.i.length - 1 : 0;
					const hasVertices = mesh.vertices?.p?.length > 0;
					const countsMatch = pointCount === 1000 && cellCount === 1000;
					setResult(
						(prev: string) =>
							prev +
							`  | mesh: points=${pointCount}, cells=${cellCount}, verts=${mesh.vertices?.p?.length ?? 0} ` +
							`${countsMatch && hasVertices ? "PASS" : "FAIL"}`,
					);
				} catch (err) {
					setResult((prev: string) => `${prev}  | mesh Error: ${String(err)}`);
				}
			}
			async function runHeightmap() {
				try {
					const mesh = await coreApi.generateMesh(1000, 42);
					const h = await coreApi.generateHeightmap(mesh, 42);
					const len = h?.length ?? 0;
					let land = 0;
					for (let i = 0; i < len; i++) if (h[i] >= 20) land++;
					const frac = len > 0 ? land / len : 0;
					setResult(
						(prev: string) =>
							prev +
							`  | hmap: landFrac=${frac.toFixed(3)} ` +
							`${len === 1000 && frac >= 0.2 && frac <= 0.7 ? "PASS" : "FAIL"}`,
					);
				} catch (err) {
					setResult((prev: string) => `${prev}  | hmap Error: ${String(err)}`);
				}
			}
			await runAdd();
			await runMesh();
			await runHeightmap();
		}
		runAll();
	}, []);

	async function runGenerateWorld() {
		if (busy) return;
		setBusy(true);
		const tickBefore = tickRef.current;
		const t0 = performance.now();
		try {
			const g: Grid = await coreApi.generateWorld(seed, 60_000, {});
			const t1 = performance.now();
			const tickAfter = tickRef.current;
			setGrid(g);
			setGenerationMeta({
				seed,
				cellCount: g.cells.h.length,
				startedAt: t0,
				finishedAt: t1,
			});
			// Step 2.5.6: fetch river + lake geometry for the freshly-generated
			// world so the rivers/lakes overlays can render immediately (the
			// Grid carries per-cell `cells.r` but not the polyline/polygon
			// geometry). `generateWorld` auto-stores the grid into the worker's
			// held handle, so no Grid arg is needed.
			try {
				const geo = await coreApi.getDrainageGeometry();
				setDrainageGeometry(geo.rivers, geo.lakes);
			} catch (geoErr) {
				setResult(
					(prev: string) =>
						`${prev}  | drainage geometry Error: ${String(geoErr)}`,
				);
			}
			// Step 3.2 + 3.3: generate states/provinces/burgs and
			// cultures/religions on top of the freshly-generated world. These
			// populate the per-cell `state`/`province`/`culture`/`religion`
			// arrays (and the entity pack) that the entity layers render.
			// generateWorld already stored the grid into the worker's held
			// handle, so no grid arg is needed for the worker call — but the
			// api wrapper still passes it for the local path.
			let statesResult: StatesResult | null = null;
			let culturesResult: CulturesResult | null = null;
			try {
				const t2 = performance.now();
				statesResult = await coreApi.generateStates(g, seed, 12);
				setStatesResult(statesResult);
				culturesResult = await coreApi.generateCulturesReligions(
					g,
					seed,
					18,
					12,
					statesResult,
				);
				setCulturesResult(culturesResult);
				// Step 3.4/3.5 fix: splice the generated per-cell entity
				// arrays back into the main-thread grid so click-to-select
				// (which reads grid.cells.religion/culture/state/province to
				// detect the clicked entity) and the state-border overlay see
				// real data. The worker fills its internal heldGrid, but the
				// store's grid kept all-zero entity arrays — so selection and
				// borders silently failed. New grid reference triggers the
				// renderer's grid-subscription so the WorldMap re-reads cells.
				setGrid({
					...g,
					cells: {
						...g.cells,
						state: Array.from(statesResult.cells_state),
						province: Array.from(statesResult.cells_province),
						culture: Array.from(culturesResult.cells_culture),
						religion: Array.from(culturesResult.cells_religion),
					},
				});
				const t3 = performance.now();
				setResult(
					(prev: string) =>
						prev +
						`  | states=${statesResult?.pack.states.length} ` +
						`cultures=${culturesResult?.cultures.length} ` +
						`religions=${culturesResult?.religions.length} ` +
						`(${(t3 - t2).toFixed(0)}ms)`,
				);
			} catch (entErr) {
				setResult(
					(prev: string) =>
						`${prev}  | entity generation Error: ${String(entErr)}`,
				);
			}
			const n = g.cells.h.length;
			let land = 0;
			for (let i = 0; i < n; i++) if (g.cells.h[i] >= 20) land++;
			setResult(
				(prev: string) =>
					prev +
					`  | generateWorld(${seed}, 60k) = ${(t1 - t0).toFixed(0)}ms ` +
					`points=${g.mesh.points.length} land=${((land / n) * 100).toFixed(1)}% ` +
					`rAF=${tickAfter - tickBefore} ${t1 - t0 < 2000 && tickAfter - tickBefore > 30 ? "PASS" : "FAIL"}`,
			);
		} catch (err) {
			setResult(
				(prev: string) => `${prev}  | generateWorld Error: ${String(err)}`,
			);
		} finally {
			setBusy(false);
		}
	}
	async function runRegenerateEntities() {
		if (busy || !grid) return;
		setBusy(true);
		const t0 = performance.now();
		try {
			// Re-run the entity generators on the CURRENT grid with a fresh
			// entity seed so the user can reshuffle states/provinces/burgs +
			// cultures/religions without regenerating the terrain. The terrain
			// (mesh + heightmap + climate + biomes) is untouched — only the
			// per-cell entity index arrays and the pack vectors change.
			// NOTE: the worker's heldGrid is the post-`generateWorld` grid;
			// we pass the store grid explicitly so the local serde path is
			// used and the held grid stays in sync with the returned arrays.
			const entSeed = Math.floor(Math.random() * 1_000_000);
			const statesResult = await coreApi.generateStates(
				grid,
				entSeed,
				12,
				);
			setStatesResult(statesResult);
			const culturesResult = await coreApi.generateCulturesReligions(
				grid,
				entSeed,
				18,
				12,
				statesResult,
				);
			setCulturesResult(culturesResult);
			// Splice the fresh per-cell entity arrays back into the store grid
			// so click-to-select and the state-border overlay see them.
			setGrid({
					...grid,
					cells: {
						...grid.cells,
						state: Array.from(statesResult.cells_state),
						province: Array.from(statesResult.cells_province),
						culture: Array.from(culturesResult.cells_culture),
						religion: Array.from(culturesResult.cells_religion),
					},
				});
			const t1 = performance.now();
			setResult(
					(prev: string) =>
						prev +
						`  | regenerate entities: states=${statesResult.pack.states.length} ` +
						`cultures=${culturesResult.cultures.length} ` +
						`religions=${culturesResult.religions.length} ` +
						`(${(t1 - t0).toFixed(0)}ms)`,
				);
		} catch (err) {
			setResult(
				(prev: string) =>
					`${prev}  | regenerate entities Error: ${String(err)}`,
			);
		} finally {
			setBusy(false);
		}
	}

	return (
		<div
			style={{
				display: "flex",
				flexDirection: "column",
				height: "100vh",
				minHeight: "100vh",
				margin: 0,
				padding: 0,
				fontFamily: "system-ui, sans-serif",
				background: "#0d1117",
				color: "#e6edf3",
			}}
		>
			<header
				style={{
					display: "flex",
					alignItems: "center",
					gap: "0.75rem",
					padding: "0.5rem 1rem",
					borderBottom: "1px solid #30363d",
					flex: "0 0 auto",
				}}
			>
				<h1 style={{ margin: 0, fontSize: "1.1rem", fontWeight: 600 }}>
					<a
						href="https://github.com/vikas-movva/worldgen"
						target="_blank"
						rel="noopener noreferrer"
						style={{ color: "inherit", textDecoration: "none" }}
					>
						Worldgen
					</a>
				</h1>
				<button
					type="button"
					onClick={runGenerateWorld}
					disabled={busy}
					style={{
						padding: "0.35rem 0.85rem",
						fontSize: "0.9rem",
						cursor: busy ? "wait" : "pointer",
					}}
				>
					{busy ? "Generating..." : "Generate 60k world"}
				</button>
				<label
					style={{
						display: "flex",
						alignItems: "center",
						gap: "0.4rem",
						fontSize: "0.85rem",
						color: "#8b949e",
					}}
				>
					Seed:
					<input
						type="number"
						value={seed}
						disabled={busy}
						onChange={(e) => {
							const v = Number.parseInt(e.target.value, 10);
							setSeed(Number.isNaN(v) ? 0 : v);
						}}
						style={{
							width: "5.5rem",
							padding: "0.3rem 0.4rem",
							fontSize: "0.85rem",
							fontFamily: "monospace",
							color: "#e6edf3",
							background: "#0d1117",
							border: "1px solid #30363d",
							borderRadius: "4px",
						}}
					/>
				</label>
				<button
					type="button"
					onClick={() => setSeed(Math.floor(Math.random() * 1_000_000))}
					disabled={busy}
					style={{
						padding: "0.35rem 0.6rem",
						fontSize: "0.85rem",
						cursor: busy ? "wait" : "pointer",
						border: "1px solid #30363d",
						background: "transparent",
						color: "#8b949e",
					}}
					title="Randomize seed"
				>
					&#x1f3b2;
				</button>
				<LayerToggle
					label="Terrain"
					active={layerEnabled.terrain}
					onClick={() => toggleLayer("terrain")}
				/>
				<LayerToggle
					label="Biome"
					active={layerEnabled.biome}
					onClick={() => toggleLayer("biome")}
				/>
				<LayerToggle
					label="Rivers"
					active={layerEnabled.rivers}
					onClick={() => toggleLayer("rivers")}
				/>
				<LayerToggle
					label="Lakes"
					active={layerEnabled.lakes}
					onClick={() => toggleLayer("lakes")}
				/>
				<span
					style={{
						fontSize: "0.7rem",
						color: "#8b949e",
						marginLeft: "0.5rem",
						borderLeft: "1px solid #30363d",
						paddingLeft: "0.5rem",
					}}
				>
					Entities
				</span>
				<LayerToggle
					label="States"
					active={layerEnabled.states}
					onClick={() => toggleEntityLayer("state")}
				/>
				<LayerToggle
					label="Provinces"
					active={layerEnabled.provinces}
					onClick={() => toggleEntityLayer("province")}
				/>
				<LayerToggle
					label="Cultures"
					active={layerEnabled.cultures}
					onClick={() => toggleEntityLayer("culture")}
				/>
				<LayerToggle
					label="Religions"
					active={layerEnabled.religions}
					onClick={() => toggleEntityLayer("religion")}
				/>
				<button
					type="button"
					onClick={runRegenerateEntities}
					disabled={busy || !grid}
					style={{
						padding: "0.3rem 0.6rem",
						fontSize: "0.8rem",
						cursor: busy || !grid ? "wait" : "pointer",
						border: "1px solid #30363d",
						background: "transparent",
						color: "#8b949e",
					}}
					title="Re-run states/provinces/burgs + cultures/religions with a new entity seed"
				>
					Regenerate entities
				</button>
				<span style={{ fontSize: "0.8rem", color: "#8b949e" }}>
					rAF: {tickCount}{" "}
					{grid ? `| grid.cells=${grid.cells.h.length} seed=${seed}` : ""}
				</span>
			</header>
			<main
				style={{
					flex: "1 1 auto",
					position: "relative",
					minHeight: 0,
					display: "flex",
				}}
			>
				<div style={{ flex: "1 1 auto", position: "relative", minHeight: 0 }}>
					<MapCanvas
						onReady={(handle) => {
							canvasHandleRef.current = handle;
						}}
						onWorldMapChange={(wm, el) => {
							setWorldMap(wm);
							setCanvasEl(el);
						}}
					/>
				</div>
				{grid && (
					<div
						style={{
							flex: "0 0 auto",
							maxWidth: 260,
							padding: "0.5rem",
							overflowY: "auto",
							borderLeft: "1px solid #30363d",
						}}
					>
						<HeightmapEditor worldMap={worldMap} canvasEl={canvasEl} />
						<CellInspector worldMap={worldMap} />
						<EntityPanel worldMap={worldMap} />
					<EntityInspector />
					</div>
				)}
			</main>
			{/* Phase 5.1: timeline scrubber at the bottom of the window. */}
			<Timeline />
			<footer
				style={{
					flex: "0 0 auto",
					padding: "0.5rem 1rem",
					borderTop: "1px solid #30363d",
					background: "#161b22",
				}}
			>
				<pre
					style={{
						margin: 0,
						fontSize: "0.78rem",
						color: "#8b949e",
						whiteSpace: "pre-wrap",
						overflow: "auto",
						maxHeight: "8rem",
					}}
				>
					{result}
				</pre>
			</footer>
		</div>
	);
}

function LayerToggle({
	label,
	active,
	onClick,
}: {
	label: string;
	active: boolean;
	onClick: () => void;
}) {
	return (
		<button
			type="button"
			onClick={onClick}
			aria-pressed={active}
			style={{
				padding: "0.35rem 0.85rem",
				fontSize: "0.9rem",
				cursor: "pointer",
				border: active ? "1px solid #2f81f7" : "1px solid #30363d",
				background: active ? "#1f6feb" : "transparent",
				color: active ? "#ffffff" : "#8b949e",
			}}
		>
			{label}
		</button>
	);
}

export default App;

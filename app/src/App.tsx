import { useEffect, useRef, useState } from "react";
import { coreApi, type Grid } from "./core/api";
import type { WorldMap } from "./render/layers";
import { MapCanvas, type MapCanvasHandle } from "./render/MapCanvas";
import { useWorldgenStore } from "./state/worldgenStore";
import { CellInspector } from "./ui/CellInspector";
import { HeightmapEditor } from "./ui/HeightmapEditor";

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
	const layerEnabled = useWorldgenStore((s) => s.layerEnabled);
	const toggleLayer = useWorldgenStore((s) => s.toggleLayer);

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
					Worldforge
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
					</div>
				)}
			</main>
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

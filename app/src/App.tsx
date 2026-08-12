import { useEffect, useRef, useState } from "react";
import { coreApi, type Grid, type Mesh } from "./core/api";

function App() {
	const [result, setResult] = useState<string>("Loading WASM...");
	const [worldResult, setWorldResult] = useState<string>("");
	const [busy, setBusy] = useState(false);
	// Stopwatch counters to demonstrate the main thread is NOT blocked
	// while the worker runs the 60k generation.  Each rAF tick bumps
	// `tickCount`.  If the main thread were blocked, the tick stream
	// would freeze during generation.
	const [tickCount, setTickCount] = useState(0);
	const tickRef = useRef(0);
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
		async function testAdd() {
			try {
				const sum = await coreApi.add(2, 3);
				setResult(`add(2, 3) = ${sum} ${sum === 5 ? "✅ PASS" : "❌ FAIL"}`);
			} catch (err) {
				setResult(`Error: ${String(err)}`);
			}
		}
		async function testMesh() {
			try {
				const mesh = await coreApi.generateMesh(1000, 42);
				const pointCount = mesh.points?.length ?? 0;
				const cellCount = mesh.cells?.i?.length ? mesh.cells.i.length - 1 : 0;
				const hasVertices = mesh.vertices?.p?.length > 0;
				const pointsMatchCells = pointCount === cellCount;
				const countsMatch = pointCount === 1000 && cellCount === 1000;
				setResult(
					`generateMesh(1000, 42): points=${pointCount}, cells=${cellCount}, vertices=${mesh.vertices?.p?.length ?? 0} ` +
						`${countsMatch && pointsMatchCells && hasVertices ? "✅ PASS" : "❌ FAIL"}`,
				);
			} catch (err) {
				setResult(`Mesh Error: ${String(err)}`);
			}
		}
		async function testHeightmap() {
			try {
				const mesh = await coreApi.generateMesh(1000, 42);
				const h = await coreApi.generateHeightmap(mesh, 42);
				const len = h?.length ?? 0;
				// Validate range 0..100 and compute land fraction.
				let inRange = true;
				let land = 0;
				for (let i = 0; i < len; i++) {
					if (h[i] < 0 || h[i] > 100) inRange = false;
					if (h[i] >= 20) land++;
				}
				const frac = len > 0 ? land / len : 0;
				const rangeOk = inRange && len === 1000;
				const landOk = frac > 0.2 && frac < 0.7;
				setResult(
					`generateHeightmap(1000, 42): len=${len}, landFrac=${frac.toFixed(3)} ` +
						`${rangeOk && landOk ? "✅ PASS" : "❌ FAIL"}`,
				);
			} catch (err) {
				setResult(`Heightmap Error: ${String(err)}`);
			}
			// Second seed (7) is the adversarial-review canary: the OLD code
			// produced 0.804 land at N=1000 seed 7 (above the 0.70 band). A
			// separate check catches regression even though the default UI shows
			// seed 42 first.
			try {
				const mesh7 = await coreApi.generateMesh(1000, 7);
				const h7 = await coreApi.generateHeightmap(mesh7, 7);
				let land7 = 0;
				for (let i = 0; i < h7.length; i++) if (h7[i] >= 20) land7++;
				const frac7 = land7 / h7.length;
				const ok7 = frac7 >= 0.2 && frac7 <= 0.7;
				setResult(
					(prev: string) =>
						prev +
						`  | seed7 landFrac=${frac7.toFixed(3)} ${ok7 ? "✅ PASS" : "❌ FAIL"}`,
				);
			} catch (err) {
				setResult(
					(prev: string) => `${prev}  | seed7 Heightmap Error: ${String(err)}`,
				);
			}
		}

		async function testClimate() {
			try {
				const mesh = await coreApi.generateMesh(1000, 42);
				const h = await coreApi.generateHeightmap(mesh, 42);
				const climate = await coreApi.generateClimate(mesh, h);
				const len = climate.temp?.length ?? 0;
				const plen = climate.prec?.length ?? 0;
				// Validate ranges and latitudinal structure.
				let tempInRange = true;
				let precInRange = true;
				let landHot = false; // a high cell near a pole should be cooler than a low equatorial cell
				let maxT = -128,
					minT = 127,
					maxP = 0,
					minP = 255;
				for (let i = 0; i < len; i++) {
					const t = climate.temp[i];
					if (t < -128 || t > 127) tempInRange = false;
					maxT = Math.max(maxT, t);
					minT = Math.min(minT, t);
				}
				for (let i = 0; i < plen; i++) {
					const p = climate.prec[i];
					if (p < 0 || p > 255) precInRange = false;
					maxP = Math.max(maxP, p);
					minP = Math.min(minP, p);
				}
				const lensOk = len === 1000 && plen === 1000;
				const rangesOk = tempInRange && precInRange && lensOk;
				// Equatorial sea-level cells should be warmer than polar ones.
				const eqT = avgTempBand(mesh, h, climate.temp, 0.5);
				const poleT = avgTempBand(mesh, h, climate.temp, 0.1, true);
				landHot = eqT > poleT;
				const structureOk = landHot && maxT - minT >= 10 && maxP - minP >= 10;
				setResult(
					(prev: string) =>
						prev +
						`  | climate: len=${len}, T[${minT},${maxT}] P[${minP},${maxP}] ` +
						`eqT=${eqT.toFixed(1)} poleT=${poleT.toFixed(1)} ` +
						`${rangesOk && structureOk ? "✅ PASS" : "❌ FAIL"}`,
				);
			} catch (err) {
				setResult((prev: string) => `${prev}  | climate Error: ${String(err)}`);
			}
		}

		// Average temperature of sea-level cells in a band of the map.
		// `fraction` selects a vertical slice; if `poles` is true, take the
		// top/bottom `fraction` (near the map edges, = high |latitude|),
		// otherwise the central `fraction` (near the equator).
		function avgTempBand(
			mesh: Mesh,
			h: Uint8Array,
			temp: Int8Array,
			fraction: number,
			poles = false,
		): number {
			const pts = mesh.points as [number, number][];
			const H = mesh.world_h as number;
			let sum = 0,
				n = 0;
			for (let i = 0; i < pts.length; i++) {
				if (h[i] >= 20) continue; // sea-level (water) only
				const y = pts[i][1];
				const rel = y / H; // 0 at top (north) .. 1 at bottom (south)
				const inBand = poles
					? rel < fraction || rel > 1 - fraction
					: Math.abs(rel - 0.5) < fraction / 2;
				if (inBand) {
					sum += temp[i];
					n++;
				}
			}
			return n > 0 ? sum / n : 0;
		}

		async function testBiomes() {
			try {
				const mesh = await coreApi.generateMesh(1000, 42);
				const h = await coreApi.generateHeightmap(mesh, 42);
				const climate = await coreApi.generateClimate(mesh, h);
				const biome = await coreApi.generateBiomes(mesh, climate, h);
				const len = biome?.length ?? 0;
				// Every water cell (h < 20) must be Marine (0).
				let waterOk = true;
				let landOk = true;
				let land = 0;
				for (let i = 0; i < len; i++) {
					if (h[i] < 20) {
						if (biome[i] !== 0) waterOk = false;
					} else {
						land++;
						if (biome[i] < 1 || biome[i] > 12) landOk = false;
					}
				}
				const lensOk = len === 1000;
				const rangesOk = waterOk && landOk && lensOk && land > 0;
				setResult(
					(prev: string) =>
						prev +
						`  | biomes: len=${len} land=${land} waterMarine=${waterOk} ` +
						`landValid=${landOk} ${rangesOk ? "✅ PASS" : "❌ FAIL"}`,
				);
			} catch (err) {
				setResult((prev: string) => `${prev}  | biomes Error: ${String(err)}`);
			}
		}

		testAdd();
		testMesh();
		testHeightmap();
		testClimate();
		testBiomes();
	}, []);

	// Step 2.1: fire `generateWorld(42, 60000)` on the worker thread.
	// The UI stays responsive while the WASM runs off-main-thread; the
	// tick counter keeps incrementing through the await.
	async function runGenerateWorld() {
		if (busy) return;
		setBusy(true);
		setWorldResult("⏳ Generating 60k world on worker thread…");
		const tickBefore = tickRef.current;
		const t0 = performance.now();
		try {
			const grid: Grid = await coreApi.generateWorld(42, 60_000, {});
			const t1 = performance.now();
			const tickAfter = tickRef.current;
			const n = grid.cells.h.length;
			let land = 0;
			let waterMarine = true;
			let landValid = true;
			const hist = new Map<number, number>();
			for (let i = 0; i < n; i++) {
				const b = grid.cells.biome[i];
				hist.set(b, (hist.get(b) ?? 0) + 1);
				if (grid.cells.h[i] >= 20) {
					land++;
					if (b < 1 || b > 12) landValid = false;
				} else if (b !== 0) {
					waterMarine = false;
				}
			}
			const fieldsOk =
				n === 60_000 &&
				grid.cells.temp.length === n &&
				grid.cells.prec.length === n &&
				grid.cells.biome.length === n &&
				grid.mesh.points.length === n;
			const gatePass = fieldsOk && t1 - t0 < 2000 && waterMarine && landValid;
			const histStr = [...hist.entries()]
				.sort((a, b) => a[0] - b[0])
				.map(([id, c]) => `${id}:${c}`)
				.join(" ");
			setWorldResult(
				`generateWorld(42, 60000) = ${(t1 - t0).toFixed(0)}ms ` +
					`${gatePass ? "PASS (<2s gate)" : "FAIL"}\n` +
					`  points=${grid.mesh.points.length} verts=${grid.mesh.vertices.p.length} ` +
					`h/temp/prec/biome=${n} land=${land} (${((land / n) * 100).toFixed(1)}%)\n` +
					`  waterMarine=${waterMarine} landValid=${landValid}\n` +
					`  biome hist: ${histStr}\n` +
					`  rAF ticks during gen: ${tickAfter - tickBefore} (UI ${tickAfter - tickBefore > 30 ? "responsive" : "blocked?"})`,
			);
		} catch (err) {
			setWorldResult(`Error: ${String(err)}`);
		} finally {
			setBusy(false);
		}
	}

	return (
		<div style={{ padding: "2rem", fontFamily: "system-ui, sans-serif" }}>
			<h1>Worldforge — Phase 2.1 Verification</h1>
			<p style={{ fontSize: "1.25rem", fontWeight: "bold" }}>{result}</p>
			<hr style={{ margin: "1.5rem 0" }} />
			<h2>Step 2.1 — Worker bridge: generateWorld on the worker thread</h2>
			<p>
				<button
					type="button"
					onClick={runGenerateWorld}
					disabled={busy}
					style={{
						padding: "0.5rem 1rem",
						fontSize: "1rem",
						cursor: busy ? "wait" : "pointer",
					}}
				>
					{busy ? "Generating…" : "Generate 60k world"}
				</button>{" "}
				<span style={{ fontSize: "0.85rem", color: "#555" }}>
					rAF ticks: {tickCount} (main-thread liveness indicator)
				</span>
			</p>
			<pre
				style={{
					background: "#f4f4f4",
					padding: "0.75rem",
					borderRadius: 6,
					whiteSpace: "pre-wrap",
					fontSize: "0.85rem",
					minHeight: "2rem",
				}}
			>
				{worldResult ||
					"Click the button to generate a 60k world on the worker thread."}
			</pre>
			<hr style={{ margin: "1.5rem 0" }} />
			<p>
				<strong>Stack:</strong> Vite + React 19 + TypeScript + PixiJS v8 +
				Rust→WASM (wasm-pack) + Web Worker
			</p>
			<p>
				<strong>Phase 0 Gate:</strong> <code>add(2,3) === 5</code> rendered via
				WASM worker bridge.
			</p>
		</div>
	);
}

export default App;

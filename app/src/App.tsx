import { useEffect, useState } from "react";
import { coreApi } from "./core/api";

function App() {
	const [result, setResult] = useState<string>("Loading WASM...");

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
				setResult(
					`generateMesh(1000, 42): points=${pointCount}, cells=${cellCount}, vertices=${mesh.vertices?.p?.length ?? 0} ` +
						`${pointCount === 1000 && cellCount === 1000 && hasVertices ? "✅ PASS" : "❌ FAIL"}`,
				);
			} catch (err) {
				setResult(`Mesh Error: ${String(err)}`);
			}
		}
		testAdd();
		testMesh();
	}, []);

	return (
		<div style={{ padding: "2rem", fontFamily: "system-ui, sans-serif" }}>
			<h1>Worldforge — Phase 0 Verification</h1>
			<p style={{ fontSize: "1.25rem", fontWeight: "bold" }}>{result}</p>
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

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
		testAdd();
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

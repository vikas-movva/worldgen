// Core worker — runs the Rust WASM module off the main thread.
// Step 0.1: exports `add(a,b)` for verification.
// Step 1.1: exports `generate_mesh(cell_count, seed)` → Mesh.
// Later phases: generate_world, project_world, edit_heightmap, recompute_dependents, generate_timeline.

import init, { add, generate_mesh } from "../core/worldgen_core.js";

let wasmReady: Promise<{ memory: WebAssembly.Memory }> | null = null;

function ensureWasm(): Promise<void> {
	if (!wasmReady) {
		wasmReady = init();
	}
	return wasmReady.then(() => undefined);
}

type WorkerRequest =
	| { kind: "add"; reqId: number; a: number; b: number }
	| { kind: "generate_mesh"; reqId: number; cellCount: number; seed: number };

// The Mesh shape (serialized from Rust via serde-wasm-bindgen).
type Mesh = {
	points: [number, number][];
	cells: {
		v: number[];
		c: number[];
		i: number[];
		b: number[];
	};
	vertices: {
		p: [number, number][];
	};
};

type WorkerResponse =
	| { kind: "add"; reqId: number; ok: true; result: number }
	| { kind: "generate_mesh"; reqId: number; ok: true; result: Mesh }
	| { kind: "error"; reqId: number; ok: false; message: string };

let nextReqId = 1;

self.onmessage = async (e: MessageEvent<WorkerRequest>) => {
	const req = e.data;
	const reqId = req.reqId ?? nextReqId++;

	const send = (res: WorkerResponse) => {
		self.postMessage(res);
	};

	try {
		await ensureWasm();

		if (req.kind === "add") {
			const result = add(req.a, req.b);
			send({ kind: "add", reqId, ok: true, result });
		} else if (req.kind === "generate_mesh") {
			const result = generate_mesh(req.cellCount, req.seed);
			send({ kind: "generate_mesh", reqId, ok: true, result });
		} else {
			const unknownReq = req as { kind: string };
			send({
				kind: "error",
				reqId,
				ok: false,
				message: `Unknown request kind: ${unknownReq.kind}`,
			});
		}
	} catch (err) {
		send({ kind: "error", reqId, ok: false, message: String(err) });
	}
};

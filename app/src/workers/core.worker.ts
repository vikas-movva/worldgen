// Core worker — runs the Rust WASM module off the main thread.
// Step 0.1: exports `add(a,b)` for verification.
// Later phases: generate_world, project_world, edit_heightmap, recompute_dependents, generate_timeline.

import init, { add } from "../core/worldforge_core.js";

let wasmReady: Promise<{ memory: WebAssembly.Memory }> | null = null;

function ensureWasm(): Promise<void> {
	if (!wasmReady) {
		wasmReady = init();
	}
	return wasmReady.then(() => undefined);
}

type WorkerRequest = { kind: "add"; reqId: number; a: number; b: number };

type WorkerResponse =
	| { kind: "add"; reqId: number; ok: true; result: number }
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

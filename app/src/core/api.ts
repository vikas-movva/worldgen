// Typed wrapper over the core worker.
// Step 0.1: only `add(a,b)` exposed.
// Later: generateWorld, projectWorld, editHeightmap, recomputeDependents, generateTimeline.

import CoreWorker from "../workers/core.worker.ts?worker";

type Res<T> = { reqId: number; ok: true; result: T } | { reqId: number; ok: false; message: string };

const worker = new CoreWorker() as unknown as Worker;

const pending = new Map<number, { resolve: (v: any) => void; reject: (e: Error) => void }>();

worker.onmessage = (e: MessageEvent<Res<any>>) => {
  const res = e.data;
  const entry = pending.get(res.reqId);
  if (!entry) return;
  if (res.ok) entry.resolve(res.result);
  else entry.reject(new Error(res.message));
  pending.delete(res.reqId);
};

function call<T, R>(kind: string, payload: T): Promise<R> {
  const reqId = Date.now() + Math.random(); // unique enough for this phase
  return new Promise((resolve, reject) => {
    pending.set(reqId, { resolve, reject });
    worker.postMessage({ kind, reqId, ...payload } as any);
  });
}

export const coreApi = {
  /** Trivial export to verify the WASM ↔ JS bridge works end-to-end. */
  add(a: number, b: number): Promise<number> {
    return call("add", { a, b });
  },

  // Placeholders for future phases:
  // generateWorld(seed, cellCount, opts): Promise<Grid>;
  // projectWorld(pack, timeline, year): Promise<WorldAt>;
  // editHeightmap(grid, ops): Promise<Grid>;
  // recomputeDependents(grid, opts): Promise<DependentResult>;
  // generateTimeline(pack, seed, params): Promise<Event[]>;
};
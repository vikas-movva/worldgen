// Step 2.1 unit tests — the worker bridge (`src/core/api.ts`).
//
// These exercise the request/response contract WITHOUT a real Web Worker or
// the WASM module: a fake `Worker` captures the `postMessage` payload and we
// drive `onmessage` by hand to simulate the worker's reply. This pins the
// Step 2.1 verification contract:
//   - `generateWorld` returns a `Grid` and never blocks (it's a Promise).
//   - seed / cellCount are clamped at the JS boundary (adversarial C1/M6).
//   - the exact `{ kind, reqId, ...payload }` wire message is emitted.
//   - resolve routes `ok:true` results; reject routes `ok:false` errors.
//   - reqIds are unique (no cross-talk between concurrent calls).
//   - `generateBiomes` converts the `temp`/`prec` Int8/Uint8Arrays to plain
//     arrays on the wire (the WASM climate step expects plain arrays).

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  __setWorkerForTest,
  clampCellCount,
  clampSeed,
  coreApi,
  type Grid,
  type EditOp,
  type EditMode,
} from "./api";

// ---- fake worker harness -------------------------------------------------

type AnyReq = { kind: string; reqId: number; [k: string]: unknown };

class FakeWorker {
  public lastMessage: AnyReq | null = null;
  public onmessage: ((e: MessageEvent) => void) | null = null;

  postMessage(msg: AnyReq) {
    this.lastMessage = msg;
  }

  /** Simulate the worker replying with a success payload. */
  reply(result: unknown) {
    const req = this.lastMessage!;
    const evt = {
      data: { kind: req.kind, reqId: req.reqId, ok: true, result },
    } as unknown as MessageEvent;
    this.onmessage?.(evt);
  }

  /** Simulate the worker replying with an error. */
  replyError(message: string) {
    const req = this.lastMessage!;
    const evt = {
      data: { kind: req.kind, reqId: req.reqId, ok: false, message },
    } as unknown as MessageEvent;
    this.onmessage?.(evt);
  }
}

let fake: FakeWorker;

beforeEach(() => {
  fake = new FakeWorker();
  __setWorkerForTest(fake as unknown as Worker);
});

afterEach(() => {
  __setWorkerForTest(null);
});

// ---- clamp helpers (boundary defense, adversarial C1 / M6) ---------------

describe("clampSeed", () => {
  it("passes through a normal in-range seed unchanged and unsigned", () => {
    expect(clampSeed(42)).toBe(42);
    expect(clampSeed(0)).toBe(0);
    expect(clampSeed(0xffffffff)).toBe(0xffffffff);
  });

  it("floors fractions instead of rounding", () => {
    expect(clampSeed(42.9)).toBe(42);
  });

  it("clamps negatives to 0", () => {
    expect(clampSeed(-5)).toBe(0);
  });

  it("clamps overflow past u32::MAX to u32::MAX (no silent wrap)", () => {
    expect(clampSeed(0x1_0000_0000)).toBe(0xffffffff); // 2^32
    expect(clampSeed(Number.MAX_SAFE_INTEGER)).toBe(0xffffffff);
  });

  it("treats non-finite / NaN as 0 (deterministic, no panic)", () => {
    expect(clampSeed(Number.NaN)).toBe(0);
    expect(clampSeed(Number.POSITIVE_INFINITY)).toBe(0);
    expect(clampSeed(Number.NEGATIVE_INFINITY)).toBe(0);
  });
});

describe("clampCellCount", () => {
  it("passes through an in-range count", () => {
    expect(clampCellCount(1000)).toBe(1000);
    expect(clampCellCount(60_000)).toBe(60_000);
  });

  it("floors fractions", () => {
    expect(clampCellCount(999.9)).toBe(999);
  });

  it("clamps below 1 up to the spade minimum of 4", () => {
    expect(clampCellCount(0)).toBe(4);
    expect(clampCellCount(-50)).toBe(4);
  });

  it("clamps above the 60k MVP cap", () => {
    expect(clampCellCount(100_000)).toBe(60_000);
    expect(clampCellCount(Number.MAX_SAFE_INTEGER)).toBe(60_000);
  });

  it("treats non-finite as 0 -> 4", () => {
    expect(clampCellCount(Number.NaN)).toBe(4);
  });
});

// ---- the typed generateWorld promise -------------------------------------

describe("coreApi.generateWorld", () => {
  it("returns a Promise (off-main-thread contract, never blocks)", () => {
    const p = coreApi.generateWorld(42, 60000);
    expect(p).toBeInstanceOf(Promise);
  });

  it("emits the exact 'generate_world' wire message with clamped args", () => {
    const opts = { map_size: 100 };
    coreApi.generateWorld(42, 60000, opts);
    expect(fake.lastMessage).toMatchObject({
      kind: "generate_world",
      seed: 42,
      cellCount: 60000,
      opts,
    });
    // reqId must be a positive integer and present
    expect(typeof fake.lastMessage!.reqId).toBe("number");
    expect(fake.lastMessage!.reqId).toBeGreaterThan(0);
  });

  it("clamps out-of-range seed and cellCount on the wire", () => {
    coreApi.generateWorld(-5, 1_000_000);
    expect(fake.lastMessage).toMatchObject({
      kind: "generate_world",
      seed: 0, // negative clamped
      cellCount: 60000, // over cap clamped
    });
  });

  it("resolves with the worker's Grid result", async () => {
    const p = coreApi.generateWorld(42, 1000);
    const expected: Grid = makeFakeGrid(1000, 42);
    fake.reply(expected);
    const result = await p;
    expect(result).toBe(expected);
  });

  it("rejects with an Error when the worker reports failure", async () => {
    const p = coreApi.generateWorld(1, 100);
    fake.replyError("wasm panicked: capacity overflow");
    await expect(p).rejects.toThrow(/capacity overflow/);
  });

  it("routes two concurrent calls to their own reqIds (no cross-talk)", async () => {
    const p1 = coreApi.generateWorld(1, 100);
    const firstMsg = fake.lastMessage!;
    const p2 = coreApi.generateWorld(2, 200);
    const secondMsg = fake.lastMessage!;
    expect(firstMsg.reqId).not.toBe(secondMsg.reqId);

    const g1 = makeFakeGrid(100, 1);

    // Deliver replies out of order to prove reqId routing.
    fake.replyError("boom on second"); // satisfies p2's message (reqId = second)
    fake.lastMessage = firstMsg; // point the fake at p1 for the next reply
    fake.reply(g1); // satisfies p1

    await expect(p2).rejects.toThrow("boom on second");
    await expect(p1).resolves.toBe(g1);
  });
});

// ---- the other bridge methods emit the right wire messages ---------------

describe("coreApi request routing", () => {
  it("generateMesh clamps cellCount and forwards seed", () => {
    coreApi.generateMesh(60000, 7);
    expect(fake.lastMessage).toMatchObject({
      kind: "generate_mesh",
      cellCount: 60000,
      seed: 7,
    });
  });

  it("add emits the 'add' message and resolves the number", async () => {
    const p = coreApi.add(2, 3);
    expect(fake.lastMessage).toMatchObject({ kind: "add", a: 2, b: 3 });
    fake.reply(5);
    await expect(p).resolves.toBe(5);
  });

  it("generateHeightmap forwards mesh + clamped seed", () => {
    const mesh = {} as never;
    coreApi.generateHeightmap(mesh, 99);
    expect(fake.lastMessage).toMatchObject({
      kind: "generate_heightmap",
      mesh,
      seed: 99,
    });
  });

  it("generateBiomes converts temp/prec to plain arrays on the wire", () => {
    const mesh = {} as never;
    const climate = {
      temp: Int8Array.from([-5, 0, 20]),
      prec: Uint8Array.from([0, 50, 200]),
    };
    coreApi.generateBiomes(mesh, climate, Uint8Array.from([10, 20, 30]));
    const msg = fake.lastMessage as AnyReq;
    expect(msg.kind).toBe("generate_biomes");
    // The WASM climate entry expects `temp`/`prec` as plain number[] (it is
    // not wired to receive TypedArrays), so api.ts must serialize them.
    expect(msg.climate).toEqual({
      temp: [-5, 0, 20],
      prec: [0, 50, 200],
    });
  });

  it("generateWorld defaults opts to {} when omitted", () => {
    coreApi.generateWorld(1, 10);
    expect((fake.lastMessage as AnyReq).opts).toEqual({});
  });
});

// ---- editHeightmap (Step 2.5.1) ------------------------------------------

describe("coreApi.editHeightmap", () => {
  it("returns a Promise (off-main-thread contract)", () => {
    const grid = makeFakeGrid(1000, 42);
    const ops: EditOp[] = [
      {
        mode: "Raise",
        center_cell: 100,
        target_cell: 0,
        radius: 500.0,
        strength: 0.5,
        cells: [],
      },
    ];
    const p = coreApi.editHeightmap(grid, ops);
    expect(p).toBeInstanceOf(Promise);
  });

  it("emits the exact 'edit_heightmap' wire message with grid + ops", () => {
    const grid = makeFakeGrid(1000, 42);
    const ops: EditOp[] = [
      {
        mode: "Raise",
        center_cell: 100,
        target_cell: 0,
        radius: 500.0,
        strength: 0.5,
        cells: [],
      },
      {
        mode: "Lower",
        center_cell: 200,
        target_cell: 0,
        radius: 300.0,
        strength: 0.3,
        cells: [200, 201, 202],
      },
    ];
    coreApi.editHeightmap(grid, ops);
    expect(fake.lastMessage).toMatchObject({
      kind: "edit_heightmap",
      grid,
      ops,
    });
    expect(typeof fake.lastMessage!.reqId).toBe("number");
    expect(fake.lastMessage!.reqId).toBeGreaterThan(0);
  });

  it("resolves with the worker's updated Grid result", async () => {
    const grid = makeFakeGrid(1000, 42);
    const ops: EditOp[] = [
      {
        mode: "Raise",
        center_cell: 100,
        target_cell: 0,
        radius: 500.0,
        strength: 0.5,
        cells: [],
      },
    ];
    const expected = makeFakeGrid(1000, 42);
    expected.cells.h[100] = 75; // simulate raise
    const p = coreApi.editHeightmap(grid, ops);
    fake.reply(expected);
    const result = await p;
    expect(result).toBe(expected);
    // Verify the returned grid has the modified height
    expect(result.cells.h[100]).toBe(75);
  });

  it("rejects with an Error when the worker reports failure", async () => {
    const grid = makeFakeGrid(1000, 42);
    const ops: EditOp[] = [
      {
        mode: "Raise",
        center_cell: 100,
        target_cell: 0,
        radius: 500.0,
        strength: 0.5,
        cells: [],
      },
    ];
    const p = coreApi.editHeightmap(grid, ops);
    fake.replyError("wasm panicked: edit_heightmap failed");
    await expect(p).rejects.toThrow(/edit_heightmap failed/);
  });

  it("routes two concurrent calls to their own reqIds (no cross-talk)", async () => {
    const grid1 = makeFakeGrid(1000, 1);
    const grid2 = makeFakeGrid(1000, 2);
    const ops1: EditOp[] = [
      {
        mode: "Raise",
        center_cell: 100,
        target_cell: 0,
        radius: 500.0,
        strength: 0.5,
        cells: [],
      },
    ];
    const ops2: EditOp[] = [
      {
        mode: "Lower",
        center_cell: 200,
        target_cell: 0,
        radius: 300.0,
        strength: 0.3,
        cells: [],
      },
    ];
    const p1 = coreApi.editHeightmap(grid1, ops1);
    const firstMsg = fake.lastMessage!;
    const p2 = coreApi.editHeightmap(grid2, ops2);
    const secondMsg = fake.lastMessage!;
    expect(firstMsg.reqId).not.toBe(secondMsg.reqId);

    const g1 = makeFakeGrid(1000, 1);
    g1.cells.h[100] = 80;

    // Deliver replies out of order to prove reqId routing.
    fake.replyError("boom on second"); // satisfies p2's message (reqId = second)
    fake.lastMessage = firstMsg; // point the fake at p1 for the next reply
    fake.reply(g1); // satisfies p1

    await expect(p2).rejects.toThrow("boom on second");
    await expect(p1).resolves.toBe(g1);
  });

  it("forwards all EditMode variants on the wire", () => {
    const modes: EditMode[] = [
      "Raise",
      "Lower",
      "Flatten",
      "Smooth",
      "Range",
      "Trough",
      "Strait",
      "Mask",
      "Invert",
      "Add",
      "Multiply",
    ];
    for (const mode of modes) {
      const grid = makeFakeGrid(100, 42);
      const ops: EditOp[] = [
        {
          mode,
          center_cell: 10,
          target_cell: 50,
          radius: 100.0,
          strength: 0.5,
          cells: [10, 11, 12],
        },
      ];
      coreApi.editHeightmap(grid, ops);
      expect(fake.lastMessage).toMatchObject({
        kind: "edit_heightmap",
        ops: [
          {
            mode,
            center_cell: 10,
            target_cell: 50,
            radius: 100.0,
            strength: 0.5,
            cells: [10, 11, 12],
          },
        ],
      });
    }
  });
});

// ---- recomputeTempBiomeLocal (Step 2.5.2) --------------------------------

describe("coreApi.recomputeTempBiomeLocal", () => {
  it("sends the exact { kind, reqId, grid, cellIds, opts } wire message", () => {
    const grid = makeFakeGrid(1000, 42);
    const cellIds = [10, 20, 30];
    coreApi.recomputeTempBiomeLocal(grid, cellIds);
    expect(fake.lastMessage).toMatchObject({
      kind: "recompute_temp_biome_local",
      grid,
      cellIds: [10, 20, 30],
      opts: {},
    });
    expect(typeof fake.lastMessage!.reqId).toBe("number");
    expect(fake.lastMessage!.reqId).toBeGreaterThan(0);
  });

  it("defaults opts to {} when not provided", () => {
    const grid = makeFakeGrid(100, 1);
    coreApi.recomputeTempBiomeLocal(grid, [0, 1]);
    expect(fake.lastMessage).toMatchObject({
      opts: {},
    });
  });

  it("forwards custom opts on the wire", () => {
    const grid = makeFakeGrid(100, 1);
    const opts = { temperature_equator: 30, height_exponent: 2.5 };
    coreApi.recomputeTempBiomeLocal(grid, [5, 6], opts);
    expect(fake.lastMessage).toMatchObject({ opts });
  });

  it("resolves with { temp: Int8Array, biome: Uint8Array } from worker", async () => {
    const grid = makeFakeGrid(100, 42);
    const result = {
      temp: new Int8Array([10, 15, 20]),
      biome: new Uint8Array([3, 6, 9]),
    };
    const p = coreApi.recomputeTempBiomeLocal(grid, [0, 1, 2]);
    fake.reply(result);
    const res = await p;
    expect(res.temp).toBeInstanceOf(Int8Array);
    expect(res.biome).toBeInstanceOf(Uint8Array);
    expect(Array.from(res.temp)).toEqual([10, 15, 20]);
    expect(Array.from(res.biome)).toEqual([3, 6, 9]);
  });

  it("rejects with an Error when the worker reports failure", async () => {
    const grid = makeFakeGrid(100, 42);
    const p = coreApi.recomputeTempBiomeLocal(grid, [0]);
    fake.replyError("wasm panic: bad grid");
    await expect(p).rejects.toThrow(/bad grid/);
  });

  it("routes two concurrent calls to their own reqIds (no cross-talk)", async () => {
    const g1 = makeFakeGrid(100, 1);
    const g2 = makeFakeGrid(100, 2);
    const p1 = coreApi.recomputeTempBiomeLocal(g1, [1, 2]);
    const firstMsg = fake.lastMessage!;
    const p2 = coreApi.recomputeTempBiomeLocal(g2, [3, 4]);
    const secondMsg = fake.lastMessage!;
    expect(firstMsg.reqId).not.toBe(secondMsg.reqId);

    // Deliver replies out of order.
    fake.reply({ temp: new Int8Array([99, 98]), biome: new Uint8Array([7, 8]) });
    fake.lastMessage = firstMsg;
    fake.reply({ temp: new Int8Array([1, 2]), biome: new Uint8Array([3, 4]) });

    const r2 = await p2;
    const r1 = await p1;
    expect(Array.from(r2.temp)).toEqual([99, 98]);
    expect(Array.from(r1.temp)).toEqual([1, 2]);
  });
});

// ---- recomputeDependents (Step 2.5.3) -------------------------------------

describe("coreApi.recomputeDependents", () => {
  it("sends the exact { kind, reqId, grid, opts } wire message", () => {
    const grid = makeFakeGrid(1000, 42);
    coreApi.recomputeDependents(grid);
    expect(fake.lastMessage).toMatchObject({
      kind: "recompute_dependents",
      grid,
      opts: {},
    });
    expect(typeof fake.lastMessage!.reqId).toBe("number");
    expect(fake.lastMessage!.reqId).toBeGreaterThan(0);
  });

  it("defaults opts to {} when not provided", () => {
    const grid = makeFakeGrid(100, 1);
    coreApi.recomputeDependents(grid);
    expect(fake.lastMessage).toMatchObject({ opts: {} });
  });

  it("forwards custom opts on the wire", () => {
    const grid = makeFakeGrid(100, 1);
    const opts = { temperature_equator: 30, height_exponent: 2.5 };
    coreApi.recomputeDependents(grid, opts);
    expect(fake.lastMessage).toMatchObject({ opts });
  });

  it("resolves with a DependentResult from the worker", async () => {
    const grid = makeFakeGrid(100, 42);
    const result = {
      temp: new Int8Array(100),
      prec: new Uint8Array(100),
      biome: new Uint8Array(100),
      removed_burgs: [],
      dissolved_states: [],
      rivers: [{ id: 1, source: 5, mouth: 10, discharge: 42, cells: [5, 6, 7, 8, 9, 10], points: [[0, 0], [1, 1]] }],
      lakes: [],
    };
    const p = coreApi.recomputeDependents(grid);
    fake.reply(result);
    const res = await p;
    expect(res.temp.length).toBe(100);
    expect(res.prec.length).toBe(100);
    expect(res.biome.length).toBe(100);
    expect(res.rivers.length).toBe(1);
    expect(res.rivers[0].id).toBe(1);
    expect(res.rivers[0].discharge).toBe(42);
    expect(res.lakes.length).toBe(0);
    expect(res.removed_burgs).toEqual([]);
    expect(res.dissolved_states).toEqual([]);
  });

  it("rejects with an Error when the worker reports failure", async () => {
    const grid = makeFakeGrid(100, 42);
    const p = coreApi.recomputeDependents(grid);
    fake.replyError("wasm panic: bad grid");
    await expect(p).rejects.toThrow(/bad grid/);
  });

  it("routes two concurrent calls to their own reqIds (no cross-talk)", async () => {
    const g1 = makeFakeGrid(100, 1);
    const g2 = makeFakeGrid(100, 2);
    const p1 = coreApi.recomputeDependents(g1);
    const firstMsg = fake.lastMessage!;
    const p2 = coreApi.recomputeDependents(g2);
    const secondMsg = fake.lastMessage!;
    expect(firstMsg.reqId).not.toBe(secondMsg.reqId);

    fake.reply({ temp: new Int8Array(100), prec: new Uint8Array(100), biome: new Uint8Array(100), removed_burgs: [], dissolved_states: [], rivers: [], lakes: [] });
    fake.lastMessage = firstMsg;
    fake.reply({ temp: new Int8Array(100), prec: new Uint8Array(100), biome: new Uint8Array(100), removed_burgs: [3], dissolved_states: [], rivers: [], lakes: [] });

    const r2 = await p2;
    const r1 = await p1;
    // First reply (reqId=2, to p2): removed_burgs = []
    // Second reply (reqId=1, to p1): removed_burgs = [3]
    expect(r2.removed_burgs).toEqual([]);
    expect(r1.removed_burgs).toEqual([3]);
  });
});

// ---- helpers -------------------------------------------------------------

function makeFakeGrid(n: number, seed: number): Grid {
  return {
    seed,
    mesh: {
      points: Array.from({ length: n }, () => [0, 0] as [number, number]),
      cells: { v: [], c: [], i: [], b: [], spacing: [], cells_x: 0, cells_y: 0 },
      vertices: { p: [] },
      world_w: 10000,
      world_h: 8000,
    },
    cells: {
      h: new Array(n).fill(0),
      temp: new Array(n).fill(0),
      prec: new Array(n).fill(0),
      biome: new Array(n).fill(0),
    },
  };
}

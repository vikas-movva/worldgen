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
  spliceDependentResult,
  type DependentResult,
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
    const p = coreApi.editHeightmap(ops, grid);
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
    coreApi.editHeightmap(ops, grid);
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
    const p = coreApi.editHeightmap(ops, grid);
    fake.reply(expected);
    const result = (await p) as Grid;
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
    const p = coreApi.editHeightmap(ops, grid);
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
    const p1 = coreApi.editHeightmap(ops1, grid1);
    const firstMsg = fake.lastMessage!;
    const p2 = coreApi.editHeightmap(ops2, grid2);
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
      coreApi.editHeightmap(ops, grid);
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
    coreApi.recomputeTempBiomeLocal(cellIds, undefined, grid);
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
    coreApi.recomputeTempBiomeLocal([0, 1], undefined, grid);
    expect(fake.lastMessage).toMatchObject({
      opts: {},
    });
  });

  it("forwards custom opts on the wire", () => {
    const grid = makeFakeGrid(100, 1);
    const opts = { temperature_equator: 30, height_exponent: 2.5 };
    coreApi.recomputeTempBiomeLocal([5, 6], opts, grid);
    expect(fake.lastMessage).toMatchObject({ opts });
  });

  it("resolves with { temp: Int8Array, biome: Uint8Array } from worker", async () => {
    const grid = makeFakeGrid(100, 42);
    const result = {
      temp: new Int8Array([10, 15, 20]),
      biome: new Uint8Array([3, 6, 9]),
    };
    const p = coreApi.recomputeTempBiomeLocal([0, 1, 2], undefined, grid);
    fake.reply(result);
    const res = await p;
    expect(res.temp).toBeInstanceOf(Int8Array);
    expect(res.biome).toBeInstanceOf(Uint8Array);
    expect(Array.from(res.temp)).toEqual([10, 15, 20]);
    expect(Array.from(res.biome)).toEqual([3, 6, 9]);
  });

  it("rejects with an Error when the worker reports failure", async () => {
    const grid = makeFakeGrid(100, 42);
    const p = coreApi.recomputeTempBiomeLocal([0], undefined, grid);
    fake.replyError("wasm panic: bad grid");
    await expect(p).rejects.toThrow(/bad grid/);
  });

  it("routes two concurrent calls to their own reqIds (no cross-talk)", async () => {
    const g1 = makeFakeGrid(100, 1);
    const g2 = makeFakeGrid(100, 2);
    const p1 = coreApi.recomputeTempBiomeLocal([1, 2], undefined, g1);
    const firstMsg = fake.lastMessage!;
    const p2 = coreApi.recomputeTempBiomeLocal([3, 4], undefined, g2);
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
    coreApi.recomputeDependents(undefined, grid);
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
    coreApi.recomputeDependents(undefined, grid);
    expect(fake.lastMessage).toMatchObject({ opts: {} });
  });

  it("forwards custom opts on the wire", () => {
    const grid = makeFakeGrid(100, 1);
    const opts = { temperature_equator: 30, height_exponent: 2.5 };
    coreApi.recomputeDependents(opts, grid);
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
    const p = coreApi.recomputeDependents(undefined, grid);
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
    const p = coreApi.recomputeDependents(undefined, grid);
    fake.replyError("wasm panic: bad grid");
    await expect(p).rejects.toThrow(/bad grid/);
  });

  it("routes two concurrent calls to their own reqIds (no cross-talk)", async () => {
    const g1 = makeFakeGrid(100, 1);
    const g2 = makeFakeGrid(100, 2);
    const p1 = coreApi.recomputeDependents(undefined, g1);
    const firstMsg = fake.lastMessage!;
    const p2 = coreApi.recomputeDependents(undefined, g2);
    const secondMsg = fake.lastMessage!;
    expect(firstMsg.reqId).not.toBe(secondMsg.reqId);

    fake.reply({ temp: new Int8Array(100), prec: new Uint8Array(100), biome: new Uint8Array(100), state: new Int32Array(100).fill(-1), province: new Int32Array(100).fill(-1), burg: new Int16Array(100).fill(-1), fl: new Uint16Array(100), r: new Uint16Array(100), conf: new Uint16Array(100), coastline: new Uint8Array(100), removed_burgs: [], dissolved_states: [], rivers: [], lakes: [] });
    fake.lastMessage = firstMsg;
    fake.reply({ temp: new Int8Array(100), prec: new Uint8Array(100), biome: new Uint8Array(100), state: new Int32Array(100).fill(-1), province: new Int32Array(100).fill(-1), burg: new Int16Array(100).fill(-1), fl: new Uint16Array(100), r: new Uint16Array(100), conf: new Uint16Array(100), coastline: new Uint8Array(100), removed_burgs: ["Helms Deep"], dissolved_states: [], rivers: [], lakes: [] });

    const r2 = await p2;
    const r1 = await p1;
    // First reply (reqId=2, to p2): removed_burgs = []
    // Second reply (reqId=1, to p1): removed_burgs = ["Helms Deep"]
    expect(r2.removed_burgs).toEqual([]);
    expect(r1.removed_burgs).toEqual(["Helms Deep"]);
  });
});

// ---- grid-form + climate/biome bridge methods (Step 1.2 / 1.3 / 1.4) -----
// These four `coreApi` methods were previously untested; a drift in their
// wire contract (key rename, dropped clamp, wrong payload) would pass CI and
// break the app. They follow the same fake-worker pattern as above.

describe("coreApi.buildGridWithHeightmap", () => {
  it("emits the 'build_grid_with_heightmap' wire message with clamped seed", () => {
    const mesh = {} as never;
    coreApi.buildGridWithHeightmap(mesh, -3);
    expect(fake.lastMessage).toMatchObject({
      kind: "build_grid_with_heightmap",
      mesh,
      seed: 0, // negative clamped
    });
    expect(typeof fake.lastMessage!.reqId).toBe("number");
    expect(fake.lastMessage!.reqId).toBeGreaterThan(0);
  });

  it("resolves with the worker's Grid result", async () => {
    const mesh = {} as never;
    const p = coreApi.buildGridWithHeightmap(mesh, 42);
    const expected = makeFakeGrid(1000, 42);
    fake.reply(expected);
    const result = await p;
    expect(result).toBe(expected);
  });

  it("rejects with an Error when the worker reports failure", async () => {
    const mesh = {} as never;
    const p = coreApi.buildGridWithHeightmap(mesh, 1);
    fake.replyError("wasm panic: bad mesh");
    await expect(p).rejects.toThrow(/bad mesh/);
  });
});

describe("coreApi.generateClimate", () => {
  it("emits the 'generate_climate' wire message with mesh + heightmap + opts", () => {
    const mesh = {} as never;
    const heightmap = new Uint8Array([0, 20, 50]);
    const opts = { temperature_equator: 30 };
    coreApi.generateClimate(mesh, heightmap, opts);
    expect(fake.lastMessage).toMatchObject({
      kind: "generate_climate",
      mesh,
      heightmap,
      opts,
    });
    expect(typeof fake.lastMessage!.reqId).toBe("number");
    expect(fake.lastMessage!.reqId).toBeGreaterThan(0);
  });

  it("defaults opts to {} when omitted", () => {
    const mesh = {} as never;
    const heightmap = new Uint8Array([0, 20]);
    coreApi.generateClimate(mesh, heightmap);
    expect((fake.lastMessage as AnyReq).opts).toEqual({});
  });

  it("resolves with { temp, prec } from the worker", async () => {
    const mesh = {} as never;
    const heightmap = new Uint8Array([10, 20]);
    const p = coreApi.generateClimate(mesh, heightmap);
    const result = {
      temp: new Int8Array([5, 10]),
      prec: new Uint8Array([40, 60]),
    };
    fake.reply(result);
    const res = await p;
    expect(res.temp).toBeInstanceOf(Int8Array);
    expect(res.prec).toBeInstanceOf(Uint8Array);
    expect(Array.from(res.temp)).toEqual([5, 10]);
    expect(Array.from(res.prec)).toEqual([40, 60]);
  });

  it("rejects with an Error when the worker reports failure", async () => {
    const mesh = {} as never;
    const p = coreApi.generateClimate(mesh, new Uint8Array([0]));
    fake.replyError("wasm panic: climate failed");
    await expect(p).rejects.toThrow(/climate failed/);
  });
});

describe("coreApi.generateClimateForGrid", () => {
  it("emits the 'generate_climate_for_grid' wire message with grid + opts", () => {
    const grid = makeFakeGrid(100, 42);
    const opts = { height_exponent: 2.0 };
    coreApi.generateClimateForGrid(grid, opts);
    expect(fake.lastMessage).toMatchObject({
      kind: "generate_climate_for_grid",
      grid,
      opts,
    });
  });

  it("defaults opts to {} when omitted", () => {
    const grid = makeFakeGrid(100, 1);
    coreApi.generateClimateForGrid(grid);
    expect((fake.lastMessage as AnyReq).opts).toEqual({});
  });

  it("resolves with the updated Grid from the worker", async () => {
    const grid = makeFakeGrid(100, 42);
    const p = coreApi.generateClimateForGrid(grid);
    const expected = makeFakeGrid(100, 42);
    expected.cells.temp[0] = 11; // pretend climate wrote temp
    fake.reply(expected);
    expect(await p).toBe(expected);
  });

  it("rejects with an Error when the worker reports failure", async () => {
    const grid = makeFakeGrid(100, 42);
    const p = coreApi.generateClimateForGrid(grid);
    fake.replyError("wasm panic: grid climate failed");
    await expect(p).rejects.toThrow(/grid climate failed/);
  });
});

describe("coreApi.generateBiomesForGrid", () => {
  it("emits the 'generate_biomes_for_grid' wire message with the grid", () => {
    const grid = makeFakeGrid(100, 42);
    coreApi.generateBiomesForGrid(grid);
    expect(fake.lastMessage).toMatchObject({
      kind: "generate_biomes_for_grid",
      grid,
    });
    // No opts in this signature — must NOT carry an `opts` key that the
    // worker doesn't expect (would deserialize as `undefined` and break).
    expect((fake.lastMessage as AnyReq).opts).toBeUndefined();
  });

  it("resolves with the updated Grid from the worker", async () => {
    const grid = makeFakeGrid(100, 42);
    const p = coreApi.generateBiomesForGrid(grid);
    const expected = makeFakeGrid(100, 42);
    expected.cells.biome[0] = 7; // pretend biome wrote
    fake.reply(expected);
    expect(await p).toBe(expected);
  });

  it("rejects with an Error when the worker reports failure", async () => {
    const grid = makeFakeGrid(100, 42);
    const p = coreApi.generateBiomesForGrid(grid);
    fake.replyError("wasm panic: grid biome failed");
    await expect(p).rejects.toThrow(/grid biome failed/);
  });
});

// ---- spliceDependentResult (Step 2.5.5 helper) ----------------------------

describe("spliceDependentResult", () => {
  it("splices all 11 numeric fields from a DependentResult into a new Grid", () => {
    const n = 10;
    const grid = makeFakeGrid(n, 1);
    // Give the grid distinctive stale values so we can confirm they are replaced.
    grid.cells.temp = new Array(n).fill(-99);
    grid.cells.prec = new Array(n).fill(99);
    grid.cells.biome = new Array(n).fill(13);
    grid.cells.state = new Array(n).fill(-1);
    grid.cells.province = new Array(n).fill(-1);
    grid.cells.culture = new Array(n).fill(-1);
    grid.cells.religion = new Array(n).fill(-1);
    grid.cells.burg = new Array(n).fill(0);
    grid.cells.fl = new Array(n).fill(0);
    grid.cells.r = new Array(n).fill(0);
    grid.cells.conf = new Array(n).fill(0);

    const dep: DependentResult = {
      temp: new Int8Array(n).fill(5),
      prec: new Uint8Array(n).fill(50),
      biome: new Uint8Array(n).fill(7),
      state: new Int32Array(n).fill(3),
      province: new Int32Array(n).fill(2),
      culture: new Int32Array(n).fill(1),
      religion: new Int32Array(n).fill(4),
      burg: new Int16Array(n).fill(8),
      fl: new Uint16Array(n).fill(100),
      r: new Uint16Array(n).fill(200),
      conf: new Uint16Array(n).fill(300),
      coastline: new Uint8Array(n).fill(1),
      removed_burgs: ["Helms Deep"],
      dissolved_states: new Uint32Array([5]),
      rivers: [],
      lakes: [],
    };

    const result = spliceDependentResult(grid, dep);

    // h is NOT touched by the dependent recompute (it remains the user's
    // edited heightmap).
    expect(result.cells.h).toEqual(grid.cells.h);
    // The 11 spliced fields should reflect dep, not the stale grid values.
    expect(result.cells.temp).toEqual(new Array(n).fill(5));
    expect(result.cells.prec).toEqual(new Array(n).fill(50));
    expect(result.cells.biome).toEqual(new Array(n).fill(7));
    expect(result.cells.state).toEqual(new Array(n).fill(3));
    expect(result.cells.province).toEqual(new Array(n).fill(2));
    expect(result.cells.culture).toEqual(new Array(n).fill(1));
    expect(result.cells.religion).toEqual(new Array(n).fill(4));
    expect(result.cells.burg).toEqual(new Array(n).fill(8));
    expect(result.cells.fl).toEqual(new Array(n).fill(100));
    expect(result.cells.r).toEqual(new Array(n).fill(200));
    expect(result.cells.conf).toEqual(new Array(n).fill(300));
    // TypedArrays are converted to plain number[] (Grid type contract).
    expect(Array.isArray(result.cells.temp)).toBe(true);
    expect(Array.isArray(result.cells.state)).toBe(true);
    // The original grid is NOT mutated (immutability for React subscribers).
    expect(grid.cells.temp).toEqual(new Array(n).fill(-99));
    expect(grid.cells.state).toEqual(new Array(n).fill(-1));
  });

  it("falls back to the grid's existing arrays when a dep field is missing", () => {
    const n = 5;
    const grid = makeFakeGrid(n, 1);
    grid.cells.temp = [10, 20, 30, 40, 50];

    // A dep with only `temp` set; everything else is undefined / missing.
    const dep = {
      temp: new Int8Array(n).fill(-5),
      rivers: [],
      lakes: [],
      removed_burgs: [],
    } as unknown as DependentResult;

    const result = spliceDependentResult(grid, dep);

    expect(result.cells.temp).toEqual([-5, -5, -5, -5, -5]);
    // prec was missing on dep, so the grid's prec is preserved.
    expect(result.cells.prec).toEqual(grid.cells.prec);
    expect(result.cells.biome).toEqual(grid.cells.biome);
  });

  it("returns a new Grid object (reference inequality) for zustand subscribers", () => {
    const grid = makeFakeGrid(3, 1);
    const dep = {
      temp: new Int8Array(3).fill(0),
      prec: new Uint8Array(3).fill(0),
      biome: new Uint8Array(3).fill(0),
      state: new Int32Array(3).fill(-1),
      province: new Int32Array(3).fill(-1),
      culture: new Int32Array(3).fill(-1),
      religion: new Int32Array(3).fill(-1),
      burg: new Int16Array(3).fill(0),
      fl: new Uint16Array(3).fill(0),
      r: new Uint16Array(3).fill(0),
      conf: new Uint16Array(3).fill(0),
      coastline: new Uint8Array(3).fill(0),
      removed_burgs: [],
      dissolved_states: new Uint32Array(0),
      rivers: [],
      lakes: [],
    } as DependentResult;

    const result = spliceDependentResult(grid, dep);
    expect(result).not.toBe(grid);
    expect(result.cells).not.toBe(grid.cells);
    // h should be the SAME array reference (not copied) since it's untouched.
    expect(result.cells.h).toBe(grid.cells.h);
  });
});

// ---- pickCell (Step 2.5.4) ------------------------------------------------

describe("coreApi.pickCell", () => {
  it("emits the 'pick_cell' wire message with x, y and no grid (hot path)", () => {
    coreApi.pickCell(123.4, 567.8);
    expect(fake.lastMessage).toMatchObject({
      kind: "pick_cell",
      x: 123.4,
      y: 567.8,
    });
    // No grid key on the wire — worker uses its held grid handle.
    expect((fake.lastMessage as AnyReq).grid).toBeUndefined();
    expect(typeof fake.lastMessage!.reqId).toBe("number");
    expect(fake.lastMessage!.reqId).toBeGreaterThan(0);
  });

  it("includes the grid on the wire when explicitly passed", () => {
    const grid = makeFakeGrid(100, 42);
    coreApi.pickCell(10, 20, grid);
    expect(fake.lastMessage).toMatchObject({
      kind: "pick_cell",
      grid,
      x: 10,
      y: 20,
    });
  });

  it("resolves with a cell id number from the worker", async () => {
    const p = coreApi.pickCell(50, 60);
    fake.reply(42);
    const result = await p;
    expect(result).toBe(42);
    expect(typeof result).toBe("number");
  });

  it("resolves with -1 when the worker finds no cell", async () => {
    const p = coreApi.pickCell(0, 0);
    fake.reply(-1);
    const result = await p;
    expect(result).toBe(-1);
  });

  it("rejects with an Error when the worker reports failure", async () => {
    const p = coreApi.pickCell(1, 2);
    fake.replyError("wasm panic: pick_cell failed");
    await expect(p).rejects.toThrow(/pick_cell failed/);
  });

  it("routes two concurrent calls to their own reqIds (no cross-talk)", async () => {
    const p1 = coreApi.pickCell(10, 20);
    const firstMsg = fake.lastMessage!;
    const p2 = coreApi.pickCell(30, 40);
    const secondMsg = fake.lastMessage!;
    expect(firstMsg.reqId).not.toBe(secondMsg.reqId);

    // Deliver replies out of order.
    fake.reply(99); // satisfies p2 (most recent reqId)
    fake.lastMessage = firstMsg;
    fake.reply(7); // satisfies p1

    await expect(p2).resolves.toBe(99);
    await expect(p1).resolves.toBe(7);
  });
});

// ---- resetHeightmap (Step 2.5.4) ------------------------------------------

describe("coreApi.resetHeightmap", () => {
  it("emits the 'reset_heightmap' wire message with no grid (hot path)", () => {
    coreApi.resetHeightmap();
    expect(fake.lastMessage).toMatchObject({ kind: "reset_heightmap" });
    expect((fake.lastMessage as AnyReq).grid).toBeUndefined();
    expect(typeof fake.lastMessage!.reqId).toBe("number");
    expect(fake.lastMessage!.reqId).toBeGreaterThan(0);
  });

  it("includes the grid on the wire when explicitly passed", () => {
    const grid = makeFakeGrid(100, 42);
    coreApi.resetHeightmap(grid);
    expect(fake.lastMessage).toMatchObject({
      kind: "reset_heightmap",
      grid,
    });
  });

  it("resolves with a HeightmapPatch (Uint8Array h) when no grid is passed", async () => {
    const p = coreApi.resetHeightmap();
    const patch = { h: new Uint8Array(100).fill(50) };
    fake.reply(patch);
    const result = (await p) as { h: Uint8Array };
    expect(result.h).toBeInstanceOf(Uint8Array);
    expect(result.h.length).toBe(100);
    expect(result.h[0]).toBe(50);
  });

  it("resolves with a full Grid when a grid is explicitly passed", async () => {
    const grid = makeFakeGrid(100, 42);
    const p = coreApi.resetHeightmap(grid);
    const expected = makeFakeGrid(100, 42);
    expected.cells.h = new Array(100).fill(25);
    fake.reply(expected);
    const result = (await p) as Grid;
    expect(result).toBe(expected);
    expect(result.cells.h[0]).toBe(25);
  });

  it("rejects with an Error when the worker reports failure", async () => {
    const p = coreApi.resetHeightmap();
    fake.replyError("wasm panic: reset failed");
    await expect(p).rejects.toThrow(/reset failed/);
  });
});

// ---- storeGrid (Step 2.5.4) ------------------------------------------------

describe("coreApi.storeGrid", () => {
  it("emits the 'store_grid' wire message with the grid", () => {
    const grid = makeFakeGrid(1000, 42);
    coreApi.storeGrid(grid);
    expect(fake.lastMessage).toMatchObject({
      kind: "store_grid",
      grid,
    });
    expect(typeof fake.lastMessage!.reqId).toBe("number");
    expect(fake.lastMessage!.reqId).toBeGreaterThan(0);
  });

  it("always includes the grid (storeGrid has no grid-optional path)", () => {
    const grid = makeFakeGrid(50, 7);
    coreApi.storeGrid(grid);
    expect((fake.lastMessage as AnyReq).grid).toBeDefined();
    expect((fake.lastMessage as AnyReq).grid).toBe(grid);
  });

  it("resolves with null after the worker stores the grid", async () => {
    const grid = makeFakeGrid(100, 1);
    const p = coreApi.storeGrid(grid);
    fake.reply(null);
    const result = await p;
    expect(result).toBeNull();
  });

  it("rejects with an Error when the worker reports failure", async () => {
    const grid = makeFakeGrid(100, 1);
    const p = coreApi.storeGrid(grid);
    fake.replyError("wasm panic: store_grid failed");
    await expect(p).rejects.toThrow(/store_grid failed/);
  });

  it("routes two concurrent calls to their own reqIds (no cross-talk)", async () => {
    const g1 = makeFakeGrid(100, 1);
    const g2 = makeFakeGrid(100, 2);
    const p1 = coreApi.storeGrid(g1);
    const firstMsg = fake.lastMessage!;
    const p2 = coreApi.storeGrid(g2);
    const secondMsg = fake.lastMessage!;
    expect(firstMsg.reqId).not.toBe(secondMsg.reqId);

    // Deliver replies out of order.
    fake.reply(null); // satisfies p2 (most recent)
    fake.lastMessage = firstMsg;
    fake.reply(null); // satisfies p1

    await expect(p2).resolves.toBeNull();
    await expect(p1).resolves.toBeNull();
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
      state: new Array(n).fill(0),
      province: new Array(n).fill(0),
      culture: new Array(n).fill(0),
      religion: new Array(n).fill(0),
      burg: new Array(n).fill(0),
      fl: new Array(n).fill(0),
      r: new Array(n).fill(0),
      conf: new Array(n).fill(0),
    },
  };
}

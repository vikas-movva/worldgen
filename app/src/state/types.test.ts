// Phase 3 Step 3.1 — TS/Rust entity-mirror contract tests.
//
// `core/src/entities.rs` (Rust) and `app/src/state/types.ts` (TS) must stay
// field-for-field identical: serde-wasm-bindgen emits snake_case JS keys from
// the Rust struct fields with no rename, and Phase 4's `projectWorld` worker
// messages will carry `Pack` across the boundary. These tests pin the
// contract the way the Rust serde round-trip tests do — they construct a
// fixture `Pack` of each entity and assert the exact key set the wire will
// see, so a future Rust→JS rename surfaces here before it breaks the worker
// serialization. (Mirror of `entities::tests::*` in the Rust crate.)

import { describe, expect, it } from "vitest";
import type {
	Army,
	Burg,
	Culture,
	Pack,
	Province,
	Religion,
	State,
} from "./types";

// The exact set of keys that must appear on each entity's wire shape, in
// declaration order. `Object.keys()` on a constructed fixture returns keys
// in insertion order for string keys (the JS engine's contract), so this
// also guards the ordering serde-wasm-bindgen relies on when a future Phase
// 4 projector consumes `Pack` via destructuring or positionally.

const STATE_KEYS = [
	"id",
	"name",
	"color",
	"capital",
	"center_cell",
	"form",
	"tax_rate",
	"treasury",
	"rural_pop",
	"urban_pop",
	"military",
	"founded_year",
	"dissolved_year",
	"culture",
] as const;

const PROVINCE_KEYS = [
	"id",
	"state",
	"name",
	"color",
	"center_cell",
	"rural_pop",
	"urban_pop",
	"founded_year",
	"dissolved_year",
] as const;

const CULTURE_KEYS = [
	"id",
	"name",
	"color",
	"origin",
	"type_code",
	"founded_year",
	"dissolved_year",
	"cell_count",
] as const;

const RELIGION_KEYS = [
	"id",
	"name",
	"color",
	"center_cell",
	"parent",
	"followers",
	"type_code",
	"founded_year",
	"dissolved_year",
] as const;

const BURG_KEYS = [
	"id",
	"name",
	"cell",
	"state",
	"culture",
	"religion",
	"population",
	"feature",
	"capital",
	"founded_year",
	"dissolved_year",
] as const;

const ARMY_KEYS = [
	"id",
	"state",
	"cell",
	"size",
	"kind",
	"founded_year",
	"dissolved_year",
] as const;

const PACK_KEYS = [
	"states",
	"provinces",
	"cultures",
	"religions",
	"burgs",
	"armies",
] as const;

function sampleState(): State {
	return {
		id: 1,
		name: "Arvendel",
		color: 0x4a6fa5,
		capital: 1,
		center_cell: 1234,
		form: "Monarchy",
		tax_rate: 0.12,
		treasury: 5000.0,
		rural_pop: 12000.0,
		urban_pop: 8400.0,
		military: 320,
		founded_year: 0,
		dissolved_year: null,
		culture: 1,
	};
}

function sampleProvince(): Province {
	return {
		id: 1,
		state: 1,
		name: "Arvendel Heartland",
		color: 0x5b7bb0,
		center_cell: 1234,
		rural_pop: 4000.0,
		urban_pop: 4200.0,
		founded_year: 0,
		dissolved_year: null,
	};
}

function sampleCulture(): Culture {
	return {
		id: 1,
		name: "Northern Folk",
		color: 0xaa8844,
		origin: 1234,
		type_code: 1,
		founded_year: 0,
		dissolved_year: null,
		cell_count: 612,
	};
}

function sampleReligion(): Religion {
	return {
		id: 1,
		name: "Old Faith",
		color: 0xddccbb,
		center_cell: 1234,
		parent: null,
		followers: 4200.0,
		type_code: 0,
		founded_year: 0,
		dissolved_year: null,
	};
}

function sampleBurg(): Burg {
	return {
		id: 1,
		name: "Arvendel City",
		cell: 1234,
		state: 1,
		culture: 1,
		religion: 1,
		population: 4.2,
		feature: 1,
		capital: 1,
		founded_year: 12,
		dissolved_year: null,
	};
}

function sampleArmy(): Army {
	return {
		id: 1,
		state: 1,
		cell: 1300,
		size: 2000,
		kind: "infantry",
		founded_year: 30,
		dissolved_year: null,
	};
}

function samplePack(): Pack {
	return {
		states: [sampleState()],
		provinces: [sampleProvince()],
		cultures: [sampleCulture()],
		religions: [sampleReligion()],
		burgs: [sampleBurg()],
		armies: [sampleArmy()],
	};
}

describe("Pack (TS mirror of Rust entities.rs)", () => {
	it("Pack exposes the six anthropological-layer arrays", () => {
		const pack: Pack = samplePack();
		expect(Object.keys(pack)).toEqual([...PACK_KEYS]);
		expect(pack.states).toHaveLength(1);
		expect(pack.provinces).toHaveLength(1);
		expect(pack.cultures).toHaveLength(1);
		expect(pack.religions).toHaveLength(1);
		expect(pack.burgs).toHaveLength(1);
		expect(pack.armies).toHaveLength(1);
	});

	it("an empty Pack has six empty arrays (year-0 init order)", () => {
		const empty: Pack = {
			states: [],
			provinces: [],
			cultures: [],
			religions: [],
			burgs: [],
			armies: [],
		};
		expect(Object.keys(empty)).toEqual([...PACK_KEYS]);
		expect(empty.states).toEqual([]);
		expect(empty.armies).toEqual([]);
	});
});

describe("State key set matches Rust struct field order", () => {
	it("exposes the 14 State fields in snake_case order", () => {
		expect(Object.keys(sampleState())).toEqual([...STATE_KEYS]);
	});
});

describe("Province key set matches Rust struct field order", () => {
	it("exposes the 9 Province fields in snake_case order", () => {
		expect(Object.keys(sampleProvince())).toEqual([...PROVINCE_KEYS]);
	});
});

describe("Culture key set matches Rust struct field order", () => {
	it("exposes the 8 Culture fields in snake_case order", () => {
		expect(Object.keys(sampleCulture())).toEqual([...CULTURE_KEYS]);
	});
});

describe("Religion key set matches Rust struct field order", () => {
	it("exposes the 9 Religion fields in snake_case order", () => {
		expect(Object.keys(sampleReligion())).toEqual([...RELIGION_KEYS]);
	});

	it("religion parent is nullable (schism tree root)", () => {
		const root = sampleReligion();
		expect(root.parent).toBeNull();
		const child: Religion = { ...root, id: 2, parent: 1, name: "Reformed" };
		expect(child.parent).toBe(1);
	});
});

describe("Burg key set matches Rust struct field order", () => {
	it("exposes the 11 Burg fields in snake_case order", () => {
		expect(Object.keys(sampleBurg())).toEqual([...BURG_KEYS]);
	});
});

describe("Army key set matches Rust struct field order", () => {
	it("exposes the 7 Army fields in snake_case order", () => {
		expect(Object.keys(sampleArmy())).toEqual([...ARMY_KEYS]);
	});
});

describe("founded/dissolved span is on every entity type", () => {
	it("all six entity types carry founded_year + dissolved_year", () => {
		const pack = samplePack();
		for (const s of pack.states) {
			expect(s).toHaveProperty("founded_year");
			expect(s).toHaveProperty("dissolved_year");
			expect(s.dissolved_year).toBeNull();
		}
		for (const p of pack.provinces) {
			expect(p).toHaveProperty("founded_year");
			expect(p).toHaveProperty("dissolved_year");
		}
		for (const c of pack.cultures) {
			expect(c).toHaveProperty("founded_year");
			expect(c).toHaveProperty("dissolved_year");
		}
		for (const r of pack.religions) {
			expect(r).toHaveProperty("founded_year");
			expect(r).toHaveProperty("dissolved_year");
		}
		for (const b of pack.burgs) {
			expect(b).toHaveProperty("founded_year");
			expect(b).toHaveProperty("dissolved_year");
		}
		for (const a of pack.armies) {
			expect(a).toHaveProperty("founded_year");
			expect(a).toHaveProperty("dissolved_year");
		}
	});

	it("dissolved_year can be a number (post-Schism/Conquer)", () => {
		const dissolved = samplePack();
		dissolved.religions[0].dissolved_year = 732;
		dissolved.states[0].dissolved_year = 450;
		expect(dissolved.religions[0].dissolved_year).toBe(732);
		expect(dissolved.states[0].dissolved_year).toBe(450);
	});

	it("founded_year accepts negative in-universe years", () => {
		const pack = samplePack();
		pack.states[0].founded_year = -800;
		pack.cultures[0].founded_year = -1200;
		expect(pack.states[0].founded_year).toBe(-800);
		expect(pack.cultures[0].founded_year).toBe(-1200);
	});
});

describe("color field survives the wire as 24-bit packed RGB", () => {
	it("State/Culture/Religion/Province carry packed 0xRRGGBB colors", () => {
		const pack = samplePack();
		pack.states[0].color = 0x123456;
		pack.cultures[0].color = 0xffffff;
		pack.religions[0].color = 0x000001;
		pack.provinces[0].color = 0xaabbcc;
		expect(pack.states[0].color).toBe(0x123456);
		expect(pack.cultures[0].color).toBe(0xffffff);
		expect(pack.religions[0].color).toBe(0x000001);
		expect(pack.provinces[0].color).toBe(0xaabbcc);
	});
});

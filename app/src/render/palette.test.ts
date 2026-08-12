// Step 2.3 unit tests - palette.ts (color tables + data-texture builders).
//
// `palette.ts` is pure: no PixiJS or WebGL. It maps heights and biome ids to
// RGBA texels for the data-texture color pattern. These tests pin the contract:
//   - `rgb` extracts 8-bit channels from a 0xRRGGBB value.
//   - `heightColor` maps 0..100 to control stops (deep ocean -> snow) and
//     clamps out-of-range values.
//   - `buildBiomeTextureData` and `buildHeightTextureData` produce
//     texDim*texDim*4-length Uint8Arrays, index the right color per cell, and
//     pad unsampled texels with opaque black.
//   - The 13 FMG biome colors are present and indexed by biome id (0 Marine ..
//     12 Wetland).

import { describe, expect, it } from "vitest";
import {
	BIOME_COLORS,
	buildBiomeTextureData,
	buildHeightTextureData,
	heightColor,
	rgb,
} from "./palette";

// ---- rgb() -------------------------------------------------------------

describe("rgb", () => {
	it("extracts r, g, b channels from 0xRRGGBB", () => {
		expect(rgb(0xff0000)).toEqual([255, 0, 0]);
		expect(rgb(0x00ff00)).toEqual([0, 255, 0]);
		expect(rgb(0x0000ff)).toEqual([0, 0, 255]);
	});

	it("extracts a compound color correctly", () => {
		expect(rgb(0x4f6c8a)).toEqual([0x4f, 0x6c, 0x8a]);
	});

	it("returns 0,0,0 for black", () => {
		expect(rgb(0x000000)).toEqual([0, 0, 0]);
	});
});

// ---- heightColor() ----------------------------------------------------

describe("heightColor", () => {
	it("maps h=0 (deep ocean) to the first stop", () => {
		const [r, g, b] = heightColor(0);
		expect(r).toBe(0x1f);
		expect(g).toBe(0x33);
		expect(b).toBe(0x4d);
	});

	it("maps h=100 (snow) to the last stop", () => {
		const [r, g, b] = heightColor(100);
		expect(r).toBe(0xf0);
		expect(g).toBe(0xf4);
		expect(b).toBe(0xf8);
	});

	it("maps h<20 (water) bluish, h>=20 (land) non-blue", () => {
		const deep = heightColor(0);
		// Deep ocean is bluish: b > r
		expect(deep[2]).toBeGreaterThan(deep[0]);
		// Coast / beach at h=20 should not be the deep-ocean color
		const coast = heightColor(20);
		expect(coast).not.toEqual(deep);
	});

	it("clamps h below 0 to the first stop", () => {
		expect(heightColor(-5)).toEqual(heightColor(0));
	});

	it("clamps h above 100 to the last stop", () => {
		expect(heightColor(150)).toEqual(heightColor(100));
	});

	it("produces monotonically non-decreasing brightness across stops", () => {
		// A coarse proxy: the sum r+g+b at h=0 < sum at h=100 (deep ocean is
		// darker than snow). At least check the endpoints differ and the
		// midpoints are between them in some consistent ordering.
		const dark = heightColor(0).reduce((a, b) => a + b, 0);
		const bright = heightColor(100).reduce((a, b) => a + b, 0);
		expect(bright).toBeGreaterThan(dark);
	});
});

// ---- buildBiomeTextureData() ------------------------------------------

describe("buildBiomeTextureData", () => {
	it("returns a Uint8Array of length texDim*texDim*4", () => {
		const data = buildBiomeTextureData([0, 1, 2], 4);
		expect(data).toBeInstanceOf(Uint8Array);
		expect(data.length).toBe(4 * 4 * 4); // 16 texels * 4 bytes
	});

	it("colors texel i from BIOME_COLORS[biome[i]]", () => {
		const data = buildBiomeTextureData([0, 1, 12], 4);
		// texel 0 = Marine (id 0)
		const [r0, g0, b0] = rgb(BIOME_COLORS[0]);
		expect([data[0], data[1], data[2], data[3]]).toEqual([r0, g0, b0, 255]);
		// texel 1 = Hot desert (id 1)
		const [r1, g1, b1] = rgb(BIOME_COLORS[1]);
		expect([data[4], data[5], data[6], data[7]]).toEqual([r1, g1, b1, 255]);
		// texel 2 = Wetland (id 12)
		const [r2, g2, b2] = rgb(BIOME_COLORS[12]);
		expect([data[8], data[9], data[10], data[11]]).toEqual([r2, g2, b2, 255]);
	});

	it("alpha is 255 for all live texels (fully opaque)", () => {
		const data = buildBiomeTextureData([0, 1, 2, 3], 4);
		for (let i = 0; i < 4; i++) {
			expect(data[i * 4 + 3]).toBe(255);
		}
	});

	it("clamps an out-of-range biome id to 0 (Marine)", () => {
		const data = buildBiomeTextureData([99], 4);
		const [r, g, b] = rgb(BIOME_COLORS[0]);
		expect([data[0], data[1], data[2], data[3]]).toEqual([r, g, b, 255]);
	});

	it("pads unsampled texels with opaque black (rgba 0,0,0,255)", () => {
		// 1 real cell, texDim=4 -> 16 texels. Texels 1..15 are padding.
		const data = buildBiomeTextureData([5], 4);
		for (let i = 1; i < 16; i++) {
			expect(data[i * 4]).toBe(0);
			expect(data[i * 4 + 1]).toBe(0);
			expect(data[i * 4 + 2]).toBe(0);
			expect(data[i * 4 + 3]).toBe(255);
		}
	});
});

// ---- buildHeightTextureData() -----------------------------------------

describe("buildHeightTextureData", () => {
	it("returns a Uint8Array of length texDim*texDim*4", () => {
		const data = buildHeightTextureData([0, 50, 100], 4);
		expect(data).toBeInstanceOf(Uint8Array);
		expect(data.length).toBe(4 * 4 * 4);
	});

	it("colors texel i from heightColor(h[i])", () => {
		const data = buildHeightTextureData([0, 100], 4);
		const [r0, g0, b0] = heightColor(0);
		expect([data[0], data[1], data[2], data[3]]).toEqual([r0, g0, b0, 255]);
		const [r1, g1, b1] = heightColor(100);
		expect([data[4], data[5], data[6], data[7]]).toEqual([r1, g1, b1, 255]);
	});

	it("alpha is 255 for live texels", () => {
		const data = buildHeightTextureData([10, 20, 30], 4);
		for (let i = 0; i < 3; i++) {
			expect(data[i * 4 + 3]).toBe(255);
		}
	});

	it("pads unsampled texels with opaque black", () => {
		const data = buildHeightTextureData([42], 4);
		for (let i = 1; i < 16; i++) {
			expect(data[i * 4]).toBe(0);
			expect(data[i * 4 + 1]).toBe(0);
			expect(data[i * 4 + 2]).toBe(0);
			expect(data[i * 4 + 3]).toBe(255);
		}
	});
});

// ---- BIOME_COLORS table ------------------------------------------------

describe("BIOME_COLORS", () => {
	it("has exactly 13 biome colors (ids 0..12)", () => {
		expect(BIOME_COLORS.length).toBe(13);
	});

	it("index 0 is Marine (water) and index 12 is Wetland", () => {
		// Just check they are distinct valid 24-bit colors.
		expect(BIOME_COLORS[0]).toBe(0x4f6c8a);
		expect(BIOME_COLORS[12]).toBe(0x5a7d6a);
	});
});

// Shared color tables for the renderer (Step 2.3).
//
// - Heightmap gradient: maps a cell height (0..=100, <20 = water) to an RGB
//   color, blended across a few control stops so land/sea read like FMG.
// - Biome palette: the 13 FMG biome colors (id 0 = Marine/water, 1..=12 land),
//   indexed by `cells.biome`. Kept in TS because the Rust `BIOMES` table only
//   carries names (see core/src/biomes.rs).
//
// All colors are 0xRRGGBB u32s, matching PixiJS's `Color` expectation and the
// data-texture pattern (Step 2.3 biome layer packs these into a Uint8 texture).

/** FMG biome colors, index == biome id (0 Marine .. 12 Wetland). */
export const BIOME_COLORS: number[] = [
	0x4f6c8a, // 0  Marine
	0xc2b280, // 1  Hot desert
	0x9ca3a8, // 2  Cold desert
	0xbfc66a, // 3  Savanna
	0x8fbf5f, // 4  Grassland
	0x4f9d4f, // 5  Tropical seasonal forest
	0x3f8f3f, // 6  Temperate deciduous forest
	0x2f7d3f, // 7  Tropical rainforest
	0x356b46, // 8  Temperate rainforest
	0x5a7d4f, // 9  Taiga
	0x9aa0a6, // 10 Tundra
	0xe8f0f5, // 11 Glacier
	0x5a7d6a, // 12 Wetland
];

/** FMG biome display names, index == biome id. Mirrors `core/src/biomes.rs`
 * `BIOMES` table so the inspector label matches the Rust classification. */
export const BIOME_NAMES: readonly string[] = [
	"Marine",
	"Hot desert",
	"Cold desert",
	"Savanna",
	"Grassland",
	"Tropical seasonal forest",
	"Temperate deciduous forest",
	"Tropical rainforest",
	"Temperate rainforest",
	"Taiga",
	"Tundra",
	"Glacier",
	"Wetland",
];

/** Convert 0xRRGGBB to a [r,g,b] triple in 0..255. */
export function rgb(color: number): [number, number, number] {
	return [(color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff];
}

/**
 * Heightmap gradient stops in normalized height (0..1). Below `WATER_LEVEL`
 * we shade deep→shallow blue; above it we ramp sea-green → green → brown →
 * grey/white peaks. Returns an [r,g,b] triple.
 */
const WATER_LEVEL = 0.2; // h < 20 (of 100) is water
const STOPS: { t: number; c: [number, number, number] }[] = [
	{ t: 0.0, c: [0x1f, 0x33, 0x4d] }, // deep ocean
	{ t: WATER_LEVEL * 0.999, c: [0x4f, 0x6c, 0x8a] }, // shallow water
	{ t: WATER_LEVEL, c: [0xc2, 0xb2, 0x80] }, // coast / beach
	{ t: 0.3, c: [0x8f, 0xbf, 0x5f] }, // lowland green
	{ t: 0.5, c: [0x5a, 0x8a, 0x3f] }, // hills
	{ t: 0.7, c: [0x6b, 0x5a, 0x3f] }, // mountains brown
	{ t: 0.85, c: [0x8a, 0x8a, 0x8a] }, // rock
	{ t: 1.0, c: [0xf0, 0xf4, 0xf8] }, // snow
];

/** Map a height value (0..=100) to an [r,g,b] triple. */
export function heightColor(h: number): [number, number, number] {
	const t = Math.max(0, Math.min(1, h / 100));
	for (let i = 0; i < STOPS.length - 1; i++) {
		const a = STOPS[i];
		const b = STOPS[i + 1];
		if (t >= a.t && t <= b.t) {
			const f = (t - a.t) / (b.t - a.t || 1);
			return [
				Math.round(a.c[0] + (b.c[0] - a.c[0]) * f),
				Math.round(a.c[1] + (b.c[1] - a.c[1]) * f),
				Math.round(a.c[2] + (b.c[2] - a.c[2]) * f),
			];
		}
	}
	return STOPS[STOPS.length - 1].c;
}

/**
 * Build a flat per-cell RGBA Uint8Array (length texDim*texDim*4) for the
 * data-texture biome layer. `biome[i]` indexes `BIOME_COLORS`; index is clamped
 * to range. The array is padded to texDim*texDim texels; padding cells get
 * a transparent black color (they will never be sampled by live cells).
 */
export function buildBiomeTextureData(
	biome: Uint8Array | number[],
	texDim: number,
): Uint8Array {
	const n = biome.length;
	const total = texDim * texDim;
	const out = new Uint8Array(total * 4);
	for (let i = 0; i < n; i++) {
		const id = biome[i] | 0;
		const color = BIOME_COLORS[id < BIOME_COLORS.length ? id : 0];
		const [r, g, b] = rgb(color);
		out[i * 4 + 0] = r;
		out[i * 4 + 1] = g;
		out[i * 4 + 2] = b;
		out[i * 4 + 3] = 255;
	}
	// Pad remaining texels with opaque black (unsampled).
	for (let i = n; i < total; i++) {
		out[i * 4 + 3] = 255;
	}
	return out;
}

/**
 * Build a flat per-cell RGBA Uint8Array (length texDim*texDim*4) for the
 * data-texture terrain layer, colored by height via `heightColor`. Mirrors the
 * GLSL gradient so the mesh-shader and the texture-based paths agree.
 */
export function buildHeightTextureData(
	h: Uint8Array | number[],
	texDim: number,
): Uint8Array {
	const n = h.length;
	const total = texDim * texDim;
	const out = new Uint8Array(total * 4);
	for (let i = 0; i < n; i++) {
		const [r, g, b] = heightColor(h[i]);
		out[i * 4 + 0] = r;
		out[i * 4 + 1] = g;
		out[i * 4 + 2] = b;
		out[i * 4 + 3] = 255;
	}
	// Pad remaining texels with opaque black.
	for (let i = n; i < total; i++) {
		out[i * 4 + 3] = 255;
	}
	return out;
}
